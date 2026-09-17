//! Consistent SQLite backups and failure-safe destructive restore support.

use crate::db::{new_id, Database};
use rusqlite::{Connection, OpenFlags, MAIN_DB};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tempfile::Builder;

pub const CURRENT_SCHEMA_VERSION: i64 = 12;
pub const MAX_BACKUPS: usize = 10;
pub const STARTUP_BACKUP_AGE: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Debug)]
pub enum BackupError {
    Io(std::io::Error),
    Sqlite(rusqlite::Error),
    Invalid(String),
    SchemaTooNew {
        found: i64,
        current: i64,
    },
    FatalRecovery {
        primary: String,
        recovery: String,
        recovery_path: String,
    },
}

impl fmt::Display for BackupError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "备份文件操作失败: {error}"),
            Self::Sqlite(error) => write!(formatter, "SQLite 备份失败: {error}"),
            Self::Invalid(message) => formatter.write_str(message),
            Self::SchemaTooNew { found, current } => write!(
                formatter,
                "备份数据库版本 {found} 高于当前应用支持的版本 {current}"
            ),
            Self::FatalRecovery {
                primary,
                recovery,
                recovery_path,
            } => write!(
                formatter,
                "数据库恢复失败，恢复前的数据库快照已保留在 {recovery_path}，需要人工处理。原始错误: {primary}；回滚错误: {recovery}"
            ),
        }
    }
}

impl std::error::Error for BackupError {}

impl From<std::io::Error> for BackupError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<rusqlite::Error> for BackupError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupMetadata {
    pub schema_version: i64,
}

pub fn backup_dir(app_data_dir: impl AsRef<Path>) -> PathBuf {
    app_data_dir.as_ref().join("backups")
}

fn normalized_sql(sql: &str) -> String {
    sql.to_ascii_lowercase()
        .chars()
        .filter(|character| !character.is_ascii_whitespace())
        .collect()
}

fn table_columns(
    connection: &Connection,
    table: &str,
) -> Result<std::collections::HashSet<String>, BackupError> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info('{table}')"))?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<std::collections::HashSet<_>>>()?;
    Ok(columns)
}

fn require_table(
    connection: &Connection,
    table: &str,
    columns: &[&str],
) -> Result<String, BackupError> {
    let sql: String = connection
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = ?1",
            [table],
            |row| row.get(0),
        )
        .map_err(|_| BackupError::Invalid(format!("备份缺少表 {table}")))?;
    let actual = table_columns(connection, table)?;
    for column in columns {
        if !actual.contains(*column) {
            return Err(BackupError::Invalid(format!(
                "备份表 {table} 缺少字段 {column}"
            )));
        }
    }
    Ok(normalized_sql(&sql))
}

fn require_index(connection: &Connection, table: &str, expected: &str) -> Result<(), BackupError> {
    let mut statement = connection.prepare(&format!("PRAGMA index_list('{table}')"))?;
    let names = statement
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if names.iter().any(|name| name == expected) {
        Ok(())
    } else {
        Err(BackupError::Invalid(format!(
            "备份表 {table} 缺少索引 {expected}"
        )))
    }
}

