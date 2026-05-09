//! SQLite database management.

use std::path::Path;

use sqlx::{Row, SqlitePool};

use crate::models::{InstalledPackage, ManifestSource, PackageStatus};

/// Database connection wrapper
pub struct Database {
    pool: SqlitePool,
}

impl Database {
    /// Initialize the database and run migrations
    pub async fn init(db_path: &Path) -> Result<Self, DatabaseError> {
        // Ensure parent directory exists
        if let Some(parent) = db_path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| {
                DatabaseError::IoError(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("Failed to create directory '{}': {e}", parent.display()),
                ))
            })?;
        }

        // Build SQLite URL. SQLite on Windows requires the `file:` URI format
        // with three slashes for absolute paths: `sqlite:///C:/path/to/db`.
        // The `?mode=rwc` flag ensures the database is created if it doesn't exist.
        let db_path_str = db_path.to_string_lossy().replace('\\', "/");
        let db_url = if cfg!(windows) {
            format!("sqlite:///{db_path_str}?mode=rwc")
        } else if db_path_str.starts_with('/') {
            format!("sqlite://{db_path_str}?mode=rwc")
        } else {
            format!("sqlite:{db_path_str}?mode=rwc")
        };
        let pool = SqlitePool::connect(&db_url).await?;

        // Run migrations
        Self::run_migrations(&pool).await?;

        Ok(Self { pool })
    }

    /// Run database schema migrations
    async fn run_migrations(pool: &SqlitePool) -> Result<(), DatabaseError> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS installed (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                forge TEXT NOT NULL,
                owner TEXT NOT NULL,
                repo TEXT NOT NULL,
                version TEXT NOT NULL,
                asset_filename TEXT NOT NULL,
                checksum TEXT,
                install_path TEXT NOT NULL,
                installed_binaries TEXT NOT NULL DEFAULT '',
                is_managed BOOLEAN NOT NULL DEFAULT 1,
                status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'orphaned', 'migrated')),
                orphaned_at INTEGER,
                last_checked INTEGER,
                installed_at INTEGER DEFAULT (strftime('%s', 'now')),
                manifest_source TEXT NOT NULL DEFAULT 'heuristic' CHECK (manifest_source IN ('registry', 'in_repo', 'heuristic'))
            )
            "#,
        )
        .execute(pool)
        .await?;

        // Add column if it doesn't exist (idempotent migration)
        // We ignore errors since the column may already exist
        sqlx::query(
            r#"ALTER TABLE installed ADD COLUMN installed_binaries TEXT NOT NULL DEFAULT ''"#,
        )
        .execute(pool)
        .await
        .ok();

        // Add manifest_source column if it doesn't exist
        sqlx::query(
            r#"ALTER TABLE installed ADD COLUMN manifest_source TEXT NOT NULL DEFAULT 'heuristic' CHECK (manifest_source IN ('registry', 'in_repo', 'heuristic'))"#,
        )
        .execute(pool)
        .await
        .ok();

        // Add is_explicit column if it doesn't exist
        sqlx::query(r#"ALTER TABLE installed ADD COLUMN is_explicit BOOLEAN NOT NULL DEFAULT 1"#)
            .execute(pool)
            .await
            .ok();

        // Create dependencies table
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS dependencies (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                package_id INTEGER NOT NULL,
                dep_forge TEXT NOT NULL,
                dep_owner TEXT NOT NULL,
                dep_repo TEXT NOT NULL,
                dep_type TEXT NOT NULL DEFAULT 'grel' CHECK (dep_type IN ('grel', 'grel_opt', 'system')),
                FOREIGN KEY (package_id) REFERENCES installed(id) ON DELETE CASCADE
            )
            "#,
        )
        .execute(pool)
        .await?;

        sqlx::query(
            r#"
            CREATE INDEX IF NOT EXISTS idx_deps_package ON dependencies(package_id)
            "#,
        )
        .execute(pool)
        .await?;

        sqlx::query(
            r#"
            CREATE INDEX IF NOT EXISTS idx_deps_target ON dependencies(dep_forge, dep_owner, dep_repo)
            "#,
        )
        .execute(pool)
        .await?;

        sqlx::query(
            r#"
            CREATE UNIQUE INDEX IF NOT EXISTS idx_pkg_unique 
            ON installed(forge, owner, repo)
            "#,
        )
        .execute(pool)
        .await?;

        sqlx::query(
            r#"
            CREATE INDEX IF NOT EXISTS idx_status 
            ON installed(status)
            "#,
        )
        .execute(pool)
        .await?;

        // Create ETag cache table
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS etag_cache (
                url TEXT PRIMARY KEY,
                etag TEXT NOT NULL,
                last_modified INTEGER NOT NULL
            )
            "#,
        )
        .execute(pool)
        .await?;

        // Create DNS cache table
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS dns_cache (
                hostname TEXT NOT NULL,
                ip_address TEXT NOT NULL,
                rtt_ms INTEGER NOT NULL,
                expires_at INTEGER NOT NULL,
                PRIMARY KEY (hostname, ip_address)
            )
            "#,
        )
        .execute(pool)
        .await?;

        Ok(())
    }

    /// Insert or update a package record
    pub async fn upsert_package(&self, pkg: &InstalledPackage) -> Result<(), DatabaseError> {
        sqlx::query(
            r#"
            INSERT INTO installed
                (forge, owner, repo, version, asset_filename, checksum, install_path,
                 installed_binaries, is_managed, status, orphaned_at, last_checked, manifest_source)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(forge, owner, repo) DO UPDATE SET
                version = excluded.version,
                asset_filename = excluded.asset_filename,
                checksum = excluded.checksum,
                install_path = excluded.install_path,
                installed_binaries = excluded.installed_binaries,
                is_managed = excluded.is_managed,
                status = excluded.status,
                orphaned_at = excluded.orphaned_at,
                last_checked = excluded.last_checked,
                manifest_source = excluded.manifest_source
            "#,
        )
        .bind(&pkg.forge)
        .bind(&pkg.owner)
        .bind(&pkg.repo)
        .bind(&pkg.version)
        .bind(&pkg.asset_filename)
        .bind(&pkg.checksum)
        .bind(&pkg.install_path)
        .bind(&pkg.installed_binaries)
        .bind(pkg.is_managed)
        .bind(pkg.status.to_string())
        .bind(pkg.orphaned_at)
        .bind(pkg.last_checked)
        .bind(pkg.manifest_source.to_string())
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Get a package by reference
    pub async fn get_package(
        &self,
        forge: &str,
        owner: &str,
        repo: &str,
    ) -> Result<Option<InstalledPackage>, DatabaseError> {
        let row = sqlx::query(
            r#"
            SELECT id, forge, owner, repo, version, asset_filename, checksum,
                   install_path, installed_binaries, is_managed, status, orphaned_at, last_checked, installed_at, manifest_source
            FROM installed
            WHERE forge = ? AND owner = ? AND repo = ?
            "#,
        )
        .bind(forge)
        .bind(owner)
        .bind(repo)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(Self::row_to_package))
    }

    /// List all packages
    pub async fn list_packages(&self) -> Result<Vec<InstalledPackage>, DatabaseError> {
        let rows = sqlx::query(
            r#"
            SELECT id, forge, owner, repo, version, asset_filename, checksum,
                   install_path, installed_binaries, is_managed, status, orphaned_at, last_checked, installed_at, manifest_source
            FROM installed
            ORDER BY forge, owner, repo
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(Self::row_to_package).collect())
    }

    /// Mark a package as orphaned
    pub async fn mark_orphaned(&self, id: i64) -> Result<(), DatabaseError> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        sqlx::query(
            r#"
            UPDATE installed 
            SET status = 'orphaned', orphaned_at = ?
            WHERE id = ?
            "#,
        )
        .bind(now)
        .bind(id)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Remove a package
    pub async fn remove_package(&self, id: i64) -> Result<(), DatabaseError> {
        sqlx::query("DELETE FROM installed WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    /// Update package version and asset info
    pub async fn update_package(
        &self,
        id: i64,
        version: &str,
        asset_filename: &str,
        checksum: Option<&str>,
        installed_binaries: Option<&str>,
        is_managed: bool,
        install_path: &str,
    ) -> Result<(), DatabaseError> {
        sqlx::query(
            r#"
            UPDATE installed
            SET version = ?, asset_filename = ?, checksum = ?,
                installed_binaries = ?,
                is_managed = ?,
                install_path = ?,
                last_checked = strftime('%s', 'now')
            WHERE id = ?
            "#,
        )
        .bind(version)
        .bind(asset_filename)
        .bind(checksum)
        .bind(installed_binaries)
        .bind(is_managed)
        .bind(install_path)
        .bind(id)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Touch the last_checked timestamp for a package
    pub async fn update_last_checked(&self, id: i64) -> Result<(), DatabaseError> {
        sqlx::query(
            r#"
            UPDATE installed
            SET last_checked = strftime('%s', 'now')
            WHERE id = ?
            "#,
        )
        .bind(id)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Helper to convert a database row to InstalledPackage
    fn row_to_package(row: sqlx::sqlite::SqliteRow) -> InstalledPackage {
        InstalledPackage {
            id: Some(row.get("id")),
            forge: row.get("forge"),
            owner: row.get("owner"),
            repo: row.get("repo"),
            version: row.get("version"),
            asset_filename: row.get("asset_filename"),
            checksum: row.get("checksum"),
            install_path: row.get("install_path"),
            installed_binaries: row.get("installed_binaries"),
            is_managed: row.get("is_managed"),
            status: PackageStatus::from_str(&row.get::<String, _>("status")),
            orphaned_at: row.get("orphaned_at"),
            last_checked: row.get("last_checked"),
            installed_at: row.get("installed_at"),
            manifest_source: ManifestSource::from_str(&row.get::<String, _>("manifest_source")),
            is_explicit: row.get("is_explicit"),
        }
    }

    /// Store ETag
    pub async fn store_etag(&self, url: &str, etag: &str) -> Result<(), DatabaseError> {
        let now = chrono::Utc::now().timestamp();

        sqlx::query(
            r#"
            INSERT INTO etag_cache (url, etag, last_modified)
            VALUES (?, ?, ?)
            ON CONFLICT(url) DO UPDATE SET etag = excluded.etag, last_modified = excluded.last_modified
            "#,
        )
        .bind(url)
        .bind(etag)
        .bind(now)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Get ETag
    pub async fn get_etag(&self, url: &str) -> Result<Option<String>, DatabaseError> {
        let row = sqlx::query("SELECT etag FROM etag_cache WHERE url = ?")
            .bind(url)
            .fetch_optional(&self.pool)
            .await?;

        Ok(row.map(|r| r.get("etag")))
    }

    /// Get dependencies for a package
    pub async fn get_dependencies(
        &self,
        package_id: i64,
    ) -> Result<Vec<crate::models::Dependency>, DatabaseError> {
        let rows = sqlx::query(
            r#"
            SELECT id, package_id, dep_forge, dep_owner, dep_repo, dep_type
            FROM dependencies
            WHERE package_id = ?
            "#,
        )
        .bind(package_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| crate::models::Dependency {
                id: Some(r.get("id")),
                package_id: r.get("package_id"),
                dep_forge: r.get("dep_forge"),
                dep_owner: r.get("dep_owner"),
                dep_repo: r.get("dep_repo"),
                dep_type: crate::models::DependencyType::from_str(&r.get::<String, _>("dep_type")),
            })
            .collect())
    }

    /// Set dependencies for a package (replaces existing)
    pub async fn set_dependencies(
        &self,
        package_id: i64,
        deps: &[crate::models::Dependency],
    ) -> Result<(), DatabaseError> {
        // Delete existing dependencies
        sqlx::query("DELETE FROM dependencies WHERE package_id = ?")
            .bind(package_id)
            .execute(&self.pool)
            .await?;

        // Insert new dependencies
        for dep in deps {
            sqlx::query(
                r#"
                INSERT INTO dependencies (package_id, dep_forge, dep_owner, dep_repo, dep_type)
                VALUES (?, ?, ?, ?, ?)
                "#,
            )
            .bind(package_id)
            .bind(&dep.dep_forge)
            .bind(&dep.dep_owner)
            .bind(&dep.dep_repo)
            .bind(dep.dep_type.to_string())
            .execute(&self.pool)
            .await?;
        }

        Ok(())
    }

    /// Get packages that depend on the given package
    pub async fn get_dependents(
        &self,
        forge: &str,
        owner: &str,
        repo: &str,
    ) -> Result<Vec<InstalledPackage>, DatabaseError> {
        let rows = sqlx::query(
            r#"
            SELECT i.id, i.forge, i.owner, i.repo, i.version, i.asset_filename, i.checksum,
                   i.install_path, i.installed_binaries, i.is_managed, i.status, i.orphaned_at, i.last_checked, i.installed_at, i.manifest_source, i.is_explicit
            FROM installed i
            JOIN dependencies d ON i.id = d.package_id
            WHERE d.dep_forge = ? AND d.dep_owner = ? AND d.dep_repo = ?
            "#,
        )
        .bind(forge)
        .bind(owner)
        .bind(repo)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(Self::row_to_package).collect())
    }

    /// Update the explicit flag for a package
    pub async fn update_explicit(
        &self,
        forge: &str,
        owner: &str,
        repo: &str,
        is_explicit: bool,
    ) -> Result<(), DatabaseError> {
        sqlx::query(
            r#"
            UPDATE installed
            SET is_explicit = ?
            WHERE forge = ? AND owner = ? AND repo = ?
            "#,
        )
        .bind(is_explicit)
        .bind(forge)
        .bind(owner)
        .bind(repo)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Get all implicitly-installed packages
    pub async fn get_implicit_packages(&self) -> Result<Vec<InstalledPackage>, DatabaseError> {
        let rows = sqlx::query(
            r#"
            SELECT id, forge, owner, repo, version, asset_filename, checksum,
                   install_path, installed_binaries, is_managed, status, orphaned_at, last_checked, installed_at, manifest_source, is_explicit
            FROM installed
            WHERE is_explicit = 0
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(Self::row_to_package).collect())
    }

    /// Close the database connection
    pub async fn close(self) {
        self.pool.close().await;
    }
}

/// Database errors
#[derive(Debug, thiserror::Error)]
pub enum DatabaseError {
    #[error("Database error: {0}")]
    SqlxError(#[from] sqlx::Error),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Package not found: {0}")]
    NotFound(String),
}
