//! HTTP client builder with proxy, DNS, and retry configuration.

use std::time::Duration;

use grel_config::GeneralConfig;
use reqwest_middleware::ClientBuilder;
use reqwest_retry::{RetryTransientMiddleware, policies::ExponentialBackoff};

use crate::DnsCache;

/// Re-export of the underlying HTTP client type (with retry middleware)
pub type Client = reqwest_middleware::ClientWithMiddleware;

/// Build a configured HTTP client with retry middleware.
///
/// If a `dns_cache` is provided, cached hostnames are pre-resolved so that
/// reqwest skips the system resolver for those domains.
pub fn build_http_client(
    config: &GeneralConfig,
    dns_cache: Option<&DnsCache>,
) -> Result<Client, NetworkError> {
    let mut builder = reqwest::Client::builder()
        // GitHub API requires a User-Agent header
        .user_agent("grel-rs/0.1.0")
        .timeout(Duration::from_secs(config.timeout_secs))
        .connect_timeout(Duration::from_secs(config.connect_timeout_secs))
        .pool_max_idle_per_host(config.pool_max_idle);

    // Pre-resolve cached hostnames
    if let Some(cache) = dns_cache {
        for (hostname, entry) in cache.iter_entries() {
            if let Some(ip_str) = &entry.fastest_ip {
                if let Ok(addr) = ip_str.parse::<std::net::IpAddr>() {
                    builder = builder.resolve(&hostname, std::net::SocketAddr::new(addr, 443));
                }
            }
        }
    }

    // Proxy configuration
    let proxy_url = resolve_proxy(config);
    if !proxy_url.is_empty() {
        let proxy = reqwest::Proxy::all(&proxy_url)?;
        builder = builder.proxy(proxy);
    }
    // If no explicit proxy, let reqwest auto-detect from environment
    // (respects $http_proxy, $https_proxy, $no_proxy, etc.)

    // Use rustls (default in reqwest 0.12 with rustls-tls feature)
    let base = builder.build()?;

    let retry_policy = ExponentialBackoff::builder()
        .retry_bounds(
            Duration::from_millis(config.retry_delay_ms),
            Duration::from_secs(60),
        )
        .build_with_max_retries(config.max_retries);

    Ok(ClientBuilder::new(base)
        .with(RetryTransientMiddleware::new_with_policy(retry_policy))
        .build())
}

/// Resolve proxy from config or environment
fn resolve_proxy(config: &GeneralConfig) -> String {
    // Priority: config.proxy > $http_proxy > $all_proxy
    if !config.proxy.is_empty() {
        return config.proxy.clone();
    }

    std::env::var("http_proxy")
        .or_else(|_| std::env::var("all_proxy"))
        .unwrap_or_default()
}

/// Network errors
#[derive(Debug, thiserror::Error)]
pub enum NetworkError {
    #[error("HTTP error: {0}")]
    HttpError(#[from] reqwest_middleware::Error),

    #[error("Reqwest error: {0}")]
    ReqwestError(#[from] reqwest::Error),

    #[error("URL parse error: {0}")]
    UrlParseError(#[from] url::ParseError),

    #[error("Network operation failed: {0}")]
    OperationFailed(String),
}
