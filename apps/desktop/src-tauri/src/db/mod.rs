//! SQLite connection and migration layer.

mod models;
pub mod welding_repository;

pub use models::{new_id, utc_now, BoxRecord, PartRecord};
use rusqlite::{Connection, Result, Transaction, TransactionBehavior};
use std::path::{Path, PathBuf};

const INITIAL_MIGRATION: &str = include_str!("../../migrations/0001_initial.sql");
const WELDING_MOVEMENT_METADATA_MIGRATION: &str =
    include_str!("../../migrations/0002_welding_movement_metadata.sql");
const MOVEMENT_AUDIT_AND_ACTIVE_SESSION_MIGRATION: &str =
    include_str!("../../migrations/0003_movement_audit_and_active_session.sql");
const LCSC_CACHE_MIGRATION: &str = include_str!("../../migrations/0004_lcsc_cache.sql");
const ALLOW_OVERCONSUMPTION_MIGRATION: &str =
    include_str!("../../migrations/0005_allow_overconsumption.sql");
const NORMALIZE_LCSC_MIGRATION: &str =
    include_str!("../../migrations/0006_normalize_lcsc_codes.sql");
const INTEGER_BOX_IDS_MIGRATION: &str = include_str!("../../migrations/0007_integer_box_ids.sql");
const CONFIRMED_DESIGNATORS_MIGRATION: &str =
    include_str!("../../migrations/0008_welding_confirmed_designators.sql");

const SOFT_DELETE_PARTS_MIGRATION: &str =
    include_str!("../../migrations/0009_soft_delete_parts.sql");

const BOM_IMPORT_SNAPSHOT_MIGRATION: &str =
    include_str!("../../migrations/0010_bom_import_snapshot.sql");

const PROJECTS_MIGRATION: &str = include_str!("../../migrations/0011_projects.sql");

const PROJECT_SOURCES_MIGRATION: &str = include_str!("../../migrations/0012_project_sources.sql");

/// The complete, ordered migration set used by every database open. Keeping it
/// in one place lets tests compare it against the migration files on disk.
pub fn migrations() -> Vec<Migration<'static>> {
    vec![
        Migration {
            version: 1,
            sql: INITIAL_MIGRATION,
        },
        Migration {
            version: 2,
            sql: WELDING_MOVEMENT_METADATA_MIGRATION,
        },
        Migration {
            version: 3,
            sql: MOVEMENT_AUDIT_AND_ACTIVE_SESSION_MIGRATION,
        },
        Migration {
            version: 4,
            sql: LCSC_CACHE_MIGRATION,
        },
        Migration {
            version: 5,
            sql: ALLOW_OVERCONSUMPTION_MIGRATION,
        },
        Migration {
            version: 6,
            sql: NORMALIZE_LCSC_MIGRATION,
        },
        Migration {
            version: 7,
            sql: INTEGER_BOX_IDS_MIGRATION,
        },
        Migration {
            version: 8,
            sql: CONFIRMED_DESIGNATORS_MIGRATION,
        },
        Migration {
            version: 9,
            sql: SOFT_DELETE_PARTS_MIGRATION,
        },
        Migration {
            version: 10,
            sql: BOM_IMPORT_SNAPSHOT_MIGRATION,
        },
        Migration {
            version: 11,
            sql: PROJECTS_MIGRATION,
        },
        Migration {
            version: 12,
            sql: PROJECT_SOURCES_MIGRATION,
        },
    ]
}

/// A versioned SQL migration. Migrations are applied in one exclusive transaction.
#[derive(Debug, Clone, Copy)]
pub struct Migration<'a> {
    pub version: i64,
    pub sql: &'a str,
}

/// An opened PartNest database connection.
pub struct Database {
    connection: Connection,
    path: PathBuf,
}

impl Database {
    /// Open a database file, configure SQLite, and apply all pending migrations.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let connection = Connection::open(&path)?;
        let mut database = Self { connection, path };
        database.configure()?;
        database.apply_migrations(&migrations())?;
        Ok(database)
    }

    /// Open `partnest.db` below an application data directory, creating it when needed.
    pub fn open_app_data_dir(app_data_dir: impl AsRef<Path>) -> Result<Self> {
        std::fs::create_dir_all(app_data_dir.as_ref())
            .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
        Self::open(app_data_dir.as_ref().join("partnest.db"))
    }

    /// Open a database with an explicit migration list for deterministic migration tests.
    pub fn open_with_migrations(
        path: impl AsRef<Path>,
        migrations: &[Migration<'_>],
    ) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let connection = Connection::open(&path)?;
        let mut database = Self { connection, path };
        database.configure()?;
        database.apply_migrations(migrations)?;
        Ok(database)
    }

    /// Borrow the underlying connection for commands and read-only queries.
    pub fn connection(&self) -> &Connection {
        &self.connection
    }

    /// Return the database file path used by this connection.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Replace the connection and path together when recovery moves the
    /// database to a preserved path.
    pub(crate) fn replace_database(&mut self, database: Database) -> Database {
        std::mem::replace(self, database)
    }

    /// Start a transaction for an operation spanning multiple statements.
    pub fn transaction(&self) -> Result<Transaction<'_>> {
        self.connection.unchecked_transaction()
    }

    fn configure(&self) -> Result<()> {
        self.connection.execute_batch(
            "PRAGMA foreign_keys = ON;
             PRAGMA journal_mode = WAL;
             PRAGMA busy_timeout = 5000;",
        )
    }

    fn apply_migrations(&mut self, migrations: &[Migration<'_>]) -> Result<()> {
        validate_migration_list(migrations)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Exclusive)?;
        transaction.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_migrations (
                version INTEGER PRIMARY KEY,
                applied_at TEXT NOT NULL
            );",
        )?;

        let applied_versions = transaction
            .prepare("SELECT version FROM schema_migrations ORDER BY version")?
            .query_map([], |row| row.get::<_, i64>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let known_versions = migrations
            .iter()
            .map(|migration| migration.version)
            .collect::<std::collections::BTreeSet<_>>();
        for version in &applied_versions {
            if !known_versions.contains(version) {
                return Err(migration_error(format!(
                    "database schema version {version} is newer or unsupported"
                )));
            }
        }
        if let Some(max_applied) = applied_versions.iter().max() {
            for version in 1..=*max_applied {
                if !applied_versions.contains(&version) {
                    return Err(migration_error(format!(
                        "database schema migration gap at version {version}"
                    )));
                }
            }
        }

        for migration in migrations {
            let applied: bool = transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE version = ?1)",
                [migration.version],
                |row| row.get(0),
            )?;
            if !applied {
                transaction.execute_batch(migration.sql)?;
                transaction.execute(
                    "INSERT INTO schema_migrations (version, applied_at) VALUES (?1, ?2)",
                    (migration.version, models::utc_now()),
                )?;
            }
        }
        transaction.commit()
    }
}

#[derive(Debug)]
struct MigrationValidationError(String);

impl std::fmt::Display for MigrationValidationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for MigrationValidationError {}

fn migration_error(message: String) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(Box::new(MigrationValidationError(message)))
}

fn validate_migration_list(migrations: &[Migration<'_>]) -> Result<()> {
    if migrations.is_empty() || migrations[0].version != 1 {
        return Err(migration_error(
            "migration set must start at version 1".into(),
        ));
    }
    for (expected, migration) in (1_i64..).zip(migrations) {
        if migration.version != expected {
            return Err(migration_error(format!(
                "migration set has a gap at version {expected}"
            )));
        }
    }
    Ok(())
}