fn validate_schema(connection: &Connection) -> Result<i64, BackupError> {
    let migrations_sql =
        require_table(connection, "schema_migrations", &["version", "applied_at"])?;
    if !migrations_sql.contains("primarykey") {
        return Err(BackupError::Invalid(
            "schema_migrations.version 必须是主键".into(),
        ));
    }
    let versions = connection
        .prepare("SELECT version FROM schema_migrations ORDER BY version")?
        .query_map([], |row| row.get::<_, i64>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if versions.is_empty() {
        return Err(BackupError::Invalid("schema_migrations 不能为空".into()));
    }
    let schema_version = *versions.last().unwrap();
    if schema_version > CURRENT_SCHEMA_VERSION {
        return Err(BackupError::SchemaTooNew {
            found: schema_version,
            current: CURRENT_SCHEMA_VERSION,
        });
    }
    let expected_versions = (1..=schema_version).collect::<Vec<_>>();
    if versions != expected_versions {
        return Err(BackupError::Invalid(
            "schema_migrations 必须包含连续且已识别的迁移版本".into(),
        ));
    }
    if schema_version != CURRENT_SCHEMA_VERSION {
        return Err(BackupError::Invalid(
            "备份 schema 不是当前应用可直接使用的完整版本".into(),
        ));
    }

    let boxes_sql = require_table(
        connection,
        "boxes",
        &["id", "name", "rows", "cols", "created_at", "updated_at"],
    )?;
    if !boxes_sql.contains("integerprimarykeyautoincrement")
        || !boxes_sql.contains("check(rows>0)")
        || !boxes_sql.contains("check(cols>0)")
    {
        return Err(BackupError::Invalid("boxes 约束不完整".into()));
    }
    let parts_sql = require_table(
        connection,
        "parts",
        &[
            "id",
            "name",
            "category",
            "package",
            "manufacturer",
            "mpn",
            "lcsc_code",
            "deleted_at",
            "quantity",
            "box_id",
            "slot",
            "note",
            "version",
            "created_at",
            "updated_at",
        ],
    )?;
    if !parts_sql.contains("primarykey")
        || !parts_sql.contains("check(quantity>=0)")
        || !parts_sql.contains("unique(box_id,slot)")
        || !parts_sql.contains("foreignkey(box_id)references")
        || !parts_sql.contains("box_idisnull")
        || !parts_sql.contains("slotisnull")
    {
        return Err(BackupError::Invalid("parts 约束不完整".into()));
    }
    require_index(connection, "parts", "parts_lcsc_code_unique")?;
    let lcsc_index: String = connection.query_row(
        "SELECT sql FROM sqlite_master WHERE type = 'index' AND name = 'parts_lcsc_code_unique'",
        [],
        |row| row.get(0),
    )?;
    let lcsc_index = normalized_sql(&lcsc_index);
    if !lcsc_index.contains("createuniqueindex")
        || !lcsc_index.contains("onparts(lcsc_codecollatenocase)")
        || !lcsc_index.contains("wheredeleted_atisnullandlcsc_codeisnotnullandtrim(lcsc_code)<>''")
    {
        return Err(BackupError::Invalid(
            "parts 活动器件 LCSC 唯一索引不完整".into(),
        ));
    }
    require_table(
        connection,
        "lcsc_cache",
        &[
            "lcsc_code",
            "name",
            "category",
            "package",
            "manufacturer",
            "mpn",
            "fetched_at",
        ],
    )?;

    require_table(
        connection,
        "projects",
        &[
            "id",
            "name",
            "original_name",
            "sha256",
            "cache_name",
            "created_at",
            "kind",
            "has_table",
            "normalized_json",
        ],
    )?;
    require_table(
        connection,
        "welding_sessions",
        &["id", "project_id", "status", "created_at", "updated_at"],
    )?;
    let progress_sql = require_table(
        connection,
        "welding_progress",
        &[
            "id",
            "session_id",
            "component_key",
            "side",
            "part_id",
            "required_quantity",
            "taken_quantity",
            "confirmed_designators",
            "updated_at",
        ],
    )?;
    require_index(
        connection,
        "welding_sessions",
        "welding_sessions_one_active",
    )?;
    if !progress_sql.contains("check(sidein('top','bottom'))")
        || !progress_sql.contains("check(required_quantity>=0)")
        || !progress_sql.contains("check(taken_quantity>=0)")
        || !progress_sql.contains("unique(session_id,component_key,side)")
    {
        return Err(BackupError::Invalid("welding_progress 约束不完整".into()));
    }
    let movements_sql = require_table(
        connection,
        "inventory_movements",
        &[
            "id",
            "part_id",
            "session_id",
            "movement_type",
            "quantity",
            "reason",
            "reverses_movement_id",
            "created_at",
            "component_key",
            "side",
            "before_quantity",
            "after_quantity",
            "movement_sequence",
            "bom_quantity",
            "confirmation_designators",
        ],
    )?;
    if !movements_sql.contains("check(movement_typein('in','consume','adjust','reverse'))")
        || !movements_sql.contains("check(quantity<>0)")
    {
        return Err(BackupError::Invalid(
            "inventory_movements 约束不完整".into(),
        ));
    }
    for index in [
        "inventory_movements_part_id",
        "inventory_movements_session_id",
        "inventory_movements_welding_scope",
        "inventory_movements_sequence_order",
    ] {
        require_index(connection, "inventory_movements", index)?;
    }
    Ok(schema_version)
}

/// Validate a backup without opening it for writing or applying migrations.
pub fn validate_backup(path: impl AsRef<Path>) -> Result<BackupMetadata, BackupError> {
    let path = path.as_ref();
    if !path.is_file() {
        return Err(BackupError::Invalid("备份文件不存在".into()));
    }
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )?;
    let integrity: String = connection.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    if integrity != "ok" {
        return Err(BackupError::Invalid(format!(
            "备份完整性检查失败: {integrity}"
        )));
    }
    let schema_version = validate_schema(&connection)?;
    Ok(BackupMetadata { schema_version })
}

