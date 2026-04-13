//! HTTP client builder with proxy and DNS configuration.

use grel_config::GeneralConfig;
use reqwest::Client;

/// Build a configured HTTP client
pub fn build_http_client(config: &GeneralConfig) -> Result<Client, NetworkError> {
    let mut builder = Client::builder()
        // GitHub API requires a User-Agent header
        .user_agent("grel-rs/0.1.0");

    // Proxy configuration
    let proxy_url = resolve_proxy(config);
    if !proxy_url.is_empty() {
        let proxy = reqwest::Proxy::all(&proxy_url)?;
        builder = builder.proxy(proxy);
    }
    // If no explicit proxy, let reqwest auto-detect from environment
    // (respects $http_proxy, $https_proxy, $no_proxy, etc.)

    // Use rustls (default in reqwest 0.12 with rustls-tls feature)
    let client = builder.build()?;
    Ok(client)
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
    HttpError(#[from] reqwest::Error),

    #[error("URL parse error: {0}")]
    UrlParseError(#[from] url::ParseError),

    #[error("Network operation failed: {0}")]
    OperationFailed(String),
}
