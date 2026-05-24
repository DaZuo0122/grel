//! DNS/IP cache resolver.

use std::time::Duration;

use dashmap::DashMap;
use hickory_resolver::TokioAsyncResolver;

/// DNS cache with RTT tracking and optional database persistence.
pub struct DnsCache {
    cache: DashMap<String, DnsEntry>,
    ttl: Duration,
    db: Option<grel_cache::Database>,
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
            db: None,
        }
    }

    /// Attach a database for persistent caching.
    pub fn with_db(mut self, db: grel_cache::Database) -> Self {
        self.db = Some(db);
        self
    }

    /// Hydrate the in-memory cache from the database.
    /// Returns the number of entries loaded.
    pub async fn load_from_db(&self) -> Result<u64, DnsError> {
        let Some(ref db) = self.db else {
            return Ok(0);
        };

        let rows = db.get_dns_cache().await.map_err(|e| {
            DnsError::ResolutionError(format!("Failed to load DNS cache from DB: {e}"))
        })?;

        let mut loaded = 0u64;
        for (hostname, ip_address, rtt_ms, expires_at) in rows {
            let expires = std::time::UNIX_EPOCH + Duration::from_secs(expires_at as u64);
            if expires <= std::time::SystemTime::now() {
                continue;
            }

            self.cache
                .entry(hostname.clone())
                .and_modify(|e| {
                    if !e.ip_addresses.contains(&ip_address) {
                        e.ip_addresses.push(ip_address.clone());
                    }
                    // Keep the fastest IP with the lowest RTT
                    if rtt_ms >= 0 {
                        if e.rtt_ms.map_or(true, |r| (rtt_ms as u64) < r) {
                            e.fastest_ip = Some(ip_address.clone());
                            e.rtt_ms = Some(rtt_ms as u64);
                        }
                    }
                })
                .or_insert_with(|| DnsEntry {
                    ip_addresses: vec![ip_address.clone()],
                    fastest_ip: Some(ip_address),
                    rtt_ms: if rtt_ms >= 0 { Some(rtt_ms as u64) } else { None },
                    expires_at: expires,
                });
            loaded += 1;
        }

        Ok(loaded)
    }

    /// Resolve a hostname to IP addresses.
    /// If a database is attached, successful resolutions are persisted.
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
            ip_addresses: ip_addresses.clone(),
            fastest_ip: fastest_ip.clone(),
            rtt_ms: None,
            expires_at: std::time::SystemTime::now() + self.ttl,
        };

        self.cache.insert(hostname.to_string(), entry.clone());

        // Persist to database if available
        if let Some(ref db) = self.db {
            let expires_timestamp = entry
                .expires_at
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64;
            for ip in &ip_addresses {
                let rtt = entry.rtt_ms.unwrap_or(0) as i64;
                // Fire-and-forget: do not block the resolution on DB write
                let _ = db
                    .store_dns_entry(hostname, ip, rtt, expires_timestamp)
                    .await;
            }
        }

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

    /// Iterate over all cached entries.
    pub fn iter_entries(&self) -> impl Iterator<Item = (String, DnsEntry)> + '_ {
        self.cache.iter().map(|r| (r.key().clone(), r.value().clone()))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn load_from_db_hydrates_cache() {
        let tmp = std::env::temp_dir().join(format!(
            "grel-dns-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        let db_path = tmp.join("test.sqlite");

        let db = grel_cache::Database::init(&db_path).await.unwrap();
        let now = chrono::Utc::now().timestamp();

        db.store_dns_entry("example.com", "1.2.3.4", 10, now + 300)
            .await
            .unwrap();
        db.store_dns_entry("example.com", "5.6.7.8", 5, now + 300)
            .await
            .unwrap();

        let cache = DnsCache::new(300).with_db(db);
        let loaded = cache.load_from_db().await.unwrap();
        assert_eq!(loaded, 2);

        let entry = cache.cache.get("example.com").unwrap();
        assert_eq!(entry.ip_addresses.len(), 2);
        assert!(entry.ip_addresses.contains(&"1.2.3.4".to_string()));
        assert!(entry.ip_addresses.contains(&"5.6.7.8".to_string()));
        // Lowest RTT wins fastest_ip
        assert_eq!(entry.fastest_ip, Some("5.6.7.8".to_string()));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn load_from_db_skips_expired() {
        let tmp = std::env::temp_dir().join(format!(
            "grel-dns-expired-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        let db_path = tmp.join("test.sqlite");

        let db = grel_cache::Database::init(&db_path).await.unwrap();
        let now = chrono::Utc::now().timestamp();

        db.store_dns_entry("old.example.com", "1.1.1.1", 10, now - 10)
            .await
            .unwrap();

        let cache = DnsCache::new(300).with_db(db);
        let loaded = cache.load_from_db().await.unwrap();
        assert_eq!(loaded, 0);
        assert!(!cache.cache.contains_key("old.example.com"));

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
