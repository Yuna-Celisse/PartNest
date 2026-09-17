//! Projects: one imported BOM per row, stored as a parsed snapshot.
//!
//! Importing an interactive HTML export or a CSV/XLSX table creates a project,
//! and the welding workspace opens from that project. The snapshot is the
//! normalized BOM the import produced, so reopening a project never needs the
//! original file, a cache copy, or another round of field mapping.

use super::types::NormalizedBomDto;
use crate::db::{new_id, Database};
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fmt, fs, path::Path};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProjectKind {
    Interactive,
    Tabular,
}

impl ProjectKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Interactive => "interactive",
            Self::Tabular => "tabular",
        }
    }

    /// The file picker offers exactly these extensions, so the import decision
    /// is made from the name the operator chose.
    pub fn from_source_path(path: &Path) -> Result<Self, ProjectError> {
        let extension = path
            .extension()
            .and_then(|name| name.to_str())
            .map(|name| name.to_ascii_lowercase())
            .unwrap_or_default();
        match extension.as_str() {
            "html" => Ok(Self::Interactive),
            "csv" | "xls" | "xlsx" => Ok(Self::Tabular),
            other => Err(ProjectError::UnsupportedSource(format!(
                "expected an html, csv, xls or xlsx file name, got {other:?}"
            ))),
        }
    }

    /// Parse the `projects.kind` column.
    pub fn from_db(value: &str) -> Result<Self, ProjectError> {
        match value {
            "interactive" => Ok(Self::Interactive),
            "tabular" => Ok(Self::Tabular),
            other => Err(ProjectError::InvalidSnapshot(format!(
                "unknown project BOM kind {other:?}"
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectRecord {
    pub id: String,
    pub name: String,
    pub original_name: String,
    pub sha256: String,
    pub cache_name: String,
    pub kind: ProjectKind,
    /// Whether a parts table backs this project, either as its own source or as
    /// the companion of the interactive export.
    pub has_table: bool,
    pub normalized_json: Option<String>,
}

impl ProjectRecord {
    /// The stored analysis, for projects that have no canvas to re-parse.
    pub fn snapshot(&self) -> Result<NormalizedBomDto, ProjectError> {
        let snapshot = self
            .normalized_json
            .as_deref()
            .ok_or_else(|| ProjectError::MissingSnapshot(self.name.clone()))?;
        serde_json::from_str(snapshot)
            .map_err(|error| ProjectError::InvalidSnapshot(error.to_string()))
    }

    /// Which source the operator still owes, or `None` once both are present.
    pub fn missing_source(&self) -> Option<ProjectKind> {
        match (self.kind, self.has_table) {
            (ProjectKind::Interactive, true) => None,
            (ProjectKind::Interactive, false) => Some(ProjectKind::Tabular),
            (ProjectKind::Tabular, _) => Some(ProjectKind::Interactive),
        }
    }
}

#[derive(Debug)]
pub enum ProjectError {
    Io(std::io::Error),
    Database(rusqlite::Error),
    UnsupportedSource(String),
    InvalidName,
    Missing(String),
    MissingSnapshot(String),
    InvalidSnapshot(String),
    InUse,
    AlreadyComplete,
    SessionActive,
    TakesExist,
    DuplicateSource,
    SuppliesWrongSource(String),
}

impl fmt::Display for ProjectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "project I/O error: {error}"),
            Self::Database(error) => write!(f, "project database error: {error}"),
            Self::UnsupportedSource(reason) => write!(f, "unsupported BOM source: {reason}"),
            Self::InvalidName => f.write_str("project name cannot be empty"),
            Self::Missing(id) => write!(f, "project {id:?} is not available"),
            Self::MissingSnapshot(name) => write!(f, "project {name:?} has no analysis snapshot"),
            Self::InvalidSnapshot(reason) => write!(f, "project snapshot is invalid: {reason}"),
            Self::InUse => f.write_str("project is referenced by a welding session"),
            Self::AlreadyComplete => f.write_str("project already has both sources"),
            Self::SessionActive => f.write_str("project owns the active welding session"),
            Self::TakesExist => f.write_str("project already has recorded takes"),
            Self::DuplicateSource => {
                f.write_str("supplied file already belongs to another project")
            }
            Self::SuppliesWrongSource(need) => write!(f, "supplementary source mismatch: {need}"),
        }
    }
}

impl std::error::Error for ProjectError {}

impl ProjectError {
    /// Reader-facing text; `Display` keeps the English form for logs.
    pub fn user_message(&self) -> String {
        match self {
            Self::UnsupportedSource(reason) => format!("无法导入该 BOM：{reason}"),
            Self::InvalidName => "项目名称不能为空".into(),
            Self::Missing(_) => "找不到该项目，请重新导入".into(),
            Self::MissingSnapshot(_) => {
                "该项目缺少解析结果，请在「项目」页重新导入原始 BOM 文件".into()
            }
            Self::InvalidSnapshot(reason) => format!("该项目的解析结果已损坏：{reason}"),
            Self::InUse => "该项目已有焊接会话记录，无法删除".into(),
            Self::AlreadyComplete => "该项目的交互式与表格都已导入，无需补充导入".into(),
            Self::SessionActive => "该项目正在焊接中，请先退出当前焊接再补充导入".into(),
            Self::TakesExist => "该项目已有取用记录，补充导入会改变器件归属，无法进行".into(),
            Self::DuplicateSource => "这个文件已经是另一个项目了，请在项目列表里直接打开它".into(),
            Self::SuppliesWrongSource(need) => format!("补充导入的来源不对，该项目{need}"),
            Self::Io(error) => format!("项目记录读写失败：{error}"),
            Self::Database(error) => format!("项目保存失败：{error}"),
        }
    }
}

impl From<std::io::Error> for ProjectError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<rusqlite::Error> for ProjectError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Database(error)
    }
}

