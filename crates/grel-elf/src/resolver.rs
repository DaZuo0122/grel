//! Orchestrates ELF dep parsing → availability check → package resolution → install.

use std::collections::HashSet;
use std::path::PathBuf;

use grel_cache::Database;
use grel_config::ElfDepConfig;

use crate::distro::DistroInfo;
use crate::pkgmgr::PkgManager;

pub struct ResolvedPackage {
    pub lib: String,
    pub package: String,
}

#[derive(Default)]
pub struct ResolutionReport {
    /// DT_NEEDED entries that were NOT found on the system.
    pub missing_libs: Vec<String>,
    /// Libs for which a package name was found.
    pub resolved_pkgs: Vec<ResolvedPackage>,
    /// Libs that could not be mapped to a package.
    pub unresolved_libs: Vec<String>,
    /// The full install command (tokens), ready to run.
    pub install_cmd: Option<Vec<String>>,
}

pub struct SystemDepResolver<'a> {
    config: &'a ElfDepConfig,
    db: Option<&'a Database>,
    distro: DistroInfo,
    pkg_mgr: Option<Box<dyn PkgManager>>,
}

impl<'a> SystemDepResolver<'a> {
    pub fn new(
        config: &'a ElfDepConfig,
        db: Option<&'a Database>,
        distro: DistroInfo,
        pkg_mgr: Option<Box<dyn PkgManager>>,
    ) -> Self {
        Self {
            config,
            db,
            distro,
            pkg_mgr,
        }
    }

    pub async fn resolve(&self, bin_paths: &[PathBuf]) -> ResolutionReport {
        // 1. Collect all DT_NEEDED (deduplicated), in a blocking thread.
        let paths = bin_paths.to_vec();
        let all_needed: HashSet<String> = tokio::task::spawn_blocking(move || {
            let mut set = HashSet::new();
            for path in &paths {
                for lib in crate::parser::needed_libs(path) {
                    set.insert(lib);
                }
            }
            set
        })
        .await
        .unwrap_or_default();

        if all_needed.is_empty() {
            return ResolutionReport::default();
        }

        // 2. Filter out libs already available (ldconfig -p + /lib scan).
        let needed_vec: Vec<String> = all_needed.into_iter().collect();
        let check_report = grel_network::system_deps::check_system_deps(&needed_vec);

        let missing_libs = check_report.missing.clone();

        if missing_libs.is_empty() {
            return ResolutionReport {
                missing_libs: vec![],
                ..Default::default()
            };
        }

        // 3. For each missing lib, look up package name.
        let mut resolved_pkgs: Vec<ResolvedPackage> = Vec::new();
        let mut unresolved_libs: Vec<String> = Vec::new();

        for lib in &missing_libs {
            if let Some(pkg) = self.lookup_package(lib).await {
                resolved_pkgs.push(ResolvedPackage {
                    lib: lib.clone(),
                    package: pkg,
                });
            } else {
                unresolved_libs.push(lib.clone());
            }
        }

        // 4. Build install command.
        let pkg_names: Vec<String> = resolved_pkgs.iter().map(|r| r.package.clone()).collect();
        let install_cmd = if pkg_names.is_empty() {
            None
        } else {
            Some(self.build_install_cmd(&pkg_names))
        };

        ResolutionReport {
            missing_libs,
            resolved_pkgs,
            unresolved_libs,
            install_cmd,
        }
    }

    async fn lookup_package(&self, lib: &str) -> Option<String> {
        // Priority 1: per-distro user override
        if let Some(distro_map) = self.config.distro_library_map.get(&self.distro.id) {
            if let Some(pkg) = distro_map.get(lib) {
                return Some(pkg.clone());
            }
        }

        // Priority 2: global user override
        if let Some(pkg) = self.config.library_map.get(lib) {
            return Some(pkg.clone());
        }

        // Priority 3: DB cache
        if let Some(db) = self.db {
            if let Ok(Some(pkg)) = db.get_cached_system_dep(lib, &self.distro.id).await {
                return Some(pkg);
            }
        }

        // Priority 4: package manager query (blocking → spawn_blocking)
        if let Some(mgr) = &self.pkg_mgr {
            let lib_owned = lib.to_string();
            // We can't move mgr into spawn_blocking since it's behind &Option<Box<dyn PkgManager>>
            // and the trait isn't Clone. Call resolve_lib directly — it's a short-lived subprocess.
            let result = mgr.resolve_lib(&lib_owned);
            if let Some(ref pkg) = result {
                // Cache the discovery
                if let Some(db) = self.db {
                    db.set_cached_system_dep(lib, &self.distro.id, pkg)
                        .await
                        .ok();
                }
                return result;
            }
        }

        // Priority 5: heuristic (strip .so.X and common lib prefix)
        Some(heuristic_pkg_name(lib))
            .filter(|s| !s.is_empty())
            .map(|s| {
                tracing::debug!("Using heuristic package name for {lib}: {s}");
                s
            })
    }

