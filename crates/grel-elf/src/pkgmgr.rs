//! Package manager backends for system dependency resolution.

use std::process::Command;

use crate::distro::DistroFamily;

pub trait PkgManager: Send + Sync {
    fn name(&self) -> &str;
    /// Returns true if this manager's resolution tool is available on PATH.
    fn detect(&self) -> bool;
    /// Try to map a library basename to a package name. None = unknown.
    fn resolve_lib(&self, lib: &str) -> Option<String>;
    /// Return the install command tokens for a list of packages.
    fn install_cmd(&self, pkgs: &[String]) -> Vec<String>;
}

// ---------------------------------------------------------------------------
// Debian/Ubuntu: apt-file
// ---------------------------------------------------------------------------

pub struct AptPkgManager;

impl PkgManager for AptPkgManager {
    fn name(&self) -> &str {
        "apt"
    }

    fn detect(&self) -> bool {
        which("apt-file")
    }

    fn resolve_lib(&self, lib: &str) -> Option<String> {
        let out = Command::new("apt-file")
            .args(["search", "-l", lib])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        // apt-file returns one package per line; take the first
        String::from_utf8(out.stdout)
            .ok()
            .and_then(|s| s.lines().next().map(str::trim).map(str::to_string))
            .filter(|s| !s.is_empty())
    }

    fn install_cmd(&self, pkgs: &[String]) -> Vec<String> {
        let mut cmd = vec![
            "apt-get".to_string(),
            "install".to_string(),
            "-y".to_string(),
        ];
        cmd.extend(pkgs.iter().cloned());
        cmd
    }
}

// ---------------------------------------------------------------------------
// Fedora/RHEL: dnf
// ---------------------------------------------------------------------------

pub struct DnfPkgManager;

impl PkgManager for DnfPkgManager {
    fn name(&self) -> &str {
        "dnf"
    }

    fn detect(&self) -> bool {
        which("dnf")
    }

    fn resolve_lib(&self, lib: &str) -> Option<String> {
        let pattern = format!("*{lib}*");
        let out = Command::new("dnf")
            .args(["repoquery", "--whatprovides", &pattern, "-q"])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        String::from_utf8(out.stdout)
            .ok()
            .and_then(|s| s.lines().next().map(str::trim).map(str::to_string))
            .filter(|s| !s.is_empty())
            // strip epoch:name-ver.arch format to just name
            .map(|s| strip_nevra(&s))
    }

    fn install_cmd(&self, pkgs: &[String]) -> Vec<String> {
        let mut cmd = vec!["dnf".to_string(), "install".to_string(), "-y".to_string()];
        cmd.extend(pkgs.iter().cloned());
        cmd
    }
}

/// Strip NEVRA suffix from dnf output (e.g. "openssl-libs-1:3.0.7-2.fc38.x86_64" → "openssl-libs").
fn strip_nevra(s: &str) -> String {
    // Format: [epoch:]name-version-release.arch
    // The simplest heuristic: drop everything after the last '-' that looks like a version
    s.rsplitn(3, '-')
        .last()
        .unwrap_or(s)
        .split(':')
        .last()
        .unwrap_or(s)
        .to_string()
}

// ---------------------------------------------------------------------------
// Arch Linux: pkgfile (with fallback to pacman -F)
// ---------------------------------------------------------------------------

pub struct PacmanPkgManager;

impl PkgManager for PacmanPkgManager {
    fn name(&self) -> &str {
        "pacman"
    }

    fn detect(&self) -> bool {
        which("pacman")
    }

    fn resolve_lib(&self, lib: &str) -> Option<String> {
        // Prefer pkgfile (faster, no db refresh needed)
        if which("pkgfile") {
            let out = Command::new("pkgfile").arg("-r").arg(lib).output().ok()?;
            if out.status.success() {
                let result = String::from_utf8(out.stdout)
                    .ok()
                    .and_then(|s| s.lines().next().map(str::trim).map(str::to_string))
                    .filter(|s| !s.is_empty());
                if result.is_some() {
                    return result;
                }
            }
        }

        // Fallback: pacman -F (requires files db to be synced)
        let out = Command::new("pacman")
            .args(["-F", "--noconfirm", lib])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        // Output: "extra/openssl 3.x.x" or similar
        String::from_utf8(out.stdout)
            .ok()
            .and_then(|s| {
                s.lines().next().map(|l| {
                    l.split_whitespace()
                        .next()
                        .unwrap_or("")
                        .split('/')
                        .last()
                        .unwrap_or("")
                        .to_string()
                })
            })
            .filter(|s| !s.is_empty())
    }

