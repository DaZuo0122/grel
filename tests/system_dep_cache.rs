//! Integration tests for the `system_dep_cache` SQLite table introduced by
//! the ELF dependency auto-resolution feature.

use grel_cache::Database;

// ---------------------------------------------------------------------------
// Test helper
// ---------------------------------------------------------------------------

/// Create a fresh, isolated SQLite database in a temporary directory.
/// Returns the `Database` handle and the temp dir path (for cleanup).
async fn make_db(label: &str) -> (Database, std::path::PathBuf) {
    let tmp = std::env::temp_dir().join(format!("grel-sysdep-test-{label}-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    let db = Database::init(&tmp.join("state.sqlite"))
        .await
        .expect("Database::init failed");
    (db, tmp)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// A cache lookup for an entry that was never written must return `None`.
#[tokio::test]
async fn cache_miss_returns_none() {
    let (db, tmp) = make_db("miss").await;
    let result = db
        .get_cached_system_dep("libssl.so.3", "ubuntu")
        .await
        .expect("get_cached_system_dep failed");
    assert!(result.is_none());
    db.close().await;
    let _ = std::fs::remove_dir_all(&tmp);
}

/// A value written with `set_cached_system_dep` must be retrievable with
/// `get_cached_system_dep` using the same library name and distro.
#[tokio::test]
async fn cache_round_trip() {
    let (db, tmp) = make_db("roundtrip").await;
    db.set_cached_system_dep("libssl.so.3", "ubuntu", "libssl3")
        .await
        .expect("set failed");
    let result = db
        .get_cached_system_dep("libssl.so.3", "ubuntu")
        .await
        .expect("get failed");
    assert_eq!(result, Some("libssl3".to_string()));
    db.close().await;
    let _ = std::fs::remove_dir_all(&tmp);
}

/// Writing the same (library, distro) key twice must update the package name
/// rather than returning an error or keeping the stale value.
#[tokio::test]
async fn cache_upsert_updates_value() {
    let (db, tmp) = make_db("upsert").await;
    db.set_cached_system_dep("libssl.so.3", "ubuntu", "libssl3")
        .await
        .expect("first set failed");
    db.set_cached_system_dep("libssl.so.3", "ubuntu", "libssl3t64")
        .await
        .expect("second set failed");
    let result = db
        .get_cached_system_dep("libssl.so.3", "ubuntu")
        .await
        .expect("get failed");
    assert_eq!(result, Some("libssl3t64".to_string()));
    db.close().await;
    let _ = std::fs::remove_dir_all(&tmp);
}

/// The same library name can map to different packages on different distros.
/// Each (library, distro) pair is stored and retrieved independently.
#[tokio::test]
async fn cache_is_distro_scoped() {
    let (db, tmp) = make_db("distroscope").await;

    db.set_cached_system_dep("libssl.so.3", "ubuntu", "libssl3")
        .await
        .expect("ubuntu set failed");
    db.set_cached_system_dep("libssl.so.3", "fedora", "openssl-libs")
        .await
        .expect("fedora set failed");

    let ubuntu = db
        .get_cached_system_dep("libssl.so.3", "ubuntu")
        .await
        .expect("ubuntu get failed");
    let fedora = db
        .get_cached_system_dep("libssl.so.3", "fedora")
        .await
        .expect("fedora get failed");
    let arch = db
        .get_cached_system_dep("libssl.so.3", "arch")
        .await
        .expect("arch get failed");

    assert_eq!(ubuntu, Some("libssl3".to_string()));
    assert_eq!(fedora, Some("openssl-libs".to_string()));
    assert!(arch.is_none(), "arch was never written, must be None");

    db.close().await;
    let _ = std::fs::remove_dir_all(&tmp);
}

/// Multiple distinct libraries for the same distro are stored separately.
#[tokio::test]
async fn cache_multiple_libs_same_distro() {
    let (db, tmp) = make_db("multilibs").await;

    db.set_cached_system_dep("libssl.so.3", "arch", "openssl")
        .await
        .expect("ssl set failed");
    db.set_cached_system_dep("libcurl.so.4", "arch", "curl")
        .await
        .expect("curl set failed");
    db.set_cached_system_dep("libz.so.1", "arch", "zlib")
        .await
        .expect("zlib set failed");

    assert_eq!(
        db.get_cached_system_dep("libssl.so.3", "arch")
            .await
            .unwrap(),
        Some("openssl".to_string())
    );
    assert_eq!(
        db.get_cached_system_dep("libcurl.so.4", "arch")
            .await
            .unwrap(),
        Some("curl".to_string())
    );
    assert_eq!(
        db.get_cached_system_dep("libz.so.1", "arch").await.unwrap(),
        Some("zlib".to_string())
    );

    db.close().await;
    let _ = std::fs::remove_dir_all(&tmp);
}
