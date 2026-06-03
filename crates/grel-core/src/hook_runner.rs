//! Cross-platform hook execution.
//!
//! Runs manifest hooks (`post_install`, `pre_remove`, etc.) using the
//! appropriate shell for the current platform:
//! - Unix: `sh`
//! - Windows: `PowerShell` → `pwsh` → `cmd`
//!
//! Script file paths are detected by extension (`.ps1`, `.bat`, `.cmd`, `.sh`)
//! and invoked with the matching interpreter.  Unix `.sh` scripts are skipped
//! on Windows unless the user explicitly opts in via configuration.

use std::path::Path;
use std::process::{Command, ExitStatus};

/// Errors that can occur when running a hook.
#[derive(Debug, thiserror::Error)]
pub enum HookError {
    /// The hook was skipped (e.g. `.sh` on Windows without opt-in).
    #[error("{0}")]
    Skipped(String),

    /// I/O error while spawning or waiting for the process.
    #[error("Failed to run hook: {0}")]
    Io(#[from] std::io::Error),

    /// The hook process exited with a non-success status.
    #[error("Hook exited with status {0}")]
    ExitStatus(ExitStatus),
}

/// Run a hook script in `cwd`.
///
/// `label` is used for user-facing output (e.g. `post_install`).
/// `allow_sh_on_windows` controls whether `.sh` files are executed on Windows.
pub fn run_hook(hook: &str, cwd: &Path, label: &str, allow_sh_on_windows: bool) {
    println!("  Running {label} hook...");
    match try_run_hook(hook, cwd, allow_sh_on_windows) {
        Ok(()) => println!("  {label} hook completed"),
        Err(HookError::Skipped(msg)) => {
            eprintln!("  Warning: {label} hook skipped: {msg}");
        }
        Err(HookError::ExitStatus(status)) => {
            eprintln!("  Warning: {label} hook exited with status {status}");
        }
        Err(HookError::Io(err)) => {
            eprintln!("  Warning: Failed to run {label} hook: {err}");
        }
    }
}

fn try_run_hook(hook: &str, cwd: &Path, allow_sh_on_windows: bool) -> Result<(), HookError> {
    let lowered = hook.to_lowercase();

    #[cfg(windows)]
    {
        if std::path::Path::new(&lowered)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("ps1"))
        {
            return run_in_shell("powershell", &["-File", hook], cwd)
                .or_else(|_| run_in_shell("pwsh", &["-File", hook], cwd));
        }
        if std::path::Path::new(&lowered)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("bat"))
            || std::path::Path::new(&lowered)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("cmd"))
        {
            return run_in_shell("cmd", &["/C", hook], cwd);
        }
        if std::path::Path::new(&lowered)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("sh"))
        {
            if allow_sh_on_windows {
                return run_in_shell("sh", &[hook], cwd);
            }
            return Err(HookError::Skipped(
                "Unix shell script skipped on Windows. \
                 Set security.allow_sh_hooks_on_windows = true to enable."
                    .to_string(),
            ));
        }
        // Inline command on Windows
        run_in_shell("powershell", &["-Command", hook], cwd)
            .or_else(|_| run_in_shell("pwsh", &["-Command", hook], cwd))
            .or_else(|_| run_in_shell("cmd", &["/C", hook], cwd))
    }

    #[cfg(unix)]
    {
        if lowered.ends_with(".sh") {
            return run_in_shell("sh", &[hook], cwd);
        }
        run_in_shell("sh", &["-c", hook], cwd)
    }
}

