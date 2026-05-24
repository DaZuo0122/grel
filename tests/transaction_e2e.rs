//! End-to-end tests for install/upgrade atomicity and rollback behavior.
//!
//! These tests exercise the real extraction, filesystem rollback, and database
//! transaction paths using temporary directories and fake archives.

#![allow(clippy::unwrap_used)]

mod common;

use std::path::Path;

use grel_cache::models::PackageFile;
use grel_network::archive::install_asset;
use sqlx::Row;

// ---------------------------------------------------------------------------
// 1. Extraction failure must not leave orphaned files
// ---------------------------------------------------------------------------

#[test]
fn corrupt_archive_extraction_leaves_no_orphans() {
    let tmp = common::temp_dir("extract-fail");
    let install_dir = tmp.join("install");
    let bin_dir = tmp.join("bin");
    let archive = tmp.join("bad.tar.gz");

    std::fs::create_dir_all(&install_dir).unwrap();
    std::fs::create_dir_all(&bin_dir).unwrap();
    common::corrupt_archive(&archive);

    let result = install_asset(&archive, &install_dir, &bin_dir, "bad.tar.gz", false);
    assert!(result.is_err(), "corrupt archive should fail extraction");

    // The install_asset helper creates install_dir before attempting extraction,
    // so the directory itself may exist, but no extracted contents should remain.
    let extracted = install_dir.join("extracted");
    assert!(
        !extracted.exists() || common::count_files(&extracted) == 0,
        "no extracted files should remain after failure"
    );

    // No binaries should have been linked
    assert_eq!(common::count_files(&bin_dir), 0, "bin_dir should be empty");

    common::cleanup(&tmp);
}

#[test]
fn valid_archive_extraction_creates_expected_layout() {
    let tmp = common::temp_dir("extract-ok");
    let install_dir = tmp.join("install");
    let bin_dir = tmp.join("bin");
    let archive = tmp.join("pkg.tar.gz");

    std::fs::create_dir_all(&install_dir).unwrap();
    std::fs::create_dir_all(&bin_dir).unwrap();
    common::fake_archive(&archive, &[("mytool", b"binary data"), ("readme.txt", b"hello")]);

    let result = install_asset(&archive, &install_dir, &bin_dir, "pkg.tar.gz", false);
    assert!(result.is_ok(), "valid archive should extract: {:?}", result.err());

    let res = result.unwrap();
    assert!(res.install_dir.exists());
    assert!(!res.is_plain_binary);

    // Extracted tree should contain the files
    let extracted = install_dir.join("extracted");
    assert!(extracted.exists(), "extracted dir should exist");
    assert!(extracted.join("mytool").exists(), "mytool should be extracted");
    assert!(extracted.join("readme.txt").exists(), "readme.txt should be extracted");

    common::cleanup(&tmp);
}

// ---------------------------------------------------------------------------
// 2. Plain binary install path
// ---------------------------------------------------------------------------

#[test]
fn plain_binary_install_links_correctly() {
    let tmp = common::temp_dir("plain-bin");
    let install_dir = tmp.join("install");
    let bin_dir = tmp.join("bin");
    let archive = tmp.join("mytool.exe");

    std::fs::create_dir_all(&install_dir).unwrap();
    std::fs::create_dir_all(&bin_dir).unwrap();
    std::fs::write(&archive, b"fake exe").unwrap();

    let result = install_asset(&archive, &install_dir, &bin_dir, "mytool.exe", false);
    assert!(result.is_ok(), "plain binary install should succeed: {:?}", result.err());

    let res = result.unwrap();
    assert!(res.is_plain_binary);
    assert_eq!(res.installed_binaries.len(), 1);

    common::cleanup(&tmp);
}

// ---------------------------------------------------------------------------
// 3. Database transaction atomicity (E2E via public API)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn db_transaction_rollback_leaves_no_trace() {
    let guard = common::TempDb::open("tx-rollback").await;

    // Begin a transaction and upsert a package
    let mut tx = guard
        .db()
        .begin_transaction()
        .await
        .expect("begin tx");

    let pkg = common::fake_managed_pkg(
        "github", "owner", "repo", "1.0.0",
        Path::new("/tmp/install"),
        &["bin"],
    );
    tx.upsert_package(&pkg).await.expect("upsert in tx");

    // Verify it's visible inside the transaction
    let inside = tx.get_package("github", "owner", "repo").await.unwrap();
    assert!(inside.is_some(), "package should be visible inside tx");

    // Rollback
    tx.rollback().await.expect("rollback");

    // Verify it's NOT visible outside the transaction
    let outside = guard
        .db()
        .get_package("github", "owner", "repo")
        .await
        .unwrap();
    assert!(outside.is_none(), "package should NOT exist after rollback");

    guard.close().await;
}

#[tokio::test]
async fn db_transaction_commit_is_durable() {
    let guard = common::TempDb::open("tx-commit").await;

    let mut tx = guard
        .db()
        .begin_transaction()
        .await
        .expect("begin tx");

    let pkg = common::fake_managed_pkg(
        "github", "owner", "repo", "1.0.0",
        Path::new("/tmp/install"),
        &["bin"],
    );
    tx.upsert_package(&pkg).await.expect("upsert in tx");
    tx.commit().await.expect("commit");

    let outside = guard
        .db()
        .get_package("github", "owner", "repo")
        .await
        .unwrap();
    assert!(outside.is_some(), "package should exist after commit");
    assert_eq!(outside.unwrap().version, "1.0.0");

    guard.close().await;
}

