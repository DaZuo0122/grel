//! SQLite database management.

use std::path::Path;

use sqlx::{Row, SqlitePool};

use crate::models::{InstalledPackage, ManifestSource, PackageStatus};

/// Database connection wrapper
#[derive(Clone)]
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
                manifest_source TEXT NOT NULL DEFAULT 'heuristic' CHECK (manifest_source IN ('registry', 'in_repo', 'heuristic')),
                is_explicit BOOLEAN NOT NULL DEFAULT 1
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

        // Migrate dependencies table from old schema (dep_forge/dep_owner/dep_repo)
        // to new schema (dep_target) if needed.
        Self::migrate_dependencies_schema(pool).await?;

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

        // Create system dependency cache table
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS system_dep_cache (
                library_name TEXT NOT NULL,
                distro_id TEXT NOT NULL,
                package_name TEXT NOT NULL,
                discovered_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
                PRIMARY KEY (library_name, distro_id)
            )
            "#,
        )
        .execute(pool)
        .await?;

        // Create package files index table
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS package_files (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                package_id INTEGER NOT NULL,
                file_path TEXT NOT NULL,
                file_type TEXT NOT NULL DEFAULT 'data',
                FOREIGN KEY (package_id) REFERENCES installed(id) ON DELETE CASCADE
            )
            "#,
        )
        .execute(pool)
        .await?;

        sqlx::query(
            r#"
            CREATE UNIQUE INDEX IF NOT EXISTS idx_pkg_file_unique
            ON package_files(package_id, file_path)
            "#,
        )
        .execute(pool)
        .await?;

        sqlx::query(
            r#"
            CREATE INDEX IF NOT EXISTS idx_pkg_file_path
            ON package_files(file_path)
            "#,
        )
        .execute(pool)
        .await?;

        // Create release metadata cache table
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS release_cache (
                forge TEXT NOT NULL,
                owner TEXT NOT NULL,
                repo TEXT NOT NULL,
                tag TEXT NOT NULL,
                release_json TEXT NOT NULL,
                cached_at INTEGER NOT NULL,
                expires_at INTEGER NOT NULL,
                PRIMARY KEY (forge, owner, repo, tag)
            )
            "#,
        )
        .execute(pool)
        .await?;

        // Create manifest content cache table
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS manifest_cache (
                owner TEXT NOT NULL,
                repo TEXT NOT NULL,
                ref_name TEXT NOT NULL,
                path TEXT NOT NULL,
                content TEXT NOT NULL,
                etag TEXT,
                cached_at INTEGER NOT NULL,
                expires_at INTEGER NOT NULL,
                PRIMARY KEY (owner, repo, ref_name, path)
            )
            "#,
        )
        .execute(pool)
        .await?;

        // Create rate limit tracking table
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS rate_limit_cache (
                provider TEXT NOT NULL PRIMARY KEY,
                remaining INTEGER NOT NULL,
                reset_at INTEGER NOT NULL,
                last_checked INTEGER NOT NULL
            )
            "#,
        )
        .execute(pool)
        .await?;

        Ok(())
    }

    /// Migrate the dependencies table from the old schema
    /// (dep_forge/dep_owner/dep_repo) to the new schema (dep_target).
    async fn migrate_dependencies_schema(pool: &SqlitePool) -> Result<(), DatabaseError> {
        // Check if the old dependencies table exists with dep_forge column
        let old_schema_exists = sqlx::query(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name='dependencies'",
        )
        .fetch_optional(pool)
        .await?
        .is_some();

        if !old_schema_exists {
            // Fresh database — create the new table directly
            sqlx::query(
                r#"
                CREATE TABLE dependencies (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    package_id INTEGER NOT NULL,
                    dep_target TEXT NOT NULL,
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
                CREATE INDEX IF NOT EXISTS idx_deps_target ON dependencies(dep_target)
                "#,
            )
            .execute(pool)
            .await?;

            return Ok(());
        }

        // Check if the existing table already has dep_target (new schema)
        let has_new_schema = sqlx::query(
            "SELECT 1 FROM pragma_table_info('dependencies') WHERE name = 'dep_target'",
        )
        .fetch_optional(pool)
        .await?
        .is_some();

        if has_new_schema {
            // Ensure indexes exist
            sqlx::query(
                r#"
                CREATE INDEX IF NOT EXISTS idx_deps_package ON dependencies(package_id)
                "#,
            )
            .execute(pool)
            .await?;

            sqlx::query(
                r#"
                CREATE INDEX IF NOT EXISTS idx_deps_target ON dependencies(dep_target)
                "#,
            )
            .execute(pool)
            .await?;

            return Ok(());
        }

        // Old schema detected — recreate in-place
        sqlx::query("ALTER TABLE dependencies RENAME TO dependencies_old")
            .execute(pool)
            .await?;

        sqlx::query(
            r#"
            CREATE TABLE dependencies (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                package_id INTEGER NOT NULL,
                dep_target TEXT NOT NULL,
                dep_type TEXT NOT NULL DEFAULT 'grel' CHECK (dep_type IN ('grel', 'grel_opt', 'system')),
                FOREIGN KEY (package_id) REFERENCES installed(id) ON DELETE CASCADE
            )
            "#,
        )
        .execute(pool)
        .await?;

        sqlx::query(
            r#"
            CREATE INDEX idx_deps_package ON dependencies(package_id)
            "#,
        )
        .execute(pool)
        .await?;

        sqlx::query(
            r#"
            CREATE INDEX idx_deps_target ON dependencies(dep_target)
            "#,
        )
        .execute(pool)
        .await?;

        // Migrate old data: dep_target = dep_forge || '/' || dep_owner || '/' || dep_repo
        sqlx::query(
            r#"
            INSERT INTO dependencies (package_id, dep_target, dep_type)
            SELECT package_id, dep_forge || '/' || dep_owner || '/' || dep_repo, dep_type
            FROM dependencies_old
            "#,
        )
        .execute(pool)
        .await?;

        sqlx::query("DROP TABLE dependencies_old").execute(pool).await?;

        Ok(())
    }

    /// Insert or update a package record
    pub async fn upsert_package(&self, pkg: &InstalledPackage) -> Result<(), DatabaseError> {
        sqlx::query(
            r#"
            INSERT INTO installed
                (forge, owner, repo, version, asset_filename, checksum, install_path,
                 installed_binaries, is_managed, status, orphaned_at, last_checked, manifest_source, is_explicit)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
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
                manifest_source = excluded.manifest_source,
                is_explicit = excluded.is_explicit
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
        .bind(pkg.is_explicit)
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
                   install_path, installed_binaries, is_managed, status, orphaned_at, last_checked, installed_at, manifest_source, is_explicit
            FROM installed
            WHERE forge = ? AND owner = ? AND repo = ?
            "#,
        )
        .bind(forge)
        .bind(owner)
        .bind(repo)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| Self::row_to_package(&r)))
    }

    /// List all packages
    pub async fn list_packages(&self) -> Result<Vec<InstalledPackage>, DatabaseError> {
        let rows = sqlx::query(
            r#"
            SELECT id, forge, owner, repo, version, asset_filename, checksum,
                   install_path, installed_binaries, is_managed, status, orphaned_at, last_checked, installed_at, manifest_source, is_explicit
            FROM installed
            ORDER BY forge, owner, repo
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(|r| Self::row_to_package(&r)).collect())
    }

    /// Mark a package as orphaned
    pub async fn mark_orphaned(&self, id: i64) -> Result<(), DatabaseError> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or(std::time::Duration::ZERO)
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

    /// Remove a package and its file index entries
    pub async fn remove_package(&self, id: i64) -> Result<(), DatabaseError> {
        // Delete file index entries first (SQLite FK cascade may be off)
        sqlx::query("DELETE FROM package_files WHERE package_id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;

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
    fn row_to_package(row: &sqlx::sqlite::SqliteRow) -> InstalledPackage {
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
            SELECT id, package_id, dep_target, dep_type
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
                dep_target: r.get("dep_target"),
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
                INSERT INTO dependencies (package_id, dep_target, dep_type)
                VALUES (?, ?, ?)
                "#,
            )
            .bind(package_id)
            .bind(&dep.dep_target)
            .bind(dep.dep_type.to_string())
            .execute(&self.pool)
            .await?;
        }

        Ok(())
    }

    /// Get packages that depend on the given dependency target.
    ///
    /// `dep_target` is the full dependency string, e.g.:
    /// - `"github/owner/repo"` for grel packages
    /// - `"system:libssl.so.3"` for system libraries
    pub async fn get_dependents(
        &self,
        dep_target: &str,
    ) -> Result<Vec<InstalledPackage>, DatabaseError> {
        let rows = sqlx::query(
            r#"
            SELECT i.id, i.forge, i.owner, i.repo, i.version, i.asset_filename, i.checksum,
                   i.install_path, i.installed_binaries, i.is_managed, i.status, i.orphaned_at, i.last_checked, i.installed_at, i.manifest_source, i.is_explicit
            FROM installed i
            JOIN dependencies d ON i.id = d.package_id
            WHERE d.dep_target = ?
            "#,
        )
        .bind(dep_target)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(|r| Self::row_to_package(&r)).collect())
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

        Ok(rows.into_iter().map(|r| Self::row_to_package(&r)).collect())
    }

    /// Look up a cached library → package mapping for the given distro.
    pub async fn get_cached_system_dep(
        &self,
        lib: &str,
        distro: &str,
    ) -> Result<Option<String>, DatabaseError> {
        let row = sqlx::query(
            "SELECT package_name FROM system_dep_cache WHERE library_name = ? AND distro_id = ?",
        )
        .bind(lib)
        .bind(distro)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| r.get("package_name")))
    }

    /// Store a library → package mapping for the given distro.
    pub async fn set_cached_system_dep(
        &self,
        lib: &str,
        distro: &str,
        pkg: &str,
    ) -> Result<(), DatabaseError> {
        sqlx::query(
            r#"
            INSERT INTO system_dep_cache (library_name, distro_id, package_name)
            VALUES (?, ?, ?)
            ON CONFLICT(library_name, distro_id) DO UPDATE SET
                package_name = excluded.package_name,
                discovered_at = strftime('%s', 'now')
            "#,
        )
        .bind(lib)
        .bind(distro)
        .bind(pkg)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Add file records for a package (replaces existing)
    pub async fn set_package_files(
        &self,
        package_id: i64,
        files: &[crate::models::PackageFile],
    ) -> Result<(), DatabaseError> {
        // Delete existing files for this package
        sqlx::query("DELETE FROM package_files WHERE package_id = ?")
            .bind(package_id)
            .execute(&self.pool)
            .await?;

        // Insert new files
        for file in files {
            sqlx::query(
                r#"
                INSERT INTO package_files (package_id, file_path, file_type)
                VALUES (?, ?, ?)
                "#,
            )
            .bind(package_id)
            .bind(&file.file_path)
            .bind(&file.file_type)
            .execute(&self.pool)
            .await?;
        }

        Ok(())
    }

    /// Get all file records for a package
    pub async fn get_package_files(
        &self,
        package_id: i64,
    ) -> Result<Vec<crate::models::PackageFile>, DatabaseError> {
        let rows = sqlx::query(
            r#"
            SELECT id, package_id, file_path, file_type
            FROM package_files
            WHERE package_id = ?
            ORDER BY file_path
            "#,
        )
        .bind(package_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| crate::models::PackageFile {
                id: Some(r.get("id")),
                package_id: r.get("package_id"),
                file_path: r.get("file_path"),
                file_type: r.get("file_type"),
            })
            .collect())
    }

    /// Search file paths by pattern (case-insensitive LIKE)
    pub async fn search_package_files(
        &self,
        pattern: &str,
    ) -> Result<Vec<(crate::models::InstalledPackage, crate::models::PackageFile)>, DatabaseError>
    {
        let like_pattern = format!("%{pattern}%");
        let rows = sqlx::query(
            r#"
            SELECT i.id, i.forge, i.owner, i.repo, i.version, i.asset_filename, i.checksum,
                   i.install_path, i.installed_binaries, i.is_managed, i.status, i.orphaned_at, i.last_checked, i.installed_at, i.manifest_source, i.is_explicit,
                   f.id as file_id, f.package_id, f.file_path, f.file_type
            FROM package_files f
            JOIN installed i ON f.package_id = i.id
            WHERE LOWER(f.file_path) LIKE LOWER(?)
            ORDER BY i.forge, i.owner, i.repo, f.file_path
            "#,
        )
        .bind(&like_pattern)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| {
                let pkg = Self::row_to_package(&r);
                let file = crate::models::PackageFile {
                    id: Some(r.get("file_id")),
                    package_id: r.get("package_id"),
                    file_path: r.get("file_path"),
                    file_type: r.get("file_type"),
                };
                (pkg, file)
            })
            .collect())
    }

    /// Find packages that own a file by exact path match
    pub async fn find_package_by_file_path(
        &self,
        path: &str,
    ) -> Result<Vec<crate::models::InstalledPackage>, DatabaseError> {
        let rows = sqlx::query(
            r#"
            SELECT i.id, i.forge, i.owner, i.repo, i.version, i.asset_filename, i.checksum,
                   i.install_path, i.installed_binaries, i.is_managed, i.status, i.orphaned_at, i.last_checked, i.installed_at, i.manifest_source, i.is_explicit
            FROM installed i
            JOIN package_files f ON i.id = f.package_id
            WHERE f.file_path = ?
            "#,
        )
        .bind(path)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(|r| Self::row_to_package(&r)).collect())
    }

    /// Find packages that own a file by filename (basename) match
    pub async fn find_package_by_filename(
        &self,
        filename: &str,
    ) -> Result<Vec<crate::models::InstalledPackage>, DatabaseError> {
        let rows = sqlx::query(
            r#"
            SELECT DISTINCT i.id, i.forge, i.owner, i.repo, i.version, i.asset_filename, i.checksum,
                   i.install_path, i.installed_binaries, i.is_managed, i.status, i.orphaned_at, i.last_checked, i.installed_at, i.manifest_source, i.is_explicit
            FROM installed i
            JOIN package_files f ON i.id = f.package_id
            WHERE f.file_path LIKE '%' || ?
            "#,
        )
        .bind(filename)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(|r| Self::row_to_package(&r)).collect())
    }

    /// Check if any file paths would conflict with existing packages
    pub async fn find_conflicting_files(
        &self,
        file_paths: &[String],
    ) -> Result<Vec<(crate::models::InstalledPackage, String)>, DatabaseError> {
        if file_paths.is_empty() {
            return Ok(vec![]);
        }

        // Build a parameterized query with multiple OR conditions
        let mut query_str = String::from(
            r#"
            SELECT i.id, i.forge, i.owner, i.repo, i.version, i.asset_filename, i.checksum,
                   i.install_path, i.installed_binaries, i.is_managed, i.status, i.orphaned_at, i.last_checked, i.installed_at, i.manifest_source, i.is_explicit,
                   f.file_path as conflict_path
            FROM installed i
            JOIN package_files f ON i.id = f.package_id
            WHERE f.file_path IN (
            "#,
        );
        let placeholders: Vec<String> = (0..file_paths.len()).map(|i| format!("?{}", i + 1)).collect();
        query_str.push_str(&placeholders.join(", "));
        query_str.push(')');

        let mut query = sqlx::query(&query_str);
        for path in file_paths {
            query = query.bind(path);
        }

        let rows = query.fetch_all(&self.pool).await?;

        Ok(rows
            .into_iter()
            .map(|r| {
                let pkg = Self::row_to_package(&r);
                let conflict_path: String = r.get("conflict_path");
                (pkg, conflict_path)
            })
            .collect())
    }

    /// Remove ETag cache entries older than 30 days.
    pub async fn clean_etag_cache(&self) -> Result<u64, DatabaseError> {
        let result = sqlx::query(
            "DELETE FROM etag_cache WHERE last_modified < strftime('%s', 'now') - 86400 * 30",
        )
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected())
    }

    /// Load all non-expired DNS cache entries.
    pub async fn get_dns_cache(&self) -> Result<Vec<(String, String, i64, i64)>, DatabaseError> {
        let rows = sqlx::query(
            "SELECT hostname, ip_address, rtt_ms, expires_at FROM dns_cache WHERE expires_at >= strftime('%s', 'now')",
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| {
                (
                    r.get::<String, _>("hostname"),
                    r.get::<String, _>("ip_address"),
                    r.get::<i64, _>("rtt_ms"),
                    r.get::<i64, _>("expires_at"),
                )
            })
            .collect())
    }

    /// Store a single DNS cache entry.
    pub async fn store_dns_entry(
        &self,
        hostname: &str,
        ip_address: &str,
        rtt_ms: i64,
        expires_at: i64,
    ) -> Result<(), DatabaseError> {
        sqlx::query(
            "INSERT OR REPLACE INTO dns_cache (hostname, ip_address, rtt_ms, expires_at) VALUES (?, ?, ?, ?)",
        )
        .bind(hostname)
        .bind(ip_address)
        .bind(rtt_ms)
        .bind(expires_at)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Remove expired DNS cache entries.
    pub async fn clean_dns_cache(&self) -> Result<u64, DatabaseError> {
        let result = sqlx::query(
            "DELETE FROM dns_cache WHERE expires_at < strftime('%s', 'now')",
        )
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected())
    }

    // -----------------------------------------------------------------------
    // Release metadata cache
    // -----------------------------------------------------------------------

    /// Load a cached release if it has not expired.
    pub async fn get_release_cache(
        &self,
        forge: &str,
        owner: &str,
        repo: &str,
        tag: &str,
    ) -> Result<Option<String>, DatabaseError> {
        let row = sqlx::query(
            "SELECT release_json FROM release_cache WHERE forge = ? AND owner = ? AND repo = ? AND tag = ? AND expires_at >= strftime('%s', 'now')",
        )
        .bind(forge)
        .bind(owner)
        .bind(repo)
        .bind(tag)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| r.get::<String, _>("release_json")))
    }

    /// Store a release in the cache with a given TTL (in seconds).
    pub async fn store_release_cache(
        &self,
        forge: &str,
        owner: &str,
        repo: &str,
        tag: &str,
        release_json: &str,
        ttl_secs: i64,
    ) -> Result<(), DatabaseError> {
        let now = chrono::Utc::now().timestamp();
        sqlx::query(
            "INSERT OR REPLACE INTO release_cache (forge, owner, repo, tag, release_json, cached_at, expires_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(forge)
        .bind(owner)
        .bind(repo)
        .bind(tag)
        .bind(release_json)
        .bind(now)
        .bind(now + ttl_secs)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Remove expired release cache entries.
    pub async fn clean_release_cache(&self) -> Result<u64, DatabaseError> {
        let result = sqlx::query(
            "DELETE FROM release_cache WHERE expires_at < strftime('%s', 'now')",
        )
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected())
    }

    // -----------------------------------------------------------------------
    // Manifest content cache
    // -----------------------------------------------------------------------

    /// Load a cached manifest if it has not expired.
    pub async fn get_manifest_cache(
        &self,
        owner: &str,
        repo: &str,
        ref_name: &str,
        path: &str,
    ) -> Result<Option<(String, Option<String>)>, DatabaseError> {
        let row = sqlx::query(
            "SELECT content, etag FROM manifest_cache WHERE owner = ? AND repo = ? AND ref_name = ? AND path = ? AND expires_at >= strftime('%s', 'now')",
        )
        .bind(owner)
        .bind(repo)
        .bind(ref_name)
        .bind(path)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| {
            (
                r.get::<String, _>("content"),
                r.get::<Option<String>, _>("etag"),
            )
        }))
    }

    /// Store a manifest in the cache with a given TTL (in seconds).
    pub async fn store_manifest_cache(
        &self,
        owner: &str,
        repo: &str,
        ref_name: &str,
        path: &str,
        content: &str,
        etag: Option<&str>,
        ttl_secs: i64,
    ) -> Result<(), DatabaseError> {
        let now = chrono::Utc::now().timestamp();
        sqlx::query(
            "INSERT OR REPLACE INTO manifest_cache (owner, repo, ref_name, path, content, etag, cached_at, expires_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(owner)
        .bind(repo)
        .bind(ref_name)
        .bind(path)
        .bind(content)
        .bind(etag)
        .bind(now)
        .bind(now + ttl_secs)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Remove expired manifest cache entries.
    pub async fn clean_manifest_cache(&self) -> Result<u64, DatabaseError> {
        let result = sqlx::query(
            "DELETE FROM manifest_cache WHERE expires_at < strftime('%s', 'now')",
        )
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected())
    }

    // -----------------------------------------------------------------------
    // Rate limit tracking
    // -----------------------------------------------------------------------

    /// Load the most recent rate limit state for a provider.
    pub async fn get_rate_limit(
        &self,
        provider: &str,
    ) -> Result<Option<(i64, i64, i64)>, DatabaseError> {
        let row = sqlx::query(
            "SELECT remaining, reset_at, last_checked FROM rate_limit_cache WHERE provider = ?",
        )
        .bind(provider)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| {
            (
                r.get::<i64, _>("remaining"),
                r.get::<i64, _>("reset_at"),
                r.get::<i64, _>("last_checked"),
            )
        }))
    }

    /// Store rate limit state for a provider.
    pub async fn store_rate_limit(
        &self,
        provider: &str,
        remaining: i64,
        reset_at: i64,
    ) -> Result<(), DatabaseError> {
        let now = chrono::Utc::now().timestamp();
        sqlx::query(
            "INSERT OR REPLACE INTO rate_limit_cache (provider, remaining, reset_at, last_checked) VALUES (?, ?, ?, ?)",
        )
        .bind(provider)
        .bind(remaining)
        .bind(reset_at)
        .bind(now)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Run SQLite PRAGMA integrity_check
    pub async fn check_integrity(&self) -> Result<String, DatabaseError> {
        let row = sqlx::query("PRAGMA integrity_check")
            .fetch_one(&self.pool)
            .await?;

        Ok(row.get::<String, _>("integrity_check"))
    }

    /// Close the database connection
    pub async fn close(self) {
        self.pool.close().await;
    }

    /// Begin a new SQLite transaction for atomic multi-statement operations.
    pub async fn begin_transaction(&self) -> Result<DbTransaction<'_>, DatabaseError> {
        let tx = self.pool.begin().await?;
        Ok(DbTransaction { tx })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{InstalledPackage, PackageFile, PackageStatus};
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_DB_COUNTER: AtomicU64 = AtomicU64::new(0);

    async fn open_test_db() -> Database {
        let id = TEST_DB_COUNTER.fetch_add(1, Ordering::SeqCst);
        let tmp = std::env::temp_dir().join(format!(
            "grel-test-db-{}-{}",
            std::process::id(),
            id
        ));
        let _ = std::fs::remove_file(&tmp);
        Database::init(&tmp).await.expect("open test db")
    }

    #[tokio::test]
    async fn package_files_round_trip() {
        let db = open_test_db().await;

        let mut pkg = InstalledPackage::new("github".into(), "owner".into(), "repo".into());
        pkg.version = "1.0.0".into();
        pkg.asset_filename = "test.tar.gz".into();
        pkg.install_path = "/tmp/test".into();
        pkg.status = PackageStatus::Active;
        db.upsert_package(&pkg).await.unwrap();

        let fetched = db.get_package("github", "owner", "repo").await.unwrap().unwrap();
        let pkg_id = fetched.id.unwrap();

        let files = vec![
            PackageFile::new(pkg_id, "/tmp/test/bin/rg".into(), "binary".into()),
            PackageFile::new(pkg_id, "/tmp/test/README.md".into(), "doc".into()),
        ];
        db.set_package_files(pkg_id, &files).await.unwrap();

        let stored = db.get_package_files(pkg_id).await.unwrap();
        assert_eq!(stored.len(), 2);
        assert!(stored.iter().any(|f| f.file_path == "/tmp/test/bin/rg" && f.file_type == "binary"));
        assert!(stored.iter().any(|f| f.file_path == "/tmp/test/README.md" && f.file_type == "doc"));

        db.close().await;
    }

    #[tokio::test]
    async fn package_files_replaced_on_update() {
        let db = open_test_db().await;

        let mut pkg = InstalledPackage::new("github".into(), "a".into(), "b".into());
        pkg.version = "1.0.0".into();
        pkg.asset_filename = "test.zip".into();
        pkg.install_path = "/tmp/ab".into();
        pkg.status = PackageStatus::Active;
        db.upsert_package(&pkg).await.unwrap();

        let fetched = db.get_package("github", "a", "b").await.unwrap().unwrap();
        let pkg_id = fetched.id.unwrap();

        db.set_package_files(pkg_id, &[PackageFile::new(pkg_id, "/tmp/ab/old".into(), "data".into())])
            .await
            .unwrap();
        db.set_package_files(pkg_id, &[PackageFile::new(pkg_id, "/tmp/ab/new".into(), "data".into())])
            .await
            .unwrap();

        let stored = db.get_package_files(pkg_id).await.unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].file_path, "/tmp/ab/new");

        db.close().await;
    }

    #[tokio::test]
    async fn search_package_files_finds_matches() {
        let db = open_test_db().await;

        let mut pkg = InstalledPackage::new("github".into(), "owner".into(), "repo".into());
        pkg.version = "1.0.0".into();
        pkg.asset_filename = "test.tar.gz".into();
        pkg.install_path = "/tmp/test".into();
        pkg.status = PackageStatus::Active;
        db.upsert_package(&pkg).await.unwrap();

        let fetched = db.get_package("github", "owner", "repo").await.unwrap().unwrap();
        let pkg_id = fetched.id.unwrap();

        db.set_package_files(
            pkg_id,
            &[PackageFile::new(pkg_id, "/tmp/test/bin/rg".into(), "binary".into())],
        )
        .await
        .unwrap();

        let results = db.search_package_files("rg").await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].1.file_path, "/tmp/test/bin/rg");

        let empty = db.search_package_files("nonexistent").await.unwrap();
        assert!(empty.is_empty());

        db.close().await;
    }

    #[tokio::test]
    async fn find_package_by_file_path_exact_match() {
        let db = open_test_db().await;

        let mut pkg = InstalledPackage::new("github".into(), "o".into(), "r".into());
        pkg.version = "1.0.0".into();
        pkg.asset_filename = "test.zip".into();
        pkg.install_path = "/tmp/or".into();
        pkg.status = PackageStatus::Active;
        db.upsert_package(&pkg).await.unwrap();

        let fetched = db.get_package("github", "o", "r").await.unwrap().unwrap();
        let pkg_id = fetched.id.unwrap();

        db.set_package_files(
            pkg_id,
            &[PackageFile::new(pkg_id, "/tmp/or/file.txt".into(), "data".into())],
        )
        .await
        .unwrap();

        let found = db.find_package_by_file_path("/tmp/or/file.txt").await.unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].package_ref(), "github/o/r");

        db.close().await;
    }

    #[tokio::test]
    async fn find_conflicting_files_detects_other_packages() {
        let db = open_test_db().await;

        let mut pkg1 = InstalledPackage::new("github".into(), "a".into(), "b".into());
        pkg1.version = "1.0.0".into();
        pkg1.asset_filename = "a.zip".into();
        pkg1.install_path = "/tmp/ab".into();
        pkg1.status = PackageStatus::Active;
        db.upsert_package(&pkg1).await.unwrap();

        let mut pkg2 = InstalledPackage::new("github".into(), "c".into(), "d".into());
        pkg2.version = "1.0.0".into();
        pkg2.asset_filename = "c.zip".into();
        pkg2.install_path = "/tmp/cd".into();
        pkg2.status = PackageStatus::Active;
        db.upsert_package(&pkg2).await.unwrap();

        let id1 = db.get_package("github", "a", "b").await.unwrap().unwrap().id.unwrap();
        let id2 = db.get_package("github", "c", "d").await.unwrap().unwrap().id.unwrap();

        db.set_package_files(
            id1,
            &[PackageFile::new(id1, "/shared/lib.so".into(), "data".into())],
        )
        .await
        .unwrap();
        db.set_package_files(
            id2,
            &[PackageFile::new(id2, "/shared/lib.so".into(), "data".into())],
        )
        .await
        .unwrap();

        let conflicts = db
            .find_conflicting_files(&["/shared/lib.so".into()])
            .await
            .unwrap();
        assert_eq!(conflicts.len(), 2);

        db.close().await;
    }

    #[tokio::test]
    async fn remove_package_deletes_file_records() {
        let db = open_test_db().await;

        let mut pkg = InstalledPackage::new("github".into(), "x".into(), "y".into());
        pkg.version = "1.0.0".into();
        pkg.asset_filename = "x.zip".into();
        pkg.install_path = "/tmp/xy".into();
        pkg.status = PackageStatus::Active;
        db.upsert_package(&pkg).await.unwrap();

        let fetched = db.get_package("github", "x", "y").await.unwrap().unwrap();
        let pkg_id = fetched.id.unwrap();

        db.set_package_files(
            pkg_id,
            &[PackageFile::new(pkg_id, "/tmp/xy/file".into(), "data".into())],
        )
        .await
        .unwrap();

        db.remove_package(pkg_id).await.unwrap();

        let stored = db.get_package_files(pkg_id).await.unwrap();
        assert!(stored.is_empty());

        db.close().await;
    }

    #[tokio::test]
    async fn find_package_by_filename_matches_basename() {
        let db = open_test_db().await;

        let mut pkg = InstalledPackage::new("github".into(), "fn".into(), "test".into());
        pkg.version = "1.0.0".into();
        pkg.asset_filename = "fn.tar.gz".into();
        pkg.install_path = "/tmp/fn".into();
        pkg.status = PackageStatus::Active;
        db.upsert_package(&pkg).await.unwrap();

        let fetched = db.get_package("github", "fn", "test").await.unwrap().unwrap();
        let pkg_id = fetched.id.unwrap();

        db.set_package_files(
            pkg_id,
            &[PackageFile::new(pkg_id, "/tmp/fn/bin/mytool".into(), "binary".into())],
        )
        .await
        .unwrap();

        let found = db.find_package_by_filename("mytool").await.unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].repo, "test");

        let empty = db.find_package_by_filename("nothere").await.unwrap();
        assert!(empty.is_empty());

        db.close().await;
    }

    #[tokio::test]
    async fn clean_etag_cache_removes_stale_entries() {
        let db = open_test_db().await;

        // Insert a stale entry (last_modified > 30 days ago)
        let stale_ts: i64 = chrono::Utc::now().timestamp() - 86400 * 31;
        sqlx::query(
            "INSERT OR REPLACE INTO etag_cache (url, etag, last_modified) VALUES (?, ?, ?)",
        )
        .bind("https://example.com/stale")
        .bind("etag-stale")
        .bind(stale_ts)
        .execute(&db.pool)
        .await
        .unwrap();

        // Insert a fresh entry
        db.store_etag("https://example.com/fresh", "etag-fresh")
            .await
            .unwrap();

        let removed = db.clean_etag_cache().await.unwrap();
        assert_eq!(removed, 1);

        // Fresh entry must still be reachable
        let still_there = db.get_etag("https://example.com/fresh").await.unwrap();
        assert_eq!(still_there.as_deref(), Some("etag-fresh"));

        db.close().await;
    }

    #[tokio::test]
    async fn clean_dns_cache_removes_expired_entries() {
        let db = open_test_db().await;

        let now = chrono::Utc::now().timestamp();

        // Insert an expired DNS entry
        sqlx::query(
            "INSERT OR REPLACE INTO dns_cache (hostname, ip_address, rtt_ms, expires_at) VALUES (?, ?, ?, ?)",
        )
        .bind("expired.example.com")
        .bind("1.2.3.4")
        .bind(10i64)
        .bind(now - 1)
        .execute(&db.pool)
        .await
        .unwrap();

        // Insert a valid DNS entry
        sqlx::query(
            "INSERT OR REPLACE INTO dns_cache (hostname, ip_address, rtt_ms, expires_at) VALUES (?, ?, ?, ?)",
        )
        .bind("valid.example.com")
        .bind("5.6.7.8")
        .bind(10i64)
        .bind(now + 300)
        .execute(&db.pool)
        .await
        .unwrap();

        let removed = db.clean_dns_cache().await.unwrap();
        assert_eq!(removed, 1);

        // Valid entry must remain
        let row = sqlx::query("SELECT ip_address FROM dns_cache WHERE hostname = ?")
            .bind("valid.example.com")
            .fetch_optional(&db.pool)
            .await
            .unwrap();
        assert!(row.is_some());

        db.close().await;
    }

    #[tokio::test]
    async fn get_dns_cache_skips_expired_entries() {
        let db = open_test_db().await;
        let now = chrono::Utc::now().timestamp();

        // Insert expired and valid entries
        sqlx::query(
            "INSERT OR REPLACE INTO dns_cache (hostname, ip_address, rtt_ms, expires_at) VALUES (?, ?, ?, ?)",
        )
        .bind("old.example.com")
        .bind("1.2.3.4")
        .bind(10i64)
        .bind(now - 1)
        .execute(&db.pool)
        .await
        .unwrap();

        sqlx::query(
            "INSERT OR REPLACE INTO dns_cache (hostname, ip_address, rtt_ms, expires_at) VALUES (?, ?, ?, ?)",
        )
        .bind("new.example.com")
        .bind("5.6.7.8")
        .bind(20i64)
        .bind(now + 300)
        .execute(&db.pool)
        .await
        .unwrap();

        let entries = db.get_dns_cache().await.unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].0, "new.example.com");
        assert_eq!(entries[0].1, "5.6.7.8");
        assert_eq!(entries[0].2, 20);

        db.close().await;
    }

    #[tokio::test]
    async fn store_dns_entry_round_trip() {
        let db = open_test_db().await;
        let now = chrono::Utc::now().timestamp();

        db.store_dns_entry("test.example.com", "9.8.7.6", 42, now + 300)
            .await
            .unwrap();

        let entries = db.get_dns_cache().await.unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].0, "test.example.com");
        assert_eq!(entries[0].1, "9.8.7.6");
        assert_eq!(entries[0].2, 42);

        db.close().await;
    }

    #[tokio::test]
    async fn release_cache_round_trip() {
        let db = open_test_db().await;

        db.store_release_cache("github", "owner", "repo", "v1.0.0", r#"{"tag":"v1.0.0"}"#, 300)
            .await
            .unwrap();

        let cached = db
            .get_release_cache("github", "owner", "repo", "v1.0.0")
            .await
            .unwrap();
        assert_eq!(cached, Some(r#"{"tag":"v1.0.0"}"#.to_string()));

        // Expired entry should return None
        db.store_release_cache("github", "owner", "repo", "old", "{}", -1)
            .await
            .unwrap();
        let expired = db.get_release_cache("github", "owner", "repo", "old").await.unwrap();
        assert_eq!(expired, None);

        db.close().await;
    }

    #[tokio::test]
    async fn manifest_cache_round_trip() {
        let db = open_test_db().await;

        db.store_manifest_cache("owner", "repo", "main", ".grel.toml", "[package]", Some("etag1"), 300)
            .await
            .unwrap();

        let cached = db
            .get_manifest_cache("owner", "repo", "main", ".grel.toml")
            .await
            .unwrap();
        assert_eq!(cached, Some(("[package]".to_string(), Some("etag1".to_string()))));

        db.close().await;
    }

    #[tokio::test]
    async fn rate_limit_cache_round_trip() {
        let db = open_test_db().await;

        db.store_rate_limit("github", 42, 1234567890)
            .await
            .unwrap();

        let cached = db.get_rate_limit("github").await.unwrap();
        assert!(cached.is_some());
        let (remaining, reset_at, _last_checked) = cached.unwrap();
        assert_eq!(remaining, 42);
        assert_eq!(reset_at, 1234567890);

        db.close().await;
    }

    #[tokio::test]
    async fn check_integrity_returns_ok_on_fresh_db() {
        let db = open_test_db().await;
        let result = db.check_integrity().await.unwrap();
        assert_eq!(result, "ok");
        db.close().await;
    }

    #[tokio::test]
    async fn test_transaction_commit_persists_data() {
        let db = open_test_db().await;

        let mut tx = db.begin_transaction().await.unwrap();

        let mut pkg = InstalledPackage::new("github".into(), "tx".into(), "commit".into());
        pkg.version = "1.0.0".into();
        pkg.asset_filename = "test.tar.gz".into();
        pkg.install_path = "/tmp/tx".into();
        pkg.status = PackageStatus::Active;

        tx.upsert_package(&pkg).await.unwrap();
        tx.commit().await.unwrap();

        let fetched = db.get_package("github", "tx", "commit").await.unwrap();
        assert!(fetched.is_some());
        assert_eq!(fetched.unwrap().version, "1.0.0");

        db.close().await;
    }

    #[tokio::test]
    async fn test_transaction_rollback_discards_data() {
        let db = open_test_db().await;

        let mut tx = db.begin_transaction().await.unwrap();

        let mut pkg = InstalledPackage::new("github".into(), "tx".into(), "rollback".into());
        pkg.version = "2.0.0".into();
        pkg.asset_filename = "test.tar.gz".into();
        pkg.install_path = "/tmp/tx".into();
        pkg.status = PackageStatus::Active;

        tx.upsert_package(&pkg).await.unwrap();
        tx.rollback().await.unwrap();

        let fetched = db.get_package("github", "tx", "rollback").await.unwrap();
        assert!(fetched.is_none());

        db.close().await;
    }

    #[tokio::test]
    async fn test_transaction_set_package_files_atomic() {
        let db = open_test_db().await;

        let mut pkg = InstalledPackage::new("github".into(), "tx".into(), "files".into());
        pkg.version = "1.0.0".into();
        pkg.asset_filename = "test.tar.gz".into();
        pkg.install_path = "/tmp/tx".into();
        pkg.status = PackageStatus::Active;

        // Insert package outside transaction first
        db.upsert_package(&pkg).await.unwrap();
        let pkg_id = db.get_package("github", "tx", "files").await.unwrap().unwrap().id.unwrap();

        // Use transaction to set files
        let mut tx = db.begin_transaction().await.unwrap();
        let files = vec![
            PackageFile::new(pkg_id, "/tmp/tx/bin/rg".into(), "binary".into()),
        ];
        tx.set_package_files(pkg_id, &files).await.unwrap();
        tx.commit().await.unwrap();

        let stored = db.get_package_files(pkg_id).await.unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].file_path, "/tmp/tx/bin/rg");

        db.close().await;
    }
}

