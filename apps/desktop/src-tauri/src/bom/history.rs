//! Imported-BOM history: parsed snapshots stored in the database.
//!
//! A snapshot is the normalized BOM the analysis produced, so reopening an
//! imported BOM never depends on the original file, on a cache copy, or on
//! re-running field mapping.

use super::types::NormalizedBomDto;
use crate::db::{new_id, Database};
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fmt, fs, path::Path};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BomImportKind {
    Interactive,
    Tabular,
}

impl BomImportKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Interactive => "interactive",
            Self::Tabular => "tabular",
        }
    }

    /// Parse the `bom_files.kind` column.
    pub fn from_db(value: &str) -> Result<Self, HistoryError> {
        match value {
            "interactive" => Ok(Self::Interactive),
            "tabular" => Ok(Self::Tabular),
            other => Err(HistoryError::InvalidSnapshot(format!(
                "unknown BOM kind {other:?}"
            ))),
        }
    }
}

#[derive(Debug)]
pub enum HistoryError {
    Io(std::io::Error),
    Database(rusqlite::Error),
    UnsupportedSource(String),
    Missing(String),
    MissingSnapshot(String),
    InvalidSnapshot(String),
    InUse,
}

impl fmt::Display for HistoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "BOM history I/O error: {error}"),
            Self::Database(error) => write!(f, "BOM history database error: {error}"),
            Self::UnsupportedSource(reason) => write!(f, "unsupported BOM source: {reason}"),
            Self::Missing(id) => write!(f, "imported BOM {id:?} is not available"),
            Self::MissingSnapshot(name) => {
                write!(f, "imported BOM {name:?} has no analysis snapshot")
            }
            Self::InvalidSnapshot(reason) => {
                write!(f, "imported BOM snapshot is invalid: {reason}")
            }
            Self::InUse => f.write_str("imported BOM is referenced by a welding session"),
        }
    }
}

impl std::error::Error for HistoryError {}

impl HistoryError {
    /// Reader-facing text; `Display` keeps the English form for logs.
    pub fn user_message(&self) -> String {
        match self {
            Self::UnsupportedSource(reason) => format!("无法导入该 BOM：{reason}"),
            Self::Missing(_) => "该 BOM 记录不存在，请重新导入".into(),
            Self::MissingSnapshot(_) => {
                "该历史记录没有可用的解析结果，请重新选择原始文件分析".into()
            }
            Self::InvalidSnapshot(reason) => format!("历史记录的解析结果已损坏：{reason}"),
            Self::InUse => "该 BOM 存在焊接会话记录，无法移除".into(),
            Self::Io(error) => format!("BOM 记录读写失败：{error}"),
            Self::Database(error) => format!("BOM 记录保存失败：{error}"),
        }
    }
}

impl From<std::io::Error> for HistoryError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<rusqlite::Error> for HistoryError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Database(error)
    }
}

/// Store an analysed BOM as a snapshot. Welding sessions and interactive cache
/// copies are never touched; the snapshot is what the history reopens later.
pub fn record_import(
    db: &Database,
    source_path: &Path,
    display_name: Option<&str>,
    kind: BomImportKind,
    normalized: &NormalizedBomDto,
) -> Result<(), HistoryError> {
    let bytes = fs::read(source_path)?;
    let original_name = source_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_owned();
    let sha256 = hex::encode(Sha256::digest(&bytes));
    let cache_name = cache_name(kind, &sha256, source_path)?;
    let provided = display_name.map(str::trim).filter(|name| !name.is_empty());
    let snapshot = serde_json::to_string(normalized)
        .map_err(|error| HistoryError::InvalidSnapshot(error.to_string()))?;
    db.connection().execute(
        "INSERT INTO bom_files (id, original_name, display_name, sha256, cache_name, kind, normalized_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) ON CONFLICT(sha256) DO UPDATE SET original_name = excluded.original_name, display_name = COALESCE(?8, display_name), cache_name = excluded.cache_name, kind = excluded.kind, normalized_json = excluded.normalized_json",
        rusqlite::params![
            new_id(),
            original_name,
            provided.unwrap_or(&original_name),
            sha256,
            cache_name,
            kind.as_str(),
            snapshot,
            provided
        ],
    )?;
    Ok(())
}

/// Reopen the stored snapshot of an imported BOM.
pub fn load_import(db: &Database, bom_file_id: &str) -> Result<NormalizedBomDto, HistoryError> {
    let (original_name, snapshot) = db
        .connection()
        .query_row(
            "SELECT original_name, normalized_json FROM bom_files WHERE id = ?1",
            [bom_file_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
        )
        .optional()?
        .ok_or_else(|| HistoryError::Missing(bom_file_id.to_owned()))?;
    let snapshot = snapshot.ok_or(HistoryError::MissingSnapshot(original_name))?;
    serde_json::from_str(&snapshot)
        .map_err(|error| HistoryError::InvalidSnapshot(error.to_string()))
}

/// Remove an imported BOM record and its cache copy. Records that welding
/// sessions reference are kept so the audit trail stays readable.
pub fn remove_import(
    db: &Database,
    cache_dir: &Path,
    bom_file_id: &str,
) -> Result<(), HistoryError> {
    let cache_name: Option<String> = db
        .connection()
        .query_row(
            "SELECT cache_name FROM bom_files WHERE id = ?1",
            [bom_file_id],
            |row| row.get(0),
        )
        .optional()?;
    let Some(cache_name) = cache_name else {
        return Err(HistoryError::Missing(bom_file_id.to_owned()));
    };
    match db
        .connection()
        .execute("DELETE FROM bom_files WHERE id = ?1", [bom_file_id])
    {
        Ok(_) => {}
        Err(rusqlite::Error::SqliteFailure(error, _))
            if error.code == rusqlite::ErrorCode::ConstraintViolation =>
        {
            return Err(HistoryError::InUse)
        }
        Err(error) => return Err(HistoryError::Database(error)),
    }
    if let Some(path) = cache_file_path(cache_dir, &cache_name) {
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

/// Tabular rows keep the content-addressed name they would use as a cache
/// entry; only interactive rows are ever loaded into the welding frame.
fn cache_name(
    kind: BomImportKind,
    sha256: &str,
    source_path: &Path,
) -> Result<String, HistoryError> {
    match kind {
        BomImportKind::Interactive => Ok(format!("{sha256}.html")),
        BomImportKind::Tabular => {
            let extension = source_path
                .extension()
                .and_then(|name| name.to_str())
                .map(str::to_ascii_lowercase)
                .filter(|extension| matches!(extension.as_str(), "csv" | "xlsx"))
                .ok_or_else(|| {
                    HistoryError::UnsupportedSource(format!(
                        "tabular BOM needs a csv or xlsx file name, got {source_path:?}"
                    ))
                })?;
            Ok(format!("{sha256}.{extension}"))
        }
    }
}