#[tokio::test]
async fn db_transaction_package_files_rollback_together() {
    let guard = common::TempDb::open("tx-files-rollback").await;

    // First insert a package outside any transaction so we have an id
    let pkg = common::fake_managed_pkg(
        "github", "owner", "repo", "1.0.0",
        Path::new("/tmp/install"),
        &[],
    );
    guard.db().upsert_package(&pkg).await.unwrap();

    let fetched = guard
        .db()
        .get_package("github", "owner", "repo")
        .await
        .unwrap()
        .unwrap();
    let id = fetched.id.unwrap();

    // Set some files in a transaction then rollback
    let mut tx = guard.db().begin_transaction().await.expect("begin tx");
    let files = vec![
        PackageFile::new(id, "/tmp/install/a.txt".into(), "config".into()),
        PackageFile::new(id, "/tmp/install/b.txt".into(), "data".into()),
    ];
    tx.set_package_files(id, &files).await.expect("set files");

    // Verify inside tx using raw sqlx on the transaction
    let inside_count: i64 = sqlx::query(
        "SELECT COUNT(*) FROM package_files WHERE package_id = ?"
    )
    .bind(id)
    .fetch_one(&mut **tx.tx())
    .await
    .map(|r| r.get::<i64, _>(0))
    .unwrap_or(0);
    assert_eq!(inside_count, 2, "2 files should exist inside tx");

    tx.rollback().await.expect("rollback");

    // Verify outside tx
    let outside_count = common::count_rows(guard.db(), "package_files").await;
    assert_eq!(outside_count, 0, "no files should exist after rollback");

    guard.close().await;
}

#[tokio::test]
async fn db_transaction_dependencies_rollback_together() {
    let guard = common::TempDb::open("tx-deps-rollback").await;

    let pkg = common::fake_managed_pkg(
        "github", "owner", "repo", "1.0.0",
        Path::new("/tmp/install"),
        &[],
    );
    guard.db().upsert_package(&pkg).await.unwrap();

    let fetched = guard
        .db()
        .get_package("github", "owner", "repo")
        .await
        .unwrap()
        .unwrap();
    let id = fetched.id.unwrap();

    let mut tx = guard.db().begin_transaction().await.expect("begin tx");
    let deps = vec![
        grel_cache::models::Dependency {
            id: None,
            package_id: id,
            dep_target: "github/other/lib".into(),
            dep_type: grel_cache::models::DependencyType::System,
        },
    ];
    tx.set_dependencies(id, &deps).await.expect("set deps");

    let inside_count: i64 = sqlx::query(
        "SELECT COUNT(*) FROM dependencies WHERE package_id = ?"
    )
    .bind(id)
    .fetch_one(&mut **tx.tx())
    .await
    .map(|r| r.get::<i64, _>(0))
    .unwrap_or(0);
    assert_eq!(inside_count, 1, "1 dep should exist inside tx");

    tx.rollback().await.expect("rollback");

    let outside_count = common::count_rows(guard.db(), "dependencies").await;
    assert_eq!(outside_count, 0, "no deps should exist after rollback");

    guard.close().await;
}

// ---------------------------------------------------------------------------
// 4. Simulated sequential batch partial failure
// ---------------------------------------------------------------------------

#[tokio::test]
async fn sequential_batch_rolls_back_failed_package_only() {
    let guard = common::TempDb::open("batch-partial").await;

    // Simulate installing package A successfully
    let pkg_a = common::fake_managed_pkg(
        "github", "a", "repo", "1.0.0",
        Path::new("/tmp/a"),
        &["bin-a"],
    );
    guard.db().upsert_package(&pkg_a).await.unwrap();

    // Now simulate package B failing inside a transaction
    let mut tx = guard.db().begin_transaction().await.expect("begin tx");
    let pkg_b = common::fake_managed_pkg(
        "github", "b", "repo", "1.0.0",
        Path::new("/tmp/b"),
        &["bin-b"],
    );
    tx.upsert_package(&pkg_b).await.expect("upsert b");

    // Simulate a failure: rollback B
    tx.rollback().await.expect("rollback");

    // A should still exist, B should not
    let a = guard.db().get_package("github", "a", "repo").await.unwrap();
    let b = guard.db().get_package("github", "b", "repo").await.unwrap();
    assert!(a.is_some(), "package A should survive");
    assert!(b.is_none(), "package B should be rolled back");

    guard.close().await;
}

#[tokio::test]
async fn sequential_batch_commits_packages_independently() {
    let guard = common::TempDb::open("batch-independent").await;

    let pkg_a = common::fake_managed_pkg(
        "github", "a", "repo", "1.0.0",
        Path::new("/tmp/a"),
        &[],
    );
    let pkg_b = common::fake_managed_pkg(
        "github", "b", "repo", "2.0.0",
        Path::new("/tmp/b"),
        &[],
    );

    // Commit A
    let mut tx_a = guard.db().begin_transaction().await.unwrap();
    tx_a.upsert_package(&pkg_a).await.unwrap();
    tx_a.commit().await.unwrap();

    // Commit B
    let mut tx_b = guard.db().begin_transaction().await.unwrap();
    tx_b.upsert_package(&pkg_b).await.unwrap();
    tx_b.commit().await.unwrap();

    assert_eq!(common::count_rows(guard.db(), "installed").await, 2);

    let fetched_a = guard.db().get_package("github", "a", "repo").await.unwrap().unwrap();
    let fetched_b = guard.db().get_package("github", "b", "repo").await.unwrap().unwrap();
    assert_eq!(fetched_a.version, "1.0.0");
    assert_eq!(fetched_b.version, "2.0.0");

    guard.close().await;
}
