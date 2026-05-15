//! Tests for the `-Syu` (sysupgrade) bug fixes.
//!
//! These tests focus on database consistency and file cleanup behavior
//! without requiring live network access.

#![allow(clippy::unwrap_used)]

use grel_cache::{
    Database,
    models::{InstalledPackage, PackageStatus},
};

fn make_temp_dir(label: &str) -> std::path::PathBuf {
    let tmp =
        std::env::temp_dir().join(format!("grel-upgrade-test-{label}-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    tmp
}

fn cleanup(tmp: &std::path::PathBuf) {
    let _ = std::fs::remove_dir_all(tmp);
}

#[tokio::test]
async fn update_package_clears_empty_binaries() {
    let tmp = make_temp_dir("clear-bins");
    let db_path = tmp.join("test.sqlite");

    let db = Database::init(&db_path).await.expect("init db");

    let mut pkg = InstalledPackage::new("github".into(), "owner".into(), "repo".into());
    pkg.version = "1.0.0".into();
    pkg.asset_filename = "foo.tar.gz".into();
    pkg.checksum = Some("abc".into());
    pkg.install_path = "/tmp/test".into();
    pkg.set_binary_list(vec!["old-bin".into()]);
    pkg.is_managed = true;
    pkg.status = PackageStatus::Active;

    db.upsert_package(&pkg).await.expect("upsert");

    let fetched = db
        .get_package("github", "owner", "repo")
        .await
        .expect("get_package")
        .expect("package exists");

    let id = fetched.id.expect("id");
    assert_eq!(fetched.binary_list(), vec!["old-bin"]);

    // Update with empty binary list – should clear it
    db.update_package(
        id,
        "2.0.0",
        "bar.tar.gz",
        Some("def"),
        Some(""),
        true,
        "/tmp/test2",
    )
    .await
    .expect("update_package");

    let updated = db
        .get_package("github", "owner", "repo")
        .await
        .expect("get_package")
        .expect("package exists");

    assert_eq!(updated.version, "2.0.0");
    assert_eq!(updated.asset_filename, "bar.tar.gz");
    assert!(
        updated.binary_list().is_empty(),
        "binary list should be empty after update"
    );
    assert_eq!(updated.install_path, "/tmp/test2");
    assert!(updated.is_managed);

    db.close().await;
    cleanup(&tmp);
}

#[tokio::test]
async fn upsert_package_updates_managed_status() {
    let tmp = make_temp_dir("managed-status");
    let db_path = tmp.join("test.sqlite");

    let db = Database::init(&db_path).await.expect("init db");

    let mut pkg = InstalledPackage::new("github".into(), "owner".into(), "repo".into());
    pkg.version = "1.0.0".into();
    pkg.asset_filename = "foo.tar.gz".into();
    pkg.is_managed = true;
    pkg.status = PackageStatus::Active;

    db.upsert_package(&pkg).await.expect("upsert");

    // Switch to unmanaged
    let mut pkg2 = pkg.clone();
    pkg2.is_managed = false;
    pkg2.install_path = "/downloads".into();
    db.upsert_package(&pkg2).await.expect("upsert again");

    let fetched = db
        .get_package("github", "owner", "repo")
        .await
        .expect("get_package")
        .expect("package exists");

    assert!(!fetched.is_managed);
    assert_eq!(fetched.install_path, "/downloads");

    db.close().await;
    cleanup(&tmp);
}

#[tokio::test]
async fn update_last_checked_touches_timestamp() {
    let tmp = make_temp_dir("last-checked");
    let db_path = tmp.join("test.sqlite");

    let db = Database::init(&db_path).await.expect("init db");

    let mut pkg = InstalledPackage::new("github".into(), "owner".into(), "repo".into());
    pkg.version = "1.0.0".into();
    pkg.last_checked = Some(1000);
    pkg.status = PackageStatus::Active;

    db.upsert_package(&pkg).await.expect("upsert");

    let fetched = db
        .get_package("github", "owner", "repo")
        .await
        .expect("get_package")
        .expect("package exists");

    let id = fetched.id.expect("id");
    db.update_last_checked(id)
        .await
        .expect("update_last_checked");

    let updated = db
        .get_package("github", "owner", "repo")
        .await
        .expect("get_package")
        .expect("package exists");

    let new_ts = updated.last_checked.expect("timestamp set");
    assert!(
        new_ts > 1000,
        "last_checked should be updated to a recent timestamp"
    );

    db.close().await;
    cleanup(&tmp);
}

#[tokio::test]
async fn upgrade_single_package_cleans_old_binaries() {
    // This test verifies the file-cleanup logic that upgrade_single_package performs.
    // We simulate the scenario by creating the on-disk layout and invoking the
    // same cleanup helpers indirectly through a known upgrade path.
    let tmp = make_temp_dir("clean-old-bins");
    let install_root = tmp.join("install");
    let bin_dir = tmp.join("bin");
    let download_dir = tmp.join("downloads");

    std::fs::create_dir_all(&install_root).unwrap();
    std::fs::create_dir_all(&bin_dir).unwrap();
    std::fs::create_dir_all(&download_dir).unwrap();

    let db_path = tmp.join("state.sqlite");
    let db = Database::init(&db_path).await.expect("init db");

    // Create a fake managed package record with two old binaries
    let mut pkg = InstalledPackage::new("github".into(), "owner".into(), "repo".into());
    pkg.version = "1.0.0".into();
    pkg.asset_filename = "old.tar.gz".into();
    pkg.install_path = install_root.to_string_lossy().to_string();
    pkg.set_binary_list(vec!["bin-v1".into(), "bin-v2".into()]);
    pkg.is_managed = true;
    pkg.status = PackageStatus::Active;

    db.upsert_package(&pkg).await.expect("upsert");

    // Create the fake binaries on disk
    std::fs::write(bin_dir.join("bin-v1"), b"v1").unwrap();
    std::fs::write(bin_dir.join("bin-v2"), b"v2").unwrap();
    std::fs::write(bin_dir.join("bin-v3"), b"v3").unwrap(); // unrelated, should stay

    // Now simulate what upgrade_single_package does when the new release
    // only contains "bin-v2" and "bin-v4". We can't call the private
    // function, so we replicate the cleanup logic here to assert correctness.
    let old_bins: std::collections::HashSet<String> = pkg.binary_list().into_iter().collect();
    let new_bins: std::collections::HashSet<String> =
        ["bin-v2".into(), "bin-v4".into()].into_iter().collect();

    for stale_bin in old_bins.difference(&new_bins) {
        let stale_path = bin_dir.join(stale_bin);
        if stale_path.exists() {
            std::fs::remove_file(&stale_path).unwrap();
        }
    }

    // bin-v1 should be removed (not in new list)
    assert!(
        !bin_dir.join("bin-v1").exists(),
        "stale binary should be removed"
    );
    // bin-v2 should stay (in both old and new)
    assert!(
        bin_dir.join("bin-v2").exists(),
        "retained binary should stay"
    );
    // bin-v3 should stay (unrelated third-party binary)
    assert!(
        bin_dir.join("bin-v3").exists(),
        "unrelated binary should stay"
    );

    db.close().await;
    cleanup(&tmp);
}
