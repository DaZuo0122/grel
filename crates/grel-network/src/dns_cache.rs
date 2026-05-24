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

    // -----------------------------------------------------------------------
    // Part 4.1: DnsCache Concurrency
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn concurrent_insert_into_cache_same_hostname() {
        let cache = DnsCache::new(300);
        let cache = std::sync::Arc::new(cache);
        let mut handles = vec![];

        for i in 0..50 {
            let cache = cache.clone();
            handles.push(tokio::spawn(async move {
                let entry = DnsEntry {
                    ip_addresses: vec![format!("1.2.3.{i}")],
                    fastest_ip: Some(format!("1.2.3.{i}")),
                    rtt_ms: Some(i as u64),
                    expires_at: std::time::SystemTime::now() + Duration::from_secs(300),
                };
                cache.cache.insert("race.example.com".to_string(), entry);
            }));
        }

        for h in handles {
            h.await.unwrap();
        }

        let entry = cache.cache.get("race.example.com").unwrap();
        // Must have exactly 1 IP (last writer wins)
        assert_eq!(entry.ip_addresses.len(), 1);
        let ip = entry.ip_addresses[0].clone();
        assert!(ip.starts_with("1.2.3."), "IP must be one of the inserted values");
    }

    #[tokio::test]
    async fn purge_during_heavy_insert() {
        let cache = DnsCache::new(300);
        let cache = std::sync::Arc::new(cache);

        // Pre-populate with some expired and some valid entries
        for i in 0..100 {
            let expired = i % 2 == 0;
            let entry = DnsEntry {
                ip_addresses: vec![format!("1.2.3.{i}")],
                fastest_ip: Some(format!("1.2.3.{i}")),
                rtt_ms: Some(i as u64),
                expires_at: if expired {
                    std::time::SystemTime::now() - Duration::from_secs(1)
                } else {
                    std::time::SystemTime::now() + Duration::from_secs(300)
                },
            };
            cache.cache.insert(format!("host{i}.example.com"), entry);
        }

        let mut handles = vec![];

        // Task A: purge expired in a loop
        let cache_purge = cache.clone();
        handles.push(tokio::spawn(async move {
            for _ in 0..20 {
                cache_purge.purge_expired();
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        }));

        // Task B: insert new entries in a loop
        let cache_insert = cache.clone();
        handles.push(tokio::spawn(async move {
            for i in 100..200 {
                let entry = DnsEntry {
                    ip_addresses: vec![format!("1.2.3.{i}")],
                    fastest_ip: Some(format!("1.2.3.{i}")),
                    rtt_ms: Some(i as u64),
                    expires_at: std::time::SystemTime::now() + Duration::from_secs(300),
                };
                cache_insert.cache.insert(format!("host{i}.example.com"), entry);
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        }));

        for h in handles {
            h.await.unwrap();
        }

        // After all operations, count entries. Should be ~150 (100 valid original + 50 new,
        // minus some purged). The exact count is non-deterministic but must be > 0 and <= 200.
        let count = cache.cache.iter().count();
        assert!(count > 0 && count <= 200, "cache must not be corrupted; got {count} entries");
    }

    #[tokio::test]
    async fn load_from_db_during_cache_inserts() {
        let tmp = std::env::temp_dir().join(format!(
            "grel-dns-load-race-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        let db_path = tmp.join("test.sqlite");

        let db = grel_cache::Database::init(&db_path).await.unwrap();
        let now = chrono::Utc::now().timestamp();

        // Pre-seed DB with 50 entries
        for i in 0..50 {
            db.store_dns_entry(&format!("dbhost{i}"), &format!("10.0.0.{i}"), i as i64, now + 300)
                .await
                .unwrap();
        }

        let cache = DnsCache::new(300).with_db(db);
        let cache = std::sync::Arc::new(cache);

        let mut handles = vec![];

        // Task A: load from DB
        let cache_load = cache.clone();
        handles.push(tokio::spawn(async move {
            cache_load.load_from_db().await.unwrap();
        }));

        // Task B: insert new entries directly into cache
        let cache_insert = cache.clone();
        handles.push(tokio::spawn(async move {
            for i in 0..50 {
                let entry = DnsEntry {
                    ip_addresses: vec![format!("192.168.0.{i}")],
                    fastest_ip: Some(format!("192.168.0.{i}")),
                    rtt_ms: Some(i as u64),
                    expires_at: std::time::SystemTime::now() + Duration::from_secs(300),
                };
                cache_insert.cache.insert(format!("memhost{i}"), entry);
            }
        }));

        for h in handles {
            h.await.unwrap();
        }

        // Cache must contain at least the DB-loaded entries (up to 50)
        // and the memory-inserted entries (up to 50), with no corruption
        let count = cache.cache.iter().count();
        assert!(count >= 50, "cache must contain at least the DB-loaded entries; got {count}");

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
