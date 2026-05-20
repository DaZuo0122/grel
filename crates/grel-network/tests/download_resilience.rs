//! Mock-based tests for download resilience features.

use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use grel_network::build_http_client;
use grel_network::download::download_file;

fn test_config() -> grel_config::GeneralConfig {
    grel_config::GeneralConfig {
        version: 1,
        max_concurrent: 4,
        proxy: String::new(),
        keep_archives: true,
        max_retries: 3,
        retry_delay_ms: 1000,
        timeout_secs: 300,
        connect_timeout_secs: 30,
        pool_max_idle: 10,
    }
}

#[tokio::test]
async fn download_completes_successfully() {
    let server = MockServer::start().await;
    let body = b"hello world";

    Mock::given(method("GET"))
        .and(path("/file"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(body.as_slice()))
        .mount(&server)
        .await;

    let client = build_http_client(&test_config()).unwrap();
    let dest = tempfile::NamedTempFile::new().unwrap().into_temp_path();

    let (checksum, etag) = download_file(&client, &format!("{}/file", server.uri()), &dest, None, false, None)
        .await
        .unwrap();

    assert_eq!(checksum, "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9");
    assert!(etag.is_none());
    assert_eq!(tokio::fs::read(&dest).await.unwrap(), body);
}

#[tokio::test]
async fn download_with_etag_conditional_request() {
    let server = MockServer::start().await;
    let body = b"cached content";
    let etag = "\"abc123\"";

    // First request: no ETag, returns 200 with ETag
    Mock::given(method("GET"))
        .and(path("/file"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("etag", etag)
                .set_body_bytes(body.as_slice()),
        )
        .up_to_n_times(1)
        .expect(1)
        .mount(&server)
        .await;

    // Second request: with If-None-Match, returns 304
    Mock::given(method("GET"))
        .and(path("/file"))
        .and(header("If-None-Match", etag))
        .respond_with(ResponseTemplate::new(304).insert_header("etag", etag))
        .expect(1)
        .mount(&server)
        .await;

    let client = build_http_client(&test_config()).unwrap();
    let dest = tempfile::NamedTempFile::new().unwrap().into_temp_path();

    // First download
    let (checksum1, etag1) = download_file(&client, &format!("{}/file", server.uri()), &dest, None, false, None)
        .await
        .unwrap();
    assert_eq!(etag1, Some(etag.to_string()));

    // Second download with cached ETag
    let (checksum2, etag2) = download_file(
        &client,
        &format!("{}/file", server.uri()),
        &dest,
        None,
        false,
        Some(etag),
    )
    .await
    .unwrap();

    // 304 should return the same checksum (re-read from disk)
    assert_eq!(checksum1, checksum2);
    assert_eq!(etag2, Some(etag.to_string()));
}

#[tokio::test]
async fn download_resumes_partial_file() {
    let server = MockServer::start().await;
    let full_body = b"hello world full content";
    let partial = b"hello world";

    // Request with Range header for partial content
    Mock::given(method("GET"))
        .and(path("/file"))
        .and(header("Range", "bytes=11-"))
        .respond_with(
            ResponseTemplate::new(206)
                .insert_header("content-range", "bytes 11-23/24")
                .set_body_bytes(&full_body[11..]),
        )
        .mount(&server)
        .await;

    let client = build_http_client(&test_config()).unwrap();
    let dest = tempfile::NamedTempFile::new().unwrap().into_temp_path();

    // Write partial content first
    tokio::fs::write(&dest, partial).await.unwrap();

    let (checksum, _etag) = download_file(
        &client,
        &format!("{}/file", server.uri()),
        &dest,
        None,
        true,
        None,
    )
    .await
    .unwrap();

    // Verify the full file was assembled
    let final_content = tokio::fs::read(&dest).await.unwrap();
    assert_eq!(final_content, full_body);

    // Verify checksum matches full content
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(full_body);
    let expected = format!("{:x}", hasher.finalize());
    assert_eq!(checksum, expected);
}

#[tokio::test]
async fn download_range_not_satisfiable_means_complete() {
    let server = MockServer::start().await;
    let body = b"already complete";

    Mock::given(method("GET"))
        .and(path("/file"))
        .and(header("Range", "bytes=16-"))
        .respond_with(ResponseTemplate::new(416))
        .mount(&server)
        .await;

    let client = build_http_client(&test_config()).unwrap();
    let dest = tempfile::NamedTempFile::new().unwrap().into_temp_path();

    // Write complete content first
    tokio::fs::write(&dest, body).await.unwrap();

    let (checksum, _etag) = download_file(
        &client,
        &format!("{}/file", server.uri()),
        &dest,
        None,
        true,
        None,
    )
    .await
    .unwrap();

    // Should compute checksum of existing file
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(body);
    let expected = format!("{:x}", hasher.finalize());
    assert_eq!(checksum, expected);
}

#[tokio::test]
async fn download_server_error_fails_immediately() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/file"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let client = build_http_client(&test_config()).unwrap();
    let dest = tempfile::NamedTempFile::new().unwrap().into_temp_path();

    let result = download_file(&client, &format!("{}/file", server.uri()), &dest, None, false, None).await;

    assert!(result.is_err());
}

#[tokio::test]
async fn download_not_found_fails_immediately() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/file"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let client = build_http_client(&test_config()).unwrap();
    let dest = tempfile::NamedTempFile::new().unwrap().into_temp_path();

    let result = download_file(&client, &format!("{}/file", server.uri()), &dest, None, false, None).await;

    assert!(result.is_err());
}
