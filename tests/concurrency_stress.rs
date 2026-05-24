//! Concurrency stress tests for grel.
//!
//! These tests spawn many parallel tasks to exercise races in the database
//! pool, filesystem operations, and archive extraction paths.
//!
//! Run individually with: cargo test --test concurrency_stress -- --nocapture

#![allow(clippy::unwrap_used)]

mod common;

use std::path::Path;
use std::sync::Arc;

use grel_cache::models::InstalledPackage;
use grel_network::archive::install_asset;

// ---------------------------------------------------------------------------
// 1. Parallel database read/write
// ---------------------------------------------------------------------------

#[tokio::test]
async fn parallel_db_upserts_no_lost_packages() {
    let guard = common::TempDb::open("parallel-upserts").await;
    let db = guard.db().clone();

    let mut handles = Vec::new();
    for i in 0..50 {
        let db = db.clone();
        let pkg = common::fake_managed_pkg(
            "github",
            &format!("owner-{i}"),
            &format!("repo-{i}"),
            "1.0.0",
            Path::new("/tmp/install"),
            &[],
        );
        handles.push(tokio::spawn(async move {
            db.upsert_package(&pkg).await.expect("upsert");
        }));
    }

    for h in handles {
        h.await.unwrap();
    }

    let count = common::count_rows(guard.db(), "installed").await;
    assert_eq!(
        count, 50,
        "all 50 parallel upserts must be present, none lost"
    );

    guard.close().await;
}

#[tokio::test]
async fn parallel_db_reads_during_heavy_write_load() {
    let guard = common::TempDb::open("parallel-read-write").await;
    let db = guard.db().clone();

    // Seed one package
    let pkg = common::fake_managed_pkg(
        "github", "seed", "repo", "1.0.0",
        Path::new("/tmp/install"),
        &[],
    );
    db.upsert_package(&pkg).await.unwrap();

    let db_read = db.clone();
    let mut read_handles = Vec::new();
    let mut write_handles = Vec::new();

    // Spawn 20 readers
    for _ in 0..20 {
        let db = db_read.clone();
        read_handles.push(tokio::spawn(async move {
            for _ in 0..20 {
                let _ = db.get_package("github", "seed", "repo").await;
                tokio::task::yield_now().await;
            }
        }));
    }

    // Spawn 20 writers updating the same package
    for i in 0..20 {
        let db = db.clone();
        let ver = format!("1.0.{i}");
        write_handles.push(tokio::spawn(async move {
            let p = common::fake_managed_pkg(
                "github", "seed", "repo", &ver,
                Path::new("/tmp/install"),
                &[],
            );
            db.upsert_package(&p).await.expect("upsert");
        }));
    }

    for h in read_handles {
        h.await.unwrap();
    }
    for h in write_handles {
        h.await.unwrap();
    }

    // The final version should be one of the written versions
    let final_pkg = guard
        .db()
        .get_package("github", "seed", "repo")
        .await
        .unwrap()
        .unwrap();
    assert!(final_pkg.version.starts_with("1.0."));

    guard.close().await;
}

#[tokio::test]
async fn parallel_db_transactions_do_not_deadlock() {
    let guard = common::TempDb::open("parallel-tx").await;
    let db = guard.db().clone();

    let mut handles = Vec::new();
    for i in 0..20 {
        let db = db.clone();
        handles.push(tokio::spawn(async move {
            let mut tx = db.begin_transaction().await.expect("begin tx");
            let pkg = common::fake_managed_pkg(
                "github",
                &format!("owner-{i}"),
                &format!("repo-{i}"),
                "1.0.0",
                Path::new("/tmp/install"),
                &[],
            );
            tx.upsert_package(&pkg).await.expect("upsert");
            tx.commit().await.expect("commit");
        }));
    }

    for h in handles {
        h.await.unwrap();
    }

    let count = common::count_rows(guard.db(), "installed").await;
    assert_eq!(count, 20, "all 20 committed transactions must persist");

    guard.close().await;
}

// ---------------------------------------------------------------------------
// 2. Parallel archive extraction
// ---------------------------------------------------------------------------