/// Store an imported BOM as a project and return its id. Same content stays one
/// project, so re-importing a BOM refreshes it instead of piling up duplicates.
pub fn record_project(
    db: &Database,
    source_path: &Path,
    bytes: &[u8],
    name: Option<&str>,
    kind: ProjectKind,
    with_table: bool,
    normalized: &NormalizedBomDto,
) -> Result<ProjectRecord, ProjectError> {
    let original_name = source_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_owned();
    let sha256 = hex::encode(Sha256::digest(bytes));
    let cache_name = cache_name(kind, &sha256, source_path)?;
    let provided = name.map(str::trim).filter(|name| !name.is_empty());
    let snapshot = serde_json::to_string(normalized)
        .map_err(|error| ProjectError::InvalidSnapshot(error.to_string()))?;
    let default_name = default_project_name(&original_name);
    let has_table = matches!(kind, ProjectKind::Tabular) || with_table;
    db.connection().execute(
        "INSERT INTO projects (id, name, original_name, sha256, cache_name, kind, has_table, normalized_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) ON CONFLICT(sha256) DO UPDATE SET original_name = excluded.original_name, name = COALESCE(?9, name), cache_name = excluded.cache_name, kind = excluded.kind, has_table = excluded.has_table, normalized_json = excluded.normalized_json",
        rusqlite::params![
            new_id(),
            provided.unwrap_or(&default_name),
            original_name,
            sha256,
            cache_name,
            kind.as_str(),
            has_table,
            snapshot,
            provided,
        ],
    )?;
    project_by_sha256(db, &sha256)
}

/// A new project is named after its file without the extension; the list shows
/// the file name on the line below anyway.
fn default_project_name(original_name: &str) -> String {
    match original_name.rfind('.') {
        Some(index) if index > 0 => original_name[..index].to_owned(),
        _ => original_name.to_owned(),
    }
}

/// Read one project row by id.
pub fn project_record(db: &Database, project_id: &str) -> Result<ProjectRecord, ProjectError> {
    let mut statement = db.connection().prepare(
        "SELECT id, name, original_name, sha256, cache_name, kind, has_table, normalized_json FROM projects WHERE id = ?1",
    )?;
    let row = statement
        .query_row([project_id], read_project_row)
        .optional()?
        .ok_or_else(|| ProjectError::Missing(project_id.to_owned()))?;
    build_project(row)
}

/// The row an import just wrote or refreshed, looked up by content hash.
fn project_by_sha256(db: &Database, sha256: &str) -> Result<ProjectRecord, ProjectError> {
    let mut statement = db
        .connection()
        .prepare("SELECT id, name, original_name, sha256, cache_name, kind, has_table, normalized_json FROM projects WHERE sha256 = ?1")?;
    let row = statement
        .query_row([sha256], read_project_row)
        .optional()?
        .ok_or_else(|| ProjectError::Missing(sha256.to_owned()))?;
    build_project(row)
}

type ProjectRow = (
    String,
    String,
    String,
    String,
    String,
    String,
    bool,
    Option<String>,
);

fn read_project_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ProjectRow> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
    ))
}

fn build_project(row: ProjectRow) -> Result<ProjectRecord, ProjectError> {
    Ok(ProjectRecord {
        id: row.0,
        name: row.1,
        original_name: row.2,
        sha256: row.3,
        cache_name: row.4,
        kind: ProjectKind::from_db(&row.5)?,
        has_table: row.6,
        normalized_json: row.7,
    })
}

/// Reopen the stored snapshot of a project.
pub fn load_snapshot(db: &Database, project_id: &str) -> Result<NormalizedBomDto, ProjectError> {
    project_record(db, project_id)?.snapshot()
}