fn backup_filename() -> String {
    let timestamp = chrono::Utc::now().format("%Y%m%dT%H%M%S%.3fZ");
    format!("partnest-{timestamp}-{}.db", new_id())
}

fn is_managed_backup(path: &Path) -> bool {
    path.is_file()
        && path.extension().is_some_and(|extension| extension == "db")
        && path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("partnest-") && name.ends_with(".db"))
}

fn modified_time(path: &Path) -> SystemTime {
    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .unwrap_or(UNIX_EPOCH)
}

fn valid_backups(dir: &Path) -> Result<Vec<(PathBuf, SystemTime)>, BackupError> {
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut paths = Vec::new();
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if is_managed_backup(&path) && validate_backup(&path).is_ok() {
            paths.push((path.clone(), modified_time(&path)));
        }
    }
    paths.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| right.0.cmp(&left.0)));
    Ok(paths)
}

fn rotate_backups(dir: &Path) -> Result<(), BackupError> {
    let paths = valid_backups(dir)?;
    for (path, _) in paths.into_iter().skip(MAX_BACKUPS) {
        if let Err(error) = fs::remove_file(&path) {
            eprintln!(
                "PartNest backup rotation skipped {}: {error}",
                path.display()
            );
        }
    }
    Ok(())
}

/// Create a WAL-consistent backup in `backup_directory` and retain ten valid files.
pub fn create_backup(
    database: &Database,
    backup_directory: impl AsRef<Path>,
) -> Result<PathBuf, BackupError> {
    let backup_directory = backup_directory.as_ref();
    fs::create_dir_all(backup_directory)?;
    let temporary = Builder::new()
        .prefix(".partnest-backup-")
        .suffix(".tmp")
        .tempfile_in(backup_directory)?
        .into_temp_path();

    database.connection().backup(MAIN_DB, &temporary, None)?;
    validate_backup(&temporary)?;

    let destination = backup_directory.join(backup_filename());
    fs::rename(&temporary, &destination)?;
    if let Err(error) = rotate_backups(backup_directory) {
        eprintln!("PartNest backup rotation skipped: {error}");
    }
    Ok(destination)
}

fn latest_valid_backup(dir: &Path) -> Result<Option<(PathBuf, SystemTime)>, BackupError> {
    Ok(valid_backups(dir)?.into_iter().next())
}

/// Create a startup backup only when no valid backup exists or the latest one is old.
pub fn maybe_create_startup_backup(
    database: &Database,
    app_data_directory: impl AsRef<Path>,
) -> Result<Option<PathBuf>, BackupError> {
    let directory = backup_dir(app_data_directory);
    let now = SystemTime::now();
    let needs_backup = latest_valid_backup(&directory)?.is_none_or(|(_, modified)| {
        now.duration_since(modified)
            .map_or(true, |age| age >= STARTUP_BACKUP_AGE)
    });
    needs_backup
        .then(|| create_backup(database, directory))
        .transpose()
}

fn copy_database(source: &Path, destination: &Path) -> Result<(), BackupError> {
    let source_connection = Connection::open_with_flags(
        source,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )?;
    source_connection.backup(MAIN_DB, destination, None)?;
    Ok(())
}

fn sidecar(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path
        .file_name()
        .map(|value| value.to_os_string())
        .unwrap_or_default();
    name.push(suffix);
    path.with_file_name(name)
}

fn remove_if_exists(path: &Path) -> Result<bool, BackupError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(BackupError::Io(error)),
    }
}

