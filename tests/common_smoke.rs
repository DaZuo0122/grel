mod common;

#[tokio::test]
async fn temp_db_opens_and_cleans_up() {
    let guard = common::TempDb::open("smoke").await;
    assert!(guard.path.exists());
    // Insert something
    let pkg = common::fake_package("github", "owner", "repo");
    guard.db().upsert_package(&pkg).await.unwrap();
    let count = common::count_rows(guard.db(), "installed").await;
    assert_eq!(count, 1);
    guard.close().await;
}

#[test]
fn fake_archive_creates_valid_tar_gz() {
    let tmp = common::temp_dir("archive-smoke");
    let archive = tmp.join("test.tar.gz");
    common::fake_archive(&archive, &[("hello.txt", b"world"), ("bin/rg", b"binary")]);
    assert!(archive.exists());
    assert!(archive.metadata().unwrap().len() > 0);
    common::cleanup(&tmp);
}

#[test]
fn corrupt_archive_is_not_valid() {
    let tmp = common::temp_dir("corrupt-smoke");
    let archive = tmp.join("bad.tar.gz");
    common::corrupt_archive(&archive);
    assert!(archive.exists());
    common::cleanup(&tmp);
}

#[test]
fn count_files_works() {
    let tmp = common::temp_dir("count-files");
    std::fs::create_dir_all(tmp.join("a/b")).unwrap();
    std::fs::write(tmp.join("a/1.txt"), b"1").unwrap();
    std::fs::write(tmp.join("a/b/2.txt"), b"2").unwrap();
    std::fs::write(tmp.join("3.txt"), b"3").unwrap();
    assert_eq!(common::count_files(&tmp), 3);
    common::cleanup(&tmp);
}