/// A database transaction wrapper for atomic multi-statement operations.
///
/// Created via [`Database::begin_transaction`]. Must be committed or rolled back.
pub struct DbTransaction<'a> {
    tx: sqlx::Transaction<'a, sqlx::Sqlite>,
}

impl<'a> DbTransaction<'a> {
    /// Insert or update a package record inside the transaction.
    pub async fn upsert_package(
        &mut self,
        pkg: &InstalledPackage,
    ) -> Result<(), DatabaseError> {
        sqlx::query(
            r#"
            INSERT INTO installed
                (forge, owner, repo, version, asset_filename, checksum, install_path,
                 installed_binaries, is_managed, status, orphaned_at, last_checked, manifest_source, is_explicit)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
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
                manifest_source = excluded.manifest_source,
                is_explicit = excluded.is_explicit
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
        .bind(pkg.is_explicit)
        .execute(&mut *self.tx)
        .await?;

        Ok(())
    }

    /// Get a package by reference inside the transaction.
    pub async fn get_package(
        &mut self,
        forge: &str,
        owner: &str,
        repo: &str,
    ) -> Result<Option<InstalledPackage>, DatabaseError> {
        let row = sqlx::query(
            r#"
            SELECT id, forge, owner, repo, version, asset_filename, checksum,
                   install_path, installed_binaries, is_managed, status, orphaned_at, last_checked, installed_at, manifest_source, is_explicit
            FROM installed
            WHERE forge = ? AND owner = ? AND repo = ?
            "#,
        )
        .bind(forge)
        .bind(owner)
        .bind(repo)
        .fetch_optional(&mut *self.tx)
        .await?;

        Ok(row.map(|r| Database::row_to_package(&r)))
    }

    /// Replace file records for a package inside the transaction.
    pub async fn set_package_files(
        &mut self,
        package_id: i64,
        files: &[crate::models::PackageFile],
    ) -> Result<(), DatabaseError> {
        sqlx::query("DELETE FROM package_files WHERE package_id = ?")
            .bind(package_id)
            .execute(&mut *self.tx)
            .await?;

        for file in files {
            sqlx::query(
                r#"
                INSERT INTO package_files (package_id, file_path, file_type)
                VALUES (?, ?, ?)
                "#,
            )
            .bind(package_id)
            .bind(&file.file_path)
            .bind(&file.file_type)
            .execute(&mut *self.tx)
            .await?;
        }

        Ok(())
    }

    /// Replace dependency records for a package inside the transaction.
    pub async fn set_dependencies(
        &mut self,
        package_id: i64,
        deps: &[crate::models::Dependency],
    ) -> Result<(), DatabaseError> {
        sqlx::query("DELETE FROM dependencies WHERE package_id = ?")
            .bind(package_id)
            .execute(&mut *self.tx)
            .await?;

        for dep in deps {
            sqlx::query(
                r#"
                INSERT INTO dependencies (package_id, dep_target, dep_type)
                VALUES (?, ?, ?)
                "#,
            )
            .bind(package_id)
            .bind(&dep.dep_target)
            .bind(dep.dep_type.to_string())
            .execute(&mut *self.tx)
            .await?;
        }

        Ok(())
    }

    /// Commit the transaction.
    pub async fn commit(self) -> Result<(), DatabaseError> {
        self.tx.commit().await.map_err(DatabaseError::from)
    }

    /// Roll back the transaction.
    pub async fn rollback(self) -> Result<(), DatabaseError> {
        self.tx.rollback().await.map_err(DatabaseError::from)
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