fn move_if_exists(source: &Path, destination: &Path) -> Result<bool, BackupError> {
    if source.is_file() {
        fs::rename(source, destination)?;
        Ok(true)
    } else if source.exists() {
        Err(BackupError::Invalid(format!(
            "数据库伴随文件不是普通文件: {}",
            source.display()
        )))
    } else {
        Ok(false)
    }
}

fn open_verified_database(path: &Path) -> Result<Database, BackupError> {
    if !path.is_file() {
        return Err(BackupError::Invalid(format!(
            "恢复数据库文件不存在: {}",
            path.display()
        )));
    }
    validate_backup(path)?;
    Database::open(path).map_err(BackupError::Sqlite)
}

fn install_database(database: &mut Database, replacement: Database) {
    let old_database = database.replace_database(replacement);
    drop(old_database);
}

fn cleanup_recovery_snapshot(path: &Path) {
    for sidecar_path in [sidecar(path, "-shm"), sidecar(path, "-wal")] {
        if let Err(error) = remove_if_exists(&sidecar_path) {
            eprintln!(
                "PartNest recovery snapshot cleanup skipped {}: {error}",
                sidecar_path.display()
            );
        }
    }
    if let Err(error) = remove_if_exists(path) {
        eprintln!(
            "PartNest recovery snapshot cleanup skipped {}: {error}",
            path.display()
        );
    }
}

fn preserve_recovery_snapshot(
    database: &mut Database,
    parent: &Path,
) -> Result<PathBuf, BackupError> {
    let recovery_path = parent.join(format!(".partnest-recovery-{}.db", new_id()));
    let result = (|| {
        database
            .connection()
            .backup(MAIN_DB, &recovery_path, None)?;
        validate_backup(&recovery_path)?;
        open_verified_database(&recovery_path)
    })();
    match result {
        Ok(recovery_database) => {
            install_database(database, recovery_database);
            Ok(recovery_path)
        }
        Err(error) => {
            cleanup_recovery_snapshot(&recovery_path);
            Err(error)
        }
    }
}

struct RestorePaths {
    live_path: PathBuf,
    previous_path: PathBuf,
    live_wal: PathBuf,
    previous_wal: PathBuf,
    live_shm: PathBuf,
    previous_shm: PathBuf,
    recovery_path: PathBuf,
}

struct RestoreState {
    moved_live: bool,
    moved_wal: bool,
    moved_shm: bool,
    replacement_installed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RestoreInjection {
    None,
    AfterSwap,
    AfterOpen,
    AfterOpenRollbackFailure,
    AfterOpenLiveReopenFailure,
}

impl RestorePaths {
    fn rollback(
        &self,
        state: &RestoreState,
        fail_after_replacement_cleanup: bool,
    ) -> Result<(), BackupError> {
        if state.replacement_installed {
            remove_if_exists(&self.live_shm)?;
            remove_if_exists(&self.live_wal)?;
            remove_if_exists(&self.live_path)?;
            if fail_after_replacement_cleanup {
                return Err(BackupError::Invalid("测试注入的回滚操作失败".into()));
            }
        }
        if state.moved_shm {
            fs::rename(&self.previous_shm, &self.live_shm)?;
        }
        if state.moved_wal {
            fs::rename(&self.previous_wal, &self.live_wal)?;
        }
        if state.moved_live {
            fs::rename(&self.previous_path, &self.live_path)?;
        }
        if !self.live_path.is_file() {
            return Err(BackupError::Invalid("回滚后原始数据库文件不存在".into()));
        }
        validate_backup(&self.live_path)?;
        Ok(())
    }
}

#[cfg(unix)]
fn file_identity(path: &Path) -> Result<(u64, u64), BackupError> {
    use std::os::unix::fs::MetadataExt;
    let metadata = fs::metadata(path)?;
    Ok((metadata.dev(), metadata.ino()))
}

#[cfg(windows)]
fn file_identity(path: &Path) -> Result<(u64, u64), BackupError> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };
    let file = fs::File::open(path)?;
    let mut information = unsafe { std::mem::zeroed::<BY_HANDLE_FILE_INFORMATION>() };
    // SAFETY: `file` owns a valid Windows file handle for the duration of this
    // call, and `information` points to writable storage of the expected type.
    let result = unsafe { GetFileInformationByHandle(file.as_raw_handle() as _, &mut information) };
    if result == 0 {
        return Err(BackupError::Io(std::io::Error::last_os_error()));
    }
    let index =
        (u64::from(information.nFileIndexHigh) << 32) | u64::from(information.nFileIndexLow);
    Ok((u64::from(information.dwVolumeSerialNumber), index))
}