pub fn rename_project(
    db: &Database,
    project_id: &str,
    name: &str,
) -> Result<ProjectRecord, ProjectError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(ProjectError::InvalidName);
    }
    let updated = db.connection().execute(
        "UPDATE projects SET name = ?1 WHERE id = ?2",
        rusqlite::params![name, project_id],
    )?;
    if updated == 0 {
        return Err(ProjectError::Missing(project_id.to_owned()));
    }
    project_record(db, project_id)
}

/// Whether a project may still receive its missing source, and which one.
pub fn ensure_supplementable(
    db: &Database,
    project_id: &str,
) -> Result<ProjectRecord, ProjectError> {
    let record = project_record(db, project_id)?;
    if record.missing_source().is_none() {
        return Err(ProjectError::AlreadyComplete);
    }
    let active: bool = db.connection().query_row(
        "SELECT EXISTS(SELECT 1 FROM welding_sessions WHERE project_id = ?1 AND status = 'active')",
        [project_id],
        |row| row.get(0),
    )?;
    if active {
        return Err(ProjectError::SessionActive);
    }
    let started: bool = db.connection().query_row(
        "SELECT EXISTS(SELECT 1 FROM welding_progress progress JOIN welding_sessions session ON session.id = progress.session_id WHERE session.project_id = ?1)",
        [project_id],
        |row| row.get(0),
    )?;
    if started {
        return Err(ProjectError::TakesExist);
    }
    Ok(record)
}

/// Store the merged result of a supplementary import on the project itself.
pub fn apply_supplement(
    db: &Database,
    project_id: &str,
    kind: ProjectKind,
    sha256: &str,
    cache_name: &str,
    normalized: &NormalizedBomDto,
) -> Result<ProjectRecord, ProjectError> {
    let snapshot = serde_json::to_string(normalized)
        .map_err(|error| ProjectError::InvalidSnapshot(error.to_string()))?;
    let updated = db.connection().execute(
        "UPDATE projects SET kind = ?2, sha256 = ?3, cache_name = ?4, has_table = 1, normalized_json = ?5 WHERE id = ?1",
        rusqlite::params![project_id, kind.as_str(), sha256, cache_name, snapshot],
    );
    match updated {
        Ok(0) => return Err(ProjectError::Missing(project_id.to_owned())),
        Ok(_) => {}
        Err(rusqlite::Error::SqliteFailure(error, _))
            if error.code == rusqlite::ErrorCode::ConstraintViolation =>
        {
            return Err(ProjectError::DuplicateSource)
        }
        Err(error) => return Err(ProjectError::Database(error)),
    }
    project_record(db, project_id)
}

/// Delete a project record and its cached canvas. Projects that welding sessions
/// reference are kept so the inventory audit trail stays readable.
pub fn remove_project(
    db: &Database,
    cache_dir: &Path,
    project_id: &str,
) -> Result<(), ProjectError> {
    let record = project_record(db, project_id)?;
    match db
        .connection()
        .execute("DELETE FROM projects WHERE id = ?1", [project_id])
    {
        Ok(_) => {}
        Err(rusqlite::Error::SqliteFailure(error, _))
            if error.code == rusqlite::ErrorCode::ConstraintViolation =>
        {
            return Err(ProjectError::InUse)
        }
        Err(error) => return Err(ProjectError::Database(error)),
    }
    if let Some(path) = cache_file_path(cache_dir, &record.cache_name) {
        let _ = fs::remove_file(path);
    }
    Ok(())
}

/// Resolve a stored cache name inside the cache directory, rejecting anything
/// that is not a content-addressed file name.
fn cache_file_path(cache_dir: &Path, cache_name: &str) -> Option<std::path::PathBuf> {
    let (hash, _) = cache_name.split_once('.')?;
    let content_addressed = hash.len() == 64
        && hash
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        && !cache_name.contains('/')
        && !cache_name.contains('\\');
    content_addressed.then(|| cache_dir.join(cache_name))
}

/// Interactive rows are named after the cached canvas they reopen; tabular rows
/// keep the name they would use, but never write a cache file.
fn cache_name(kind: ProjectKind, sha256: &str, source_path: &Path) -> Result<String, ProjectError> {
    match kind {
        ProjectKind::Interactive => Ok(format!("{sha256}.html")),
        ProjectKind::Tabular => {
            let extension = source_path
                .extension()
                .and_then(|name| name.to_str())
                .map(str::to_ascii_lowercase)
                .filter(|extension| matches!(extension.as_str(), "csv" | "xls" | "xlsx"))
                .ok_or_else(|| {
                    ProjectError::UnsupportedSource(format!(
                        "tabular BOM needs a csv, xls or xlsx file name, got {source_path:?}"
                    ))
                })?;
            Ok(format!("{sha256}.{extension}"))
        }
    }
}
