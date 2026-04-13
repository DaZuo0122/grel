//! DNS/IP cache resolver.

use std::time::Duration;

use dashmap::DashMap;
use hickory_resolver::TokioAsyncResolver;

/// DNS cache with RTT tracking
pub struct DnsCache {
    cache: DashMap<String, DnsEntry>,
    ttl: Duration,
}

#[derive(Debug, Clone)]
pub struct DnsEntry {
    pub ip_addresses: Vec<String>,
    pub fastest_ip: Option<String>,
    pub rtt_ms: Option<u64>,
    pub expires_at: std::time::SystemTime,
}

impl DnsCache {
    /// Create a new DNS cache
    pub fn new(ttl_secs: u64) -> Self {
        Self {
            cache: DashMap::new(),
            ttl: Duration::from_secs(ttl_secs),
        }
    }

    /// Resolve a hostname to IP addresses
    pub async fn resolve(&self, hostname: &str) -> Result<DnsEntry, DnsError> {
        // Check cache first
        if let Some(entry) = self.cache.get(hostname) {
            if entry.expires_at > std::time::SystemTime::now() {
                return Ok(entry.value().clone());
            }
        }

        // Perform DNS resolution
        let resolver = TokioAsyncResolver::tokio_from_system_conf()?;
        let lookup = resolver.lookup_ip(hostname).await?;

        let ip_addresses: Vec<String> = lookup.iter().map(|ip| ip.to_string()).collect();

        // TODO: Probe IPs for fastest RTT
        let fastest_ip = ip_addresses.first().cloned();

        let entry = DnsEntry {
            ip_addresses,
            fastest_ip,
            rtt_ms: None,
            expires_at: std::time::SystemTime::now() + self.ttl,
        };

        self.cache.insert(hostname.to_string(), entry.clone());
        Ok(entry)
    }

    /// Clear expired entries
    pub fn purge_expired(&self) {
        let now = std::time::SystemTime::now();
        self.cache.retain(|_, v| v.expires_at > now);
    }

    /// Clear all entries
    pub fn clear(&self) {
        self.cache.clear();
    }
}

/// DNS resolution errors
#[derive(Debug, thiserror::Error)]
pub enum DnsError {
    #[error("DNS resolution failed: {0}")]
    ResolutionError(String),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}

impl From<hickory_resolver::error::ResolveError> for DnsError {
    fn from(e: hickory_resolver::error::ResolveError) -> Self {
        DnsError::ResolutionError(e.to_string())
    }
}