#[cfg(not(any(unix, windows)))]
fn file_identity(path: &Path) -> Result<(u64, u64), BackupError> {
    let metadata = fs::metadata(path)?;
    Ok((
        metadata.len(),
        modified_time(path)
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64,
    ))
}

fn fatal_recovery(
    primary: &BackupError,
    recovery: &BackupError,
    recovery_path: &Path,
) -> BackupError {
    BackupError::FatalRecovery {
        primary: primary.to_string(),
        recovery: recovery.to_string(),
        recovery_path: recovery_path.display().to_string(),
    }
}

fn recover_after_failure(
    database: &mut Database,
    paths: &RestorePaths,
    state: &RestoreState,
    primary: BackupError,
    injection: RestoreInjection,
) -> BackupError {
    let rollback = paths.rollback(
        state,
        injection == RestoreInjection::AfterOpenRollbackFailure,
    );
    match rollback {
        Err(recovery) => fatal_recovery(&primary, &recovery, &paths.recovery_path),
        Ok(()) if injection == RestoreInjection::AfterOpenLiveReopenFailure => {
            let recovery = BackupError::Invalid("测试注入的现场数据库重新打开失败".into());
            fatal_recovery(&primary, &recovery, &paths.recovery_path)
        }
        Ok(()) => match open_verified_database(&paths.live_path) {
            Ok(recovered) => {
                install_database(database, recovered);
                cleanup_recovery_snapshot(&paths.recovery_path);
                primary
            }
            Err(recovery) => fatal_recovery(&primary, &recovery, &paths.recovery_path),
        },
    }
}