/// Spawn `program` with `args` in `cwd` and map the result.
fn run_in_shell(program: &str, args: &[&str], cwd: &Path) -> Result<(), HookError> {
    let status = Command::new(program)
        .args(args)
        .current_dir(cwd)
        .status()?;

    if status.success() {
        Ok(())
    } else {
        Err(HookError::ExitStatus(status))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn make_temp_dir(label: &str) -> std::path::PathBuf {
        let tmp = std::env::temp_dir().join(format!("grel-hook-test-{label}-{}", std::process::id()));
        fs::create_dir_all(&tmp).expect("failed to create temp dir");
        tmp
    }

    fn cleanup(tmp: &std::path::Path) {
        let _ = fs::remove_dir_all(tmp);
    }

    #[test]
    #[cfg(unix)]
    fn run_hook_executes_inline_command() {
        let tmp = make_temp_dir("run-hook-inline");
        let marker = tmp.join("hook_ran");

        run_hook(
            &format!("touch {}", marker.display()),
            &tmp,
            "post_install",
            false,
        );

        assert!(marker.exists(), "hook should have created marker file");
        cleanup(&tmp);
    }

    #[test]
    #[cfg(unix)]
    fn run_hook_executes_sh_file() {
        let tmp = make_temp_dir("run-hook-sh");
        let marker = tmp.join("hook_ran");
        let script = tmp.join("hook.sh");
        fs::write(&script, format!("#!/bin/sh\ntouch '{}'\n", marker.display())).expect("failed to write sh script");

        run_hook(script.to_str().expect("invalid path"), &tmp, "post_install", false);

        assert!(marker.exists(), ".sh hook should have created marker file");
        cleanup(&tmp);
    }

    #[test]
    #[cfg(unix)]
    fn run_hook_handles_failure_gracefully() {
        let tmp = make_temp_dir("run-hook-fail");

        // This should not panic
        run_hook("exit 1", &tmp, "pre_remove", false);

        cleanup(&tmp);
    }

    #[test]
    #[cfg(windows)]
    fn run_hook_executes_powershell_inline() {
        let tmp = make_temp_dir("run-hook-ps-inline");
        let marker = tmp.join("hook_ran.txt");

        run_hook(
            &format!(
                "Write-Output 'test' | Out-File -FilePath '{}'",
                marker.display()
            ),
            &tmp,
            "post_install",
            false,
        );

        assert!(marker.exists(), "PowerShell hook should have created marker file");
        cleanup(&tmp);
    }

    #[test]
    #[cfg(windows)]
    fn run_hook_executes_ps1_file() {
        let tmp = make_temp_dir("run-hook-ps1");
        let marker = tmp.join("hook_ran.txt");
        let script = tmp.join("hook.ps1");
        fs::write(
            &script,
            format!("Write-Output 'test' | Out-File -FilePath '{}'\n", marker.display()),
        )
        .expect("failed to write ps1 script");

        run_hook(script.to_str().expect("invalid path"), &tmp, "post_install", false);

        assert!(marker.exists(), ".ps1 hook should have created marker file");
        cleanup(&tmp);
    }

    #[test]
    #[cfg(windows)]
    fn run_hook_skips_sh_when_not_allowed() {
        let tmp = make_temp_dir("run-hook-sh-skip");
        let marker = tmp.join("hook_ran");
        let script = tmp.join("hook.sh");
        fs::write(&script, format!("#!/bin/sh\ntouch '{}'\n", marker.display())).expect("failed to write sh script");

        run_hook(script.to_str().expect("invalid path"), &tmp, "post_install", false);

        assert!(!marker.exists(), ".sh hook should be skipped on Windows by default");
        cleanup(&tmp);
    }

    #[test]
    #[cfg(windows)]
    fn run_hook_runs_sh_when_allowed() {
        let tmp = make_temp_dir("run-hook-sh-allowed");
        let marker = tmp.join("hook_ran");
        let script = tmp.join("hook.sh");
        fs::write(&script, format!("#!/bin/sh\ntouch '{}'\n", marker.display())).expect("failed to write sh script");

        run_hook(script.to_str().expect("invalid path"), &tmp, "post_install", true);

        // This only succeeds if `sh` is available (Git Bash / WSL / MSYS2).
        // We assert based on whether sh is present.
        let sh_available = Command::new("sh").arg("-c").arg("exit 0").status().is_ok();
        if sh_available {
            assert!(
                marker.exists(),
                ".sh hook should run on Windows when allowed and sh is available"
            );
        }
        cleanup(&tmp);
    }

    #[test]
    #[cfg(windows)]
    fn run_hook_graceful_when_no_shell_found() {
        let tmp = make_temp_dir("run-hook-win-graceful");

        // A non-existent .bat file will fail to spawn, but should not panic
        run_hook("nonexistent_script_xyz.bat", &tmp, "post_install", false);

        cleanup(&tmp);
    }
}