    fn install_cmd(&self, pkgs: &[String]) -> Vec<String> {
        let mut cmd = vec![
            "pacman".to_string(),
            "-S".to_string(),
            "--noconfirm".to_string(),
        ];
        cmd.extend(pkgs.iter().cloned());
        cmd
    }
}

// ---------------------------------------------------------------------------
// Alpine: apk
// ---------------------------------------------------------------------------

pub struct ApkPkgManager;

impl PkgManager for ApkPkgManager {
    fn name(&self) -> &str {
        "apk"
    }

    fn detect(&self) -> bool {
        which("apk")
    }

    fn resolve_lib(&self, lib: &str) -> Option<String> {
        // `apk info --who-owns /usr/lib/libssl.so.3` needs the full path.
        // Without the path we try the search approach.
        let out = Command::new("apk")
            .args(["search", "-q", lib])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        String::from_utf8(out.stdout)
            .ok()
            .and_then(|s| s.lines().next().map(str::trim).map(str::to_string))
            .filter(|s| !s.is_empty())
    }

    fn install_cmd(&self, pkgs: &[String]) -> Vec<String> {
        let mut cmd = vec!["apk".to_string(), "add".to_string()];
        cmd.extend(pkgs.iter().cloned());
        cmd
    }
}

// ---------------------------------------------------------------------------
// openSUSE: zypper
// ---------------------------------------------------------------------------

pub struct ZypperPkgManager;

impl PkgManager for ZypperPkgManager {
    fn name(&self) -> &str {
        "zypper"
    }

    fn detect(&self) -> bool {
        which("zypper")
    }

    fn resolve_lib(&self, lib: &str) -> Option<String> {
        let out = Command::new("zypper")
            .args(["--non-interactive", "search", "--provides", "-q", lib])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        // zypper output has a header; skip lines until we find a package line
        String::from_utf8(out.stdout)
            .ok()
            .and_then(|s| {
                s.lines()
                    .find(|l| l.starts_with('i') || l.starts_with(' ') || l.starts_with('v'))
                    .and_then(|l| l.split('|').nth(1))
                    .map(str::trim)
                    .map(str::to_string)
            })
            .filter(|s| !s.is_empty())
    }

    fn install_cmd(&self, pkgs: &[String]) -> Vec<String> {
        let mut cmd = vec![
            "zypper".to_string(),
            "install".to_string(),
            "-y".to_string(),
        ];
        cmd.extend(pkgs.iter().cloned());
        cmd
    }
}

// ---------------------------------------------------------------------------
// Detection helpers
// ---------------------------------------------------------------------------

/// Select the best available package manager for the given distro family.
pub fn for_family(family: &DistroFamily) -> Option<Box<dyn PkgManager>> {
    let candidates: Vec<Box<dyn PkgManager>> = match family {
        DistroFamily::Debian => vec![Box::new(AptPkgManager)],
        DistroFamily::RedHat => vec![Box::new(DnfPkgManager)],
        DistroFamily::Arch => vec![Box::new(PacmanPkgManager)],
        DistroFamily::Alpine => vec![Box::new(ApkPkgManager)],
        DistroFamily::Suse => vec![Box::new(ZypperPkgManager)],
        DistroFamily::Unknown => vec![
            Box::new(AptPkgManager),
            Box::new(DnfPkgManager),
            Box::new(PacmanPkgManager),
            Box::new(ApkPkgManager),
            Box::new(ZypperPkgManager),
        ],
    };

    candidates.into_iter().find(|m| m.detect())
}