fn restore_database_file_inner(
    database: &mut Database,
    selected_backup: impl AsRef<Path>,
    injection: RestoreInjection,
) -> Result<(), BackupError> {
    let selected_backup = selected_backup.as_ref();
    let selected_canonical = fs::canonicalize(selected_backup)
        .map_err(|_| BackupError::Invalid("备份文件不存在".into()))?;
    let live_path = database.path().to_path_buf();
    if !live_path.is_file() {
        return Err(BackupError::Invalid("当前数据库文件不存在".into()));
    }
    if file_identity(&live_path)? == file_identity(&selected_canonical)? {
        return Err(BackupError::Invalid(
            "不能从当前数据库文件或其硬链接恢复".into(),
        ));
    }
    validate_backup(&selected_canonical)?;

    let parent = live_path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let temporary = Builder::new()
        .prefix(".partnest-restore-")
        .suffix(".tmp")
        .tempfile_in(parent)?
        .into_temp_path();
    copy_database(&selected_canonical, &temporary)?;
    validate_backup(&temporary)?;

    database
        .connection()
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?;
    let recovery_path = preserve_recovery_snapshot(database, parent)?;

    let previous_path = parent.join(format!(".partnest-previous-{}.db", new_id()));
    let previous_wal = sidecar(&previous_path, "-wal");
    let previous_shm = sidecar(&previous_path, "-shm");
    let live_wal = sidecar(&live_path, "-wal");
    let live_shm = sidecar(&live_path, "-shm");
    let paths = RestorePaths {
        live_path: live_path.clone(),
        previous_path: previous_path.clone(),
        live_wal: live_wal.clone(),
        previous_wal: previous_wal.clone(),
        live_shm: live_shm.clone(),
        previous_shm: previous_shm.clone(),
        recovery_path,
    };
    let mut state = RestoreState {
        moved_live: false,
        moved_wal: false,
        moved_shm: false,
        replacement_installed: false,
    };
    state.moved_live = match move_if_exists(&live_path, &previous_path) {
        Ok(moved) => moved,
        Err(error) => {
            return Err(recover_after_failure(
                database, &paths, &state, error, injection,
            ))
        }
    };
    state.moved_wal = match move_if_exists(&live_wal, &previous_wal) {
        Ok(moved) => moved,
        Err(error) => {
            return Err(recover_after_failure(
                database, &paths, &state, error, injection,
            ))
        }
    };
    state.moved_shm = match move_if_exists(&live_shm, &previous_shm) {
        Ok(moved) => moved,
        Err(error) => {
            return Err(recover_after_failure(
                database, &paths, &state, error, injection,
            ))
        }
    };

    if let Err(error) = fs::rename(&temporary, &live_path) {
        return Err(recover_after_failure(
            database,
            &paths,
            &state,
            BackupError::Io(error),
            injection,
        ));
    }
    state.replacement_installed = true;
    if injection == RestoreInjection::AfterSwap {
        return Err(recover_after_failure(
            database,
            &paths,
            &state,
            BackupError::Invalid("测试注入的替换后打开失败".into()),
            injection,
        ));
    }

    let restored = match Database::open(&live_path) {
        Ok(restored) => restored,
        Err(error) => {
            return Err(recover_after_failure(
                database,
                &paths,
                &state,
                BackupError::Sqlite(error),
                injection,
            ))
        }
    };
    if matches!(
        injection,
        RestoreInjection::AfterOpen
            | RestoreInjection::AfterOpenRollbackFailure
            | RestoreInjection::AfterOpenLiveReopenFailure
    ) {
        drop(restored);
        return Err(recover_after_failure(
            database,
            &paths,
            &state,
            BackupError::Invalid("测试注入的打开后失败".into()),
            injection,
        ));
    }
    if let Err(error) = validate_backup(&live_path) {
        drop(restored);
        return Err(recover_after_failure(
            database, &paths, &state, error, injection,
        ));
    }
    install_database(database, restored);

    if let Err(error) = remove_if_exists(&previous_path) {
        eprintln!(
            "PartNest restore retained previous database {}: {error}",
            previous_path.display()
        );
    }
    if state.moved_wal {
        if let Err(error) = remove_if_exists(&previous_wal) {
            eprintln!(
                "PartNest restore retained previous WAL {}: {error}",
                previous_wal.display()
            );
        }
    }
    if state.moved_shm {
        if let Err(error) = remove_if_exists(&previous_shm) {
            eprintln!(
                "PartNest restore retained previous SHM {}: {error}",
                previous_shm.display()
            );
        }
    }
    cleanup_recovery_snapshot(&paths.recovery_path);
    Ok(())
}

/// Replace an open live database after validating a selected backup.
pub fn restore_database_file(
    database: &mut Database,
    selected_backup: impl AsRef<Path>,
) -> Result<(), BackupError> {
    restore_database_file_inner(database, selected_backup, RestoreInjection::None)
}

/// Deterministic failure seam used by integration tests to exercise post-swap rollback.
pub fn restore_database_file_with_injected_failure(
    database: &mut Database,
    selected_backup: impl AsRef<Path>,
) -> Result<(), BackupError> {
    restore_database_file_inner(database, selected_backup, RestoreInjection::AfterSwap)
}

/// Deterministic failure seam used by integration tests to exercise the
/// post-open candidate cleanup and rollback path.
pub fn restore_database_file_with_injected_post_open_failure(
    database: &mut Database,
    selected_backup: impl AsRef<Path>,
) -> Result<(), BackupError> {
    restore_database_file_inner(database, selected_backup, RestoreInjection::AfterOpen)
}

/// Deterministic failure seam used by integration tests to exercise preserved
/// recovery when a rollback operation itself fails.
pub fn restore_database_file_with_injected_rollback_failure(
    database: &mut Database,
    selected_backup: impl AsRef<Path>,
) -> Result<(), BackupError> {
    restore_database_file_inner(
        database,
        selected_backup,
        RestoreInjection::AfterOpenRollbackFailure,
    )
}

/// Deterministic failure seam used by integration tests to exercise the
/// fatal path after filesystem rollback but before reopening the live file.
pub fn restore_database_file_with_injected_live_reopen_failure(
    database: &mut Database,
    selected_backup: impl AsRef<Path>,
) -> Result<(), BackupError> {
    restore_database_file_inner(
        database,
        selected_backup,
        RestoreInjection::AfterOpenLiveReopenFailure,
    )
}