    fn build_install_cmd(&self, pkgs: &[String]) -> Vec<String> {
        // User-supplied template overrides everything
        if let Some(ref template) = self.config.install_cmd_template {
            let joined = pkgs.join(" ");
            return template
                .replace("{packages}", &joined)
                .split_whitespace()
                .map(str::to_string)
                .collect();
        }

        let mut cmd = if let Some(mgr) = &self.pkg_mgr {
            mgr.install_cmd(pkgs)
        } else {
            // Fallback: just print the packages
            pkgs.to_vec()
        };

        // Prepend sudo if not running as root
        if !cmd.is_empty() && !is_root() {
            cmd.insert(0, "sudo".to_string());
        }

        cmd
    }
}

/// Strip common library prefixes/suffixes to guess a package name.
/// e.g. "libssl.so.3" → "libssl3", "libcurl.so.4" → "libcurl4"
fn heuristic_pkg_name(lib: &str) -> String {
    // Remove ".so.X" suffix
    let base = if let Some(pos) = lib.find(".so") {
        &lib[..pos]
    } else {
        lib
    };

    // Extract version suffix from the ".so.X" part
    let version_suffix: String = lib
        .find(".so.")
        .map(|pos| {
            lib[pos + 4..]
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .collect()
        })
        .unwrap_or_default();

    format!("{base}{version_suffix}")
}

fn is_root() -> bool {
    std::process::Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim() == "0")
        .unwrap_or(false)
}