#[test]
fn parallel_archive_extractions_to_different_dirs() {
    let tmp = common::temp_dir("parallel-extract");
    let archive = tmp.join("pkg.tar.gz");
    common::fake_archive(
        &archive,
        &[
            ("bin/tool", b"binary"),
            ("share/doc/readme.txt", b"hello"),
        ],
    );

    let mut handles = Vec::new();
    for i in 0..10 {
        let install_dir = tmp.join(format!("install-{i}"));
        let bin_dir = tmp.join(format!("bin-{i}"));
        let archive_path = archive.clone();
        std::fs::create_dir_all(&install_dir).unwrap();
        std::fs::create_dir_all(&bin_dir).unwrap();

        handles.push(std::thread::spawn(move || {
            let result = install_asset(&archive_path, &install_dir, &bin_dir, "pkg.tar.gz", false);
            assert!(result.is_ok(), "thread {i} failed: {:?}", result.err());
            // Verify extraction succeeded
            assert!(
                install_dir.join("extracted/bin/tool").exists(),
                "thread {i}: tool missing"
            );
        }));
    }

    for h in handles {
        h.join().unwrap();
    }

    common::cleanup(&tmp);
}

// ---------------------------------------------------------------------------
// 3. Concurrent filesystem operations
// ---------------------------------------------------------------------------

#[test]
fn concurrent_temp_dir_creation_no_collisions() {
    let mut handles = Vec::new();
    for _ in 0..50 {
        handles.push(std::thread::spawn(move || {
            let tmp = common::temp_dir("collision-check");
            let marker = tmp.join("marker.txt");
            std::fs::write(&marker, b"x").unwrap();
            // Another thread should never see our marker
            let count = common::count_files(&tmp);
            assert_eq!(count, 1, "each temp dir must be isolated");
            common::cleanup(&tmp);
        }));
    }

    for h in handles {
        h.join().unwrap();
    }
}

// ---------------------------------------------------------------------------
// 4. Rapid sequential temp db open/close
// ---------------------------------------------------------------------------

#[tokio::test]
async fn rapid_db_open_close_no_leak() {
    for i in 0..20 {
        let guard = common::TempDb::open(&format!("rapid-{i}")).await;
        let pkg = common::fake_package("github", "owner", "repo");
        guard.db().upsert_package(&pkg).await.unwrap();
        let count = common::count_rows(guard.db(), "installed").await;
        assert_eq!(count, 1);
        guard.close().await;
        // After close, the temp directory should be gone or at least the db closed
    }
}

// ---------------------------------------------------------------------------
// 5. Mixed parallel read + transaction commit
// ---------------------------------------------------------------------------

#[tokio::test]
async fn parallel_reads_during_transaction_commits() {
    let guard = common::TempDb::open("mixed-parallel").await;
    let db = guard.db().clone();

    // Seed 10 packages
    for i in 0..10 {
        let pkg = common::fake_managed_pkg(
            "github",
            &format!("owner-{i}"),
            &format!("repo-{i}"),
            "1.0.0",
            Path::new("/tmp/install"),
            &[],
        );
        db.upsert_package(&pkg).await.unwrap();
    }

    let mut read_handles = Vec::new();
    let mut tx_handles = Vec::new();

    // 10 readers scanning all packages
    for _ in 0..10 {
        let db = db.clone();
        read_handles.push(tokio::spawn(async move {
            for _ in 0..10 {
                for i in 0..10 {
                    let _ = db
                        .get_package("github", &format!("owner-{i}"), &format!("repo-{i}"))
                        .await;
                }
                tokio::task::yield_now().await;
            }
        }));
    }

    // 10 writers committing new packages in transactions
    for i in 10..20 {
        let db = db.clone();
        tx_handles.push(tokio::spawn(async move {
            let mut tx = db.begin_transaction().await.expect("begin tx");
            let pkg = common::fake_managed_pkg(
                "github",
                &format!("owner-{i}"),
                &format!("repo-{i}"),
                "1.0.0",
                Path::new("/tmp/install"),
                &[],
            );
            tx.upsert_package(&pkg).await.expect("upsert");
            tx.commit().await.expect("commit");
        }));
    }

    for h in read_handles {
        h.await.unwrap();
    }
    for h in tx_handles {
        h.await.unwrap();
    }

    let count = common::count_rows(guard.db(), "installed").await;
    assert_eq!(count, 20, "all 20 packages must be present");

    guard.close().await;
}