fn which(cmd: &str) -> bool {
    Command::new("which")
        .arg(cmd)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[allow(clippy::unwrap_used)]
#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // install_cmd format
    // -----------------------------------------------------------------------

    #[test]
    fn apt_install_cmd_format() {
        let mgr = AptPkgManager;
        let pkgs = vec!["libssl3".to_string(), "libcurl4".to_string()];
        assert_eq!(
            mgr.install_cmd(&pkgs),
            vec!["apt-get", "install", "-y", "libssl3", "libcurl4"]
        );
    }

    #[test]
    fn dnf_install_cmd_format() {
        let mgr = DnfPkgManager;
        let cmd = mgr.install_cmd(&["openssl-libs".to_string()]);
        assert_eq!(cmd, vec!["dnf", "install", "-y", "openssl-libs"]);
    }

    #[test]
    fn pacman_install_cmd_format() {
        let mgr = PacmanPkgManager;
        let cmd = mgr.install_cmd(&["openssl".to_string()]);
        assert_eq!(cmd, vec!["pacman", "-S", "--noconfirm", "openssl"]);
    }

    #[test]
    fn apk_install_cmd_format() {
        let mgr = ApkPkgManager;
        let cmd = mgr.install_cmd(&["libssl3".to_string()]);
        assert_eq!(cmd, vec!["apk", "add", "libssl3"]);
    }

    #[test]
    fn zypper_install_cmd_format() {
        let mgr = ZypperPkgManager;
        let cmd = mgr.install_cmd(&["libopenssl3".to_string()]);
        assert_eq!(cmd, vec!["zypper", "install", "-y", "libopenssl3"]);
    }

    #[test]
    fn install_cmd_multiple_packages() {
        let mgr = DnfPkgManager;
        let cmd = mgr.install_cmd(&[
            "pkg-a".to_string(),
            "pkg-b".to_string(),
            "pkg-c".to_string(),
        ]);
        assert_eq!(cmd, vec!["dnf", "install", "-y", "pkg-a", "pkg-b", "pkg-c"]);
    }

    #[test]
    fn install_cmd_empty_packages() {
        // Each manager should still emit a valid (if vacuous) command prefix
        assert_eq!(
            AptPkgManager.install_cmd(&[]),
            vec!["apt-get", "install", "-y"]
        );
        assert_eq!(DnfPkgManager.install_cmd(&[]), vec!["dnf", "install", "-y"]);
        assert_eq!(
            PacmanPkgManager.install_cmd(&[]),
            vec!["pacman", "-S", "--noconfirm"]
        );
        assert_eq!(ApkPkgManager.install_cmd(&[]), vec!["apk", "add"]);
        assert_eq!(
            ZypperPkgManager.install_cmd(&[]),
            vec!["zypper", "install", "-y"]
        );
    }

    // -----------------------------------------------------------------------
    // name()
    // -----------------------------------------------------------------------

    #[test]
    fn manager_names() {
        assert_eq!(AptPkgManager.name(), "apt");
        assert_eq!(DnfPkgManager.name(), "dnf");
        assert_eq!(PacmanPkgManager.name(), "pacman");
        assert_eq!(ApkPkgManager.name(), "apk");
        assert_eq!(ZypperPkgManager.name(), "zypper");
    }

    // -----------------------------------------------------------------------
    // strip_nevra
    // -----------------------------------------------------------------------

    #[test]
    fn strip_nevra_plain_name() {
        assert_eq!(strip_nevra("openssl"), "openssl");
    }

    #[test]
    fn strip_nevra_name_version_release_arch() {
        // "openssl-libs-3.0.7-2.fc38.x86_64" → "openssl-libs"
        assert_eq!(
            strip_nevra("openssl-libs-3.0.7-2.fc38.x86_64"),
            "openssl-libs"
        );
    }

    #[test]
    fn strip_nevra_with_epoch() {
        // epoch embedded in version: "openssl-libs-1:3.0.7-2.fc38.x86_64" → "openssl-libs"
        assert_eq!(
            strip_nevra("openssl-libs-1:3.0.7-2.fc38.x86_64"),
            "openssl-libs"
        );
    }

    #[test]
    fn strip_nevra_epoch_prefixed() {
        // epoch on whole string: "1:openssl-3.0.7-2.fc38.x86_64" → "openssl"
        assert_eq!(strip_nevra("1:openssl-3.0.7-2.fc38.x86_64"), "openssl");
    }

    #[test]
    fn which_nonexistent_returns_false() {
        assert!(!which("__grel_nonexistent_binary__"));
    }
}