#[allow(clippy::unwrap_used)]
#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use grel_config::ElfDepConfig;

    use super::*;
    use crate::distro::{DistroFamily, DistroInfo};
    use crate::pkgmgr::PkgManager;

    // -----------------------------------------------------------------------
    // heuristic_pkg_name edge cases
    // -----------------------------------------------------------------------

    #[test]
    fn heuristic_names() {
        assert_eq!(heuristic_pkg_name("libssl.so.3"), "libssl3");
        assert_eq!(heuristic_pkg_name("libcurl.so.4"), "libcurl4");
        assert_eq!(heuristic_pkg_name("libc.so.6"), "libc6");
        assert_eq!(heuristic_pkg_name("libz.so.1"), "libz1");
    }

    #[test]
    fn heuristic_no_so_suffix() {
        // Plain name without any ".so" — returned as-is
        assert_eq!(heuristic_pkg_name("libfoo"), "libfoo");
    }

    #[test]
    fn heuristic_bare_so() {
        // ".so" without a version number → no version digit appended
        assert_eq!(heuristic_pkg_name("libfoo.so"), "libfoo");
    }

    #[test]
    fn heuristic_multi_digit_version() {
        assert_eq!(heuristic_pkg_name("libfoo.so.10"), "libfoo10");
    }

    // -----------------------------------------------------------------------
    // ResolutionReport default
    // -----------------------------------------------------------------------

    #[test]
    fn resolution_report_default_is_empty() {
        let r = ResolutionReport::default();
        assert!(r.missing_libs.is_empty());
        assert!(r.resolved_pkgs.is_empty());
        assert!(r.unresolved_libs.is_empty());
        assert!(r.install_cmd.is_none());
    }

    // -----------------------------------------------------------------------
    // build_install_cmd
    // -----------------------------------------------------------------------

    /// A minimal mock package manager for deterministic testing.
    struct MockPkgManager {
        resolve_result: Option<String>,
    }

    impl PkgManager for MockPkgManager {
        fn name(&self) -> &str {
            "mock"
        }
        fn detect(&self) -> bool {
            true
        }
        fn resolve_lib(&self, _lib: &str) -> Option<String> {
            self.resolve_result.clone()
        }
        fn install_cmd(&self, pkgs: &[String]) -> Vec<String> {
            let mut cmd = vec!["mock-install".to_string()];
            cmd.extend(pkgs.iter().cloned());
            cmd
        }
    }

    fn unknown_distro() -> DistroInfo {
        DistroInfo {
            id: "unknown".to_string(),
            family: DistroFamily::Unknown,
        }
    }

    #[test]
    fn build_install_cmd_uses_template() {
        let mut config = ElfDepConfig::default();
        config.install_cmd_template = Some("sudo apt install {packages}".to_string());
        let resolver = SystemDepResolver::new(&config, None, unknown_distro(), None);
        let cmd = resolver.build_install_cmd(&["libssl3".to_string(), "libcurl4".to_string()]);
        assert_eq!(cmd, vec!["sudo", "apt", "install", "libssl3", "libcurl4"]);
    }

    #[test]
    fn build_install_cmd_template_single_package() {
        let mut config = ElfDepConfig::default();
        config.install_cmd_template = Some("apk add {packages}".to_string());
        let resolver = SystemDepResolver::new(&config, None, unknown_distro(), None);
        let cmd = resolver.build_install_cmd(&["libssl3".to_string()]);
        assert_eq!(cmd, vec!["apk", "add", "libssl3"]);
    }

    #[test]
    fn build_install_cmd_with_mock_mgr_includes_packages() {
        let config = ElfDepConfig::default();
        let mock: Box<dyn PkgManager> = Box::new(MockPkgManager {
            resolve_result: None,
        });
        let resolver = SystemDepResolver::new(&config, None, unknown_distro(), Some(mock));
        let cmd = resolver.build_install_cmd(&["libssl3".to_string()]);
        // The install command must include the package name regardless of sudo prefix
        assert!(cmd.contains(&"mock-install".to_string()));
        assert!(cmd.contains(&"libssl3".to_string()));
    }

    // -----------------------------------------------------------------------
    // lookup_package priority ordering (no DB, no system calls)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn lookup_package_uses_distro_map() {
        let mut config = ElfDepConfig::default();
        let mut distro_map = HashMap::new();
        distro_map.insert("libssl.so.3".to_string(), "openssl-libs".to_string());
        config
            .distro_library_map
            .insert("fedora".to_string(), distro_map);

        let distro = DistroInfo {
            id: "fedora".to_string(),
            family: DistroFamily::RedHat,
        };
        let resolver = SystemDepResolver::new(&config, None, distro, None);
        let result = resolver.lookup_package("libssl.so.3").await;
        assert_eq!(result, Some("openssl-libs".to_string()));
    }

    #[tokio::test]
    async fn lookup_package_uses_global_map() {
        let mut config = ElfDepConfig::default();
        config
            .library_map
            .insert("libssl.so.3".to_string(), "libssl3".to_string());

        let resolver = SystemDepResolver::new(&config, None, unknown_distro(), None);
        let result = resolver.lookup_package("libssl.so.3").await;
        assert_eq!(result, Some("libssl3".to_string()));
    }

    #[tokio::test]
    async fn lookup_package_distro_map_takes_precedence_over_global() {
        let mut config = ElfDepConfig::default();
        config
            .library_map
            .insert("libssl.so.3".to_string(), "global-pkg".to_string());
        let mut distro_map = HashMap::new();
        distro_map.insert("libssl.so.3".to_string(), "distro-pkg".to_string());
        config
            .distro_library_map
            .insert("fedora".to_string(), distro_map);

        let distro = DistroInfo {
            id: "fedora".to_string(),
            family: DistroFamily::RedHat,
        };
        let resolver = SystemDepResolver::new(&config, None, distro, None);
        let result = resolver.lookup_package("libssl.so.3").await;
        assert_eq!(result, Some("distro-pkg".to_string()));
    }

    #[tokio::test]
    async fn lookup_package_distro_map_mismatched_distro_falls_through_to_global() {
        let mut config = ElfDepConfig::default();
        config
            .library_map
            .insert("libssl.so.3".to_string(), "global-pkg".to_string());
        // This distro_library_map entry is for "ubuntu", but our distro is "arch"
        let mut ubuntu_map = HashMap::new();
        ubuntu_map.insert("libssl.so.3".to_string(), "ubuntu-pkg".to_string());
        config
            .distro_library_map
            .insert("ubuntu".to_string(), ubuntu_map);

        let distro = DistroInfo {
            id: "arch".to_string(),
            family: DistroFamily::Arch,
        };
        let resolver = SystemDepResolver::new(&config, None, distro, None);
        let result = resolver.lookup_package("libssl.so.3").await;
        // No distro match → falls back to global library_map
        assert_eq!(result, Some("global-pkg".to_string()));
    }

    #[tokio::test]
    async fn lookup_package_falls_back_to_heuristic_when_no_overrides_or_mgr() {
        let config = ElfDepConfig::default();
        let resolver = SystemDepResolver::new(&config, None, unknown_distro(), None);
        // No library_map, no DB, no pkg_mgr → heuristic
        let result = resolver.lookup_package("libssl.so.3").await;
        assert_eq!(result, Some("libssl3".to_string()));
    }

    #[tokio::test]
    async fn lookup_package_mock_mgr_result_returned() {
        let config = ElfDepConfig::default();
        let mock: Box<dyn PkgManager> = Box::new(MockPkgManager {
            resolve_result: Some("mock-openssl".to_string()),
        });
        let resolver = SystemDepResolver::new(&config, None, unknown_distro(), Some(mock));
        let result = resolver.lookup_package("libssl.so.3").await;
        assert_eq!(result, Some("mock-openssl".to_string()));
    }
}
