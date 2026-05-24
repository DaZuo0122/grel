//! Integration tests for the `system_dep_cache` SQLite table introduced by
//! the ELF dependency auto-resolution feature.

#![allow(clippy::unwrap_used)]

mod common;

/// A cache lookup for an entry that was never written must return `None`.
#[tokio::test]
async fn cache_miss_returns_none() {
    let guard = common::TempDb::open("miss").await;
    let result = guard
        .db()
        .get_cached_system_dep("libssl.so.3", "ubuntu")
        .await
        .expect("get_cached_system_dep failed");
    assert!(result.is_none());
    guard.close().await;
}

/// A value written with `set_cached_system_dep` must be retrievable with
/// `get_cached_system_dep` using the same library name and distro.
#[tokio::test]
async fn cache_round_trip() {
    let guard = common::TempDb::open("roundtrip").await;
    guard
        .db()
        .set_cached_system_dep("libssl.so.3", "ubuntu", "libssl3")
        .await
        .expect("set failed");
    let result = guard
        .db()
        .get_cached_system_dep("libssl.so.3", "ubuntu")
        .await
        .expect("get failed");
    assert_eq!(result, Some("libssl3".to_string()));
    guard.close().await;
}

/// Writing the same (library, distro) key twice must update the package name
/// rather than returning an error or keeping the stale value.
#[tokio::test]
async fn cache_upsert_updates_value() {
    let guard = common::TempDb::open("upsert").await;
    guard
        .db()
        .set_cached_system_dep("libssl.so.3", "ubuntu", "libssl3")
        .await
        .expect("first set failed");
    guard
        .db()
        .set_cached_system_dep("libssl.so.3", "ubuntu", "libssl3t64")
        .await
        .expect("second set failed");
    let result = guard
        .db()
        .get_cached_system_dep("libssl.so.3", "ubuntu")
        .await
        .expect("get failed");
    assert_eq!(result, Some("libssl3t64".to_string()));
    guard.close().await;
}

/// The same library name can map to different packages on different distros.
/// Each (library, distro) pair is stored and retrieved independently.
#[tokio::test]
async fn cache_is_distro_scoped() {
    let guard = common::TempDb::open("distroscope").await;

    guard
        .db()
        .set_cached_system_dep("libssl.so.3", "ubuntu", "libssl3")
        .await
        .expect("ubuntu set failed");
    guard
        .db()
        .set_cached_system_dep("libssl.so.3", "fedora", "openssl-libs")
        .await
        .expect("fedora set failed");

    let ubuntu = guard
        .db()
        .get_cached_system_dep("libssl.so.3", "ubuntu")
        .await
        .expect("ubuntu get failed");
    let fedora = guard
        .db()
        .get_cached_system_dep("libssl.so.3", "fedora")
        .await
        .expect("fedora get failed");
    let arch = guard
        .db()
        .get_cached_system_dep("libssl.so.3", "arch")
        .await
        .expect("arch get failed");

    assert_eq!(ubuntu, Some("libssl3".to_string()));
    assert_eq!(fedora, Some("openssl-libs".to_string()));
    assert!(arch.is_none(), "arch was never written, must be None");

    guard.close().await;
}

/// Multiple distinct libraries for the same distro are stored separately.
#[tokio::test]
async fn cache_multiple_libs_same_distro() {
    let guard = common::TempDb::open("multilibs").await;

    guard
        .db()
        .set_cached_system_dep("libssl.so.3", "arch", "openssl")
        .await
        .expect("ssl set failed");
    guard
        .db()
        .set_cached_system_dep("libcurl.so.4", "arch", "curl")
        .await
        .expect("curl set failed");
    guard
        .db()
        .set_cached_system_dep("libz.so.1", "arch", "zlib")
        .await
        .expect("zlib set failed");

    assert_eq!(
        guard
            .db()
            .get_cached_system_dep("libssl.so.3", "arch")
            .await
            .unwrap(),
        Some("openssl".to_string())
    );
    assert_eq!(
        guard
            .db()
            .get_cached_system_dep("libcurl.so.4", "arch")
            .await
            .unwrap(),
        Some("curl".to_string())
    );
    assert_eq!(
        guard
            .db()
            .get_cached_system_dep("libz.so.1", "arch")
            .await
            .unwrap(),
        Some("zlib".to_string())
    );

    guard.close().await;
}
