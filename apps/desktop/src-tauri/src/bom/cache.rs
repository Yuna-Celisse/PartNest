//! Immutable source caching and active interactive BOM session metadata.

use super::{
    bridge::{constant_time_eq, validate_token_and_designators, BridgeError},
    interactive_html::{
        parse_interactive_html_text_with_companion, InteractiveHtmlError, MAX_INTERACTIVE_BOM_BYTES,
    },
    projects::{
        apply_supplement, ensure_supplementable, project_record, record_project, ProjectError,
        ProjectKind, ProjectRecord,
    },
    tabular::{inspect_tabular_bom, parse_tabular_bom, TabularError},
    types::{BomSide, FieldMapping, ImportPreview, NormalizedBomDto},
};
use crate::db::{new_id, Database};
use rusqlite::OptionalExtension;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fmt, fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use uuid::Uuid;

pub use super::bridge::SelectionError;

const BRIDGE_JS: &str = include_str!("../../resources/bridge-v1.js");
const BRIDGE_MARKER: &[u8] = b"\n<!-- partnest bridge-v1 -->";
const COMPANION_START: &[u8] =
    b"<script data-partnest-companion=\"bom-v1\" type=\"application/json\">";
const COMPANION_END: &[u8] = b"</script>";

#[derive(Debug, Clone, Serialize)]
pub struct CachedBomSession {
    pub session_id: String,
    pub project_id: String,
    pub project_name: String,
    pub original_name: String,
    pub sha256: String,
    pub cache_name: String,
    /// `None` for tabular projects: they have no interactive canvas to load.
    pub cache_path: Option<PathBuf>,
    pub kind: ProjectKind,
    pub token: String,
    pub normalized: NormalizedBomDto,
}

/// What an import stored: the project row plus the analysis it came from.
#[derive(Debug, Clone, Serialize)]
pub struct ImportedProject {
    pub project_id: String,
    pub name: String,
    pub original_name: String,
    pub cache_name: String,
    pub kind: ProjectKind,
    pub has_table: bool,
    pub normalized: NormalizedBomDto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResolvedSelection {
    pub session_id: String,
    pub component_key: String,
    /// `None` when the BOM records no board side for the selection; the
    /// operator's current side tab then decides where the take is recorded.
    pub side: Option<BomSide>,
    pub designators: Vec<String>,
}

#[derive(Debug)]
pub enum CacheError {
    Io(std::io::Error),
    Database(rusqlite::Error),
    Parse(InteractiveHtmlError),
    Tabular(TabularError),
    Project(ProjectError),
    NeedsMapping,
    TamperedCache(String),
    AtomicReplace(String),
    Companion(String),
    MissingCache(String),
}
impl fmt::Display for CacheError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "cache I/O error: {error}"),
            Self::Database(error) => write!(f, "cache database error: {error}"),
            Self::Parse(error) => error.fmt(f),
            Self::Tabular(error) => write!(f, "tabular BOM error: {error}"),
            Self::Project(error) => error.fmt(f),
            Self::NeedsMapping => {
                f.write_str("tabular BOM needs a field mapping before it can be imported")
            }
            Self::TamperedCache(reason) => write!(f, "cached BOM is invalid: {reason}"),
            Self::AtomicReplace(reason) => write!(f, "cached BOM atomic replace failed: {reason}"),
            Self::Companion(reason) => write!(f, "companion CSV error: {reason}"),
            Self::MissingCache(name) => {
                write!(f, "cached BOM file for {name:?} is not available")
            }
        }
    }
}
impl std::error::Error for CacheError {}

impl CacheError {
    /// Reader-facing text; `Display` keeps the English form for logs.
    pub fn user_message(&self) -> String {
        match self {
            Self::Parse(error) => error.user_message(),
            Self::Project(error) => error.user_message(),
            Self::MissingCache(_) => {
                "该项目的画布缓存已丢失，请在「项目」页重新导入原始 BOM 文件".into()
            }
            Self::TamperedCache(_) => "该项目的 BOM 缓存已损坏，请在「项目」页重新导入".into(),
            Self::NeedsMapping => "表格 BOM 需要先完成字段映射，再导入为项目".into(),
            Self::Companion(reason) => format!("配套 CSV 解析失败：{reason}"),
            Self::AtomicReplace(reason) => format!("写入 BOM 缓存失败：{reason}"),
            Self::Io(error) => format!("BOM 缓存读写失败：{error}"),
            Self::Database(error) => format!("焊接会话保存失败：{error}"),
            Self::Tabular(error) => format!("表格 BOM 解析失败：{error}"),
        }
    }
}
impl From<std::io::Error> for CacheError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}
impl From<rusqlite::Error> for CacheError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Database(error)
    }
}
impl From<InteractiveHtmlError> for CacheError {
    fn from(error: InteractiveHtmlError) -> Self {
        Self::Parse(error)
    }
}
impl From<TabularError> for CacheError {
    fn from(error: TabularError) -> Self {
        Self::Tabular(error)
    }
}
impl From<ProjectError> for CacheError {
    fn from(error: ProjectError) -> Self {
        Self::Project(error)
    }
}

struct ActiveSession {
    session_id: String,
    token: String,
    designators: BTreeMap<String, DesignatorBinding>,
}

#[derive(Clone)]
struct DesignatorBinding {
    component_key: String,
    /// `None` for BOMs that do not record a board side for this placement.
    side: Option<BomSide>,
}

/// A cached interactive copy reopened for a new session.
struct ReopenedCanvas {
    cache_path: PathBuf,
    token: String,
    normalized: NormalizedBomDto,
}

/// A verified cached canvas, free of the bridge payload.
struct CachedCanvas {
    cache_path: PathBuf,
    source: Vec<u8>,
    companion: Option<NormalizedBomDto>,
}

pub struct InteractiveBomCache<'db> {
    db: &'db Database,
    cache_dir: PathBuf,
    active: Arc<Mutex<Option<ActiveSession>>>,
}

/// Process-managed state used by Tauri commands. The database remains owned by
/// Tauri's existing `Database` state; this object keeps only cache location
/// and the in-memory token/session lookup.
pub struct InteractiveBomRuntime {
    cache_dir: PathBuf,
    active: Arc<Mutex<Option<ActiveSession>>>,
}

impl InteractiveBomRuntime {
    pub fn new(cache_dir: impl AsRef<Path>) -> Self {
        Self {
            cache_dir: cache_dir.as_ref().to_owned(),
            active: Arc::new(Mutex::new(None)),
        }
    }

    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }

    /// Import a BOM file as a project. Importing never switches the welding
    /// session: the operator opens the workspace from the project when ready.
    pub fn import_project(
        &self,
        db: &Database,
        source_path: &Path,
        name: Option<&str>,
        mapping: Option<&FieldMapping>,
        companion_path: Option<&Path>,
    ) -> Result<ImportedProject, CacheError> {
        InteractiveBomCache::with_active(db, &self.cache_dir, self.active.clone()).import_project(
            source_path,
            name,
            mapping,
            companion_path,
        )
    }

    /// Add the source an existing project is still missing.
    pub fn supplement_project(
        &self,
        db: &Database,
        project_id: &str,
        source_path: &Path,
        mapping: Option<&FieldMapping>,
    ) -> Result<ImportedProject, CacheError> {
        InteractiveBomCache::with_active(db, &self.cache_dir, self.active.clone())
            .supplement_project(project_id, source_path, mapping)
    }

    pub fn resolve_bom_selection(
        &self,
        db: &Database,
        token: &str,
        designators: &[String],
    ) -> Result<ResolvedSelection, BridgeError> {
        InteractiveBomCache::with_active(db, &self.cache_dir, self.active.clone())
            .resolve_bom_selection(token, designators)
    }

    pub fn restore_active_session(
        &self,
        db: &Database,
    ) -> Result<Option<CachedBomSession>, CacheError> {
        InteractiveBomCache::with_active(db, &self.cache_dir, self.active.clone())
            .restore_active_session()
    }

    /// Open the welding workspace for a project, making it the one active session.
    pub fn open_project_welding(
        &self,
        db: &Database,
        project_id: &str,
    ) -> Result<CachedBomSession, CacheError> {
        InteractiveBomCache::with_active(db, &self.cache_dir, self.active.clone())
            .open_project_welding(project_id)
    }

    /// Drop the in-memory bridge token and designator bindings after the
    /// backing database has been replaced by a restore operation.
    pub fn invalidate_active_session(&self) -> Result<(), CacheError> {
        let mut active = self
            .active
            .lock()
            .map_err(|_| CacheError::Io(std::io::Error::other("session lock poisoned")))?;
        *active = None;
        Ok(())
    }

    /// Validate the ownership captured by the active BOM bridge before a
    /// welding transaction is allowed to mutate inventory.
    pub fn validate_welding_selection(
        &self,
        db: &Database,
        session_id: &str,
        component_key: &str,
        side: &BomSide,
        designators: &[String],
    ) -> Result<i64, BridgeError> {
        let active = self
            .active
            .lock()
            .map_err(|_| BridgeError::InactiveSession)?;
        let active = active.as_ref().ok_or(BridgeError::InactiveSession)?;
        if active.session_id != session_id {
            return Err(BridgeError::InactiveSession);
        }
        let still_active = db
            .connection()
            .query_row(
                "SELECT status FROM welding_sessions WHERE id = ?1",
                [session_id],
                |row| row.get::<_, String>(0),
            )
            .map(|status| status == "active")
            .unwrap_or(false);
        if !still_active {
            return Err(BridgeError::InactiveSession);
        }
        validate_token_and_designators("welding-selection", designators)?;
        let mut submitted = std::collections::HashSet::with_capacity(designators.len());
        for designator in designators {
            if !submitted.insert(designator) {
                return Err(BridgeError::DuplicateDesignator);
            }
            let binding = active
                .designators
                .get(designator)
                .ok_or(BridgeError::UnknownDesignator)?;
            if binding.component_key != component_key {
                return Err(BridgeError::CrossGroupSelection);
            }
            if let Some(recorded) = &binding.side {
                if recorded != side {
                    return Err(BridgeError::MixedSideSelection);
                }
            }
        }
        Ok(designators.len() as i64)
    }
}

impl<'db> InteractiveBomCache<'db> {
    pub fn new(db: &'db Database, cache_dir: impl AsRef<Path>) -> Self {
        Self::with_active(db, cache_dir, Arc::new(Mutex::new(None)))
    }

    fn with_active(
        db: &'db Database,
        cache_dir: impl AsRef<Path>,
        active: Arc<Mutex<Option<ActiveSession>>>,
    ) -> Self {
        Self {
            db,
            cache_dir: cache_dir.as_ref().to_owned(),
            active,
        }
    }

    /// Import a BOM file as a project. An interactive BOM also gets its bridged
    /// canvas copy written here, so opening the workspace later on never needs
    /// the original file again.
    pub fn import_project(
        &self,
        source_path: &Path,
        name: Option<&str>,
        mapping: Option<&FieldMapping>,
        companion_path: Option<&Path>,
    ) -> Result<ImportedProject, CacheError> {
        let kind = ProjectKind::from_source_path(source_path)?;
        let (bytes, original_name) = read_source(source_path, kind)?;
        let (normalized, companion) = match kind {
            ProjectKind::Interactive => {
                parse_interactive_source(&bytes, &original_name, companion_path)?
            }
            ProjectKind::Tabular => match inspect_tabular_bom(source_path, mapping)? {
                ImportPreview::Ready(normalized) => (normalized, None),
                // The import dialog resolves mapping before calling, so this is
                // a guard against a stale mapping rather than a user flow.
                ImportPreview::NeedsMapping { .. } => return Err(CacheError::NeedsMapping),
            },
        };
        let record = record_project(
            self.db,
            source_path,
            &bytes,
            name,
            kind,
            companion.is_some(),
            &normalized,
        )?;
        if kind == ProjectKind::Interactive {
            // The token baked into a freshly imported canvas is never published;
            // opening the project mints the one the bridge has to present.
            let cache_path = validate_cache_path(&self.cache_dir, &record.cache_name)?;
            self.write_cache_atomically(
                &cache_path,
                &bytes,
                &Uuid::new_v4().to_string(),
                &bindings(&normalized)?,
                companion.as_ref(),
            )?;
        }
        Ok(ImportedProject {
            project_id: record.id,
            name: record.name,
            original_name: record.original_name,
            cache_name: record.cache_name,
            kind: record.kind,
            has_table: record.has_table,
            normalized,
        })
    }

    /// Add the source a project is still missing: a parts table merged into its
    /// cached canvas, or a canvas built on top of its stored table analysis.
    pub fn supplement_project(
        &self,
        project_id: &str,
        source_path: &Path,
        mapping: Option<&FieldMapping>,
    ) -> Result<ImportedProject, CacheError> {
        let record = ensure_supplementable(self.db, project_id)?;
        let supplied = ProjectKind::from_source_path(source_path)?;
        if record.missing_source() != Some(supplied) {
            return Err(CacheError::Project(ProjectError::SuppliesWrongSource(
                match record.missing_source() {
                    Some(ProjectKind::Tabular) => "需要器件表格（CSV / XLS / XLSX）".into(),
                    Some(ProjectKind::Interactive) => "需要可交互式 BOM（HTML）".into(),
                    None => String::new(),
                },
            )));
        }
        let (record, normalized) = match supplied {
            ProjectKind::Tabular => self.attach_table(&record, source_path, mapping)?,
            ProjectKind::Interactive => self.attach_canvas(&record, source_path)?,
        };
        Ok(ImportedProject {
            project_id: record.id.clone(),
            name: record.name.clone(),
            original_name: record.original_name.clone(),
            cache_name: record.cache_name.clone(),
            kind: record.kind,
            has_table: record.has_table,
            normalized,
        })
    }

    /// Merge a parts table into the project's cached canvas and store the result.
    fn attach_table(
        &self,
        record: &ProjectRecord,
        table_path: &Path,
        mapping: Option<&FieldMapping>,
    ) -> Result<(ProjectRecord, NormalizedBomDto), CacheError> {
        let companion = match inspect_tabular_bom(table_path, mapping)? {
            ImportPreview::Ready(normalized) => normalized,
            ImportPreview::NeedsMapping { .. } => return Err(CacheError::NeedsMapping),
        };
        let canvas = self.cached_canvas(record)?;
        let source_text = std::str::from_utf8(&canvas.source)
            .map_err(|error| CacheError::TamperedCache(error.to_string()))?;
        let normalized = parse_interactive_html_text_with_companion(
            source_text,
            record.original_name.clone(),
            Some(&companion),
        )?;
        // The table travels inside the cached copy, so a later restart rebuilds
        // the same merge without the operator picking the file again.
        self.write_cache_atomically(
            &canvas.cache_path,
            &canvas.source,
            &Uuid::new_v4().to_string(),
            &bindings(&normalized)?,
            Some(&companion),
        )?;
        let record = apply_supplement(
            self.db,
            &record.id,
            ProjectKind::Interactive,
            &record.sha256,
            &record.cache_name,
            &normalized,
        )?;
        Ok((record, normalized))
    }

    /// Cache an interactive export as the project's canvas, keeping the analysis
    /// it already stored as the parts table.
    fn attach_canvas(
        &self,
        record: &ProjectRecord,
        html_path: &Path,
    ) -> Result<(ProjectRecord, NormalizedBomDto), CacheError> {
        let (bytes, original_name) = read_source(html_path, ProjectKind::Interactive)?;
        let companion = record.snapshot()?;
        let source_text = std::str::from_utf8(&bytes).map_err(|error| {
            CacheError::Parse(InteractiveHtmlError::UnsupportedInteractiveBom(
                error.to_string(),
            ))
        })?;
        let normalized = parse_interactive_html_text_with_companion(
            source_text,
            original_name,
            Some(&companion),
        )?;
        let sha256 = hex::encode(Sha256::digest(&bytes));
        let cache_name = format!("{sha256}.html");
        // Record first: a canvas that belongs to another project must not be
        // written over before that clash is reported.
        let record = apply_supplement(
            self.db,
            &record.id,
            ProjectKind::Interactive,
            &sha256,
            &cache_name,
            &normalized,
        )?;
        let cache_path = validate_cache_path(&self.cache_dir, &cache_name)?;
        self.write_cache_atomically(
            &cache_path,
            &bytes,
            &Uuid::new_v4().to_string(),
            &bindings(&normalized)?,
            Some(&companion),
        )?;
        Ok((record, normalized))
    }

    pub fn resolve_bom_selection(
        &self,
        token: &str,
        designators: &[String],
    ) -> Result<ResolvedSelection, BridgeError> {
        validate_token_and_designators(token, designators)?;
        let active = self
            .active
            .lock()
            .map_err(|_| BridgeError::InactiveSession)?;
        let active = active.as_ref().ok_or(BridgeError::InactiveSession)?;
        let still_active = self
            .db
            .connection()
            .query_row(
                "SELECT status FROM welding_sessions WHERE id = ?1",
                [&active.session_id],
                |row| row.get::<_, String>(0),
            )
            .map(|status| status == "active")
            .unwrap_or(false);
        if !still_active {
            return Err(BridgeError::InactiveSession);
        }
        if !constant_time_eq(&active.token, token) {
            return Err(BridgeError::InvalidToken);
        }
        let mut group = None;
        let mut side = None;
        let mut unique = std::collections::HashSet::with_capacity(designators.len());
        for designator in designators {
            if !unique.insert(designator) {
                return Err(BridgeError::DuplicateDesignator);
            }
            let binding = active
                .designators
                .get(designator)
                .ok_or(BridgeError::UnknownDesignator)?;
            if let Some(expected) = &group {
                if expected != &binding.component_key {
                    return Err(BridgeError::CrossGroupSelection);
                }
            } else {
                group = Some(binding.component_key.clone());
            }
            if let (Some(expected), Some(recorded)) = (&side, &binding.side) {
                if expected != recorded {
                    return Err(BridgeError::MixedSideSelection);
                }
            } else if side.is_none() {
                side = binding.side.clone();
            }
        }
        Ok(ResolvedSelection {
            session_id: active.session_id.clone(),
            component_key: group.expect("nonempty checked"),
            side,
            designators: designators.to_owned(),
        })
    }

    /// Re-open the session the database still considers active, so returning to
    /// the workspace restores the same project and its recorded progress.
    pub fn restore_active_session(&self) -> Result<Option<CachedBomSession>, CacheError> {
        let active = self
            .db
            .connection()
            .query_row(
                "SELECT id, project_id FROM welding_sessions WHERE status = 'active' ORDER BY updated_at DESC LIMIT 1",
                [],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?;
        let Some((session_id, project_id)) = active else {
            return Ok(None);
        };
        let record = project_record(self.db, &project_id)?;
        Ok(Some(self.publish_session(&session_id, &record)?))
    }

    /// Verify a cached interactive copy, re-issue its bridge token, and rebuild
    /// the designator bindings the host trusts for that canvas.
    fn reopen_canvas(&self, record: &ProjectRecord) -> Result<ReopenedCanvas, CacheError> {
        let canvas = self.cached_canvas(record)?;
        let source_text = std::str::from_utf8(&canvas.source)
            .map_err(|error| CacheError::TamperedCache(error.to_string()))?;
        let normalized = parse_interactive_html_text_with_companion(
            source_text,
            record.original_name.clone(),
            canvas.companion.as_ref(),
        )?;
        let token = Uuid::new_v4().to_string();
        let designators = bindings(&normalized)?;
        self.write_cache_atomically(
            &canvas.cache_path,
            &canvas.source,
            &token,
            &designators,
            canvas.companion.as_ref(),
        )?;
        Ok(ReopenedCanvas {
            cache_path: canvas.cache_path,
            token,
            normalized,
        })
    }

    /// The project's cached export, stripped of the bridge payload, together
    /// with the companion table that was merged into it.
    fn cached_canvas(&self, record: &ProjectRecord) -> Result<CachedCanvas, CacheError> {
        let cache_path = validate_cache_path(&self.cache_dir, &record.cache_name)?;
        let cached = fs::read(&cache_path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                CacheError::MissingCache(record.original_name.clone())
            } else {
                CacheError::Io(error)
            }
        })?;
        let marker_index = cached
            .windows(BRIDGE_MARKER.len())
            .rposition(|window| window == BRIDGE_MARKER)
            .ok_or_else(|| CacheError::TamperedCache("bridge marker missing".into()))?;
        let source = cached[..marker_index].to_vec();
        if hex::encode(Sha256::digest(&source)) != record.sha256 {
            return Err(CacheError::TamperedCache(
                "cached source hash mismatch".into(),
            ));
        }
        let companion = read_cached_companion(&cached[marker_index..])?;
        Ok(CachedCanvas {
            cache_path,
            source,
            companion,
        })
    }

    /// Open the welding workspace for a project. Its already-active session is
    /// reused, so leaving and returning keeps the takes recorded so far.
    pub fn open_project_welding(&self, project_id: &str) -> Result<CachedBomSession, CacheError> {
        let record = project_record(self.db, project_id)?;
        let session_id = {
            let tx = self.db.transaction()?;
            let session_id = activate_session(&tx, &record.id)?;
            tx.commit()?;
            session_id
        };
        self.publish_session(&session_id, &record)
    }

    /// Publish a session that is already active in the database against a stored
    /// project: reopen whatever the project owns as its source of truth, mint the
    /// bridge token, and bind the designators that token may select.
    fn publish_session(
        &self,
        session_id: &str,
        record: &ProjectRecord,
    ) -> Result<CachedBomSession, CacheError> {
        let (normalized, cache_path, token) = match record.kind {
            ProjectKind::Interactive => {
                let canvas = self.reopen_canvas(record)?;
                // The canvas only accepts the token baked into its cached copy.
                (canvas.normalized, Some(canvas.cache_path), canvas.token)
            }
            // Table projects have no canvas: the analysis snapshot drives the
            // workspace and the operator picks designators from the tray.
            ProjectKind::Tabular => (record.snapshot()?, None, Uuid::new_v4().to_string()),
        };
        self.record_active_session(session_id, &token, bindings(&normalized)?)?;
        Ok(CachedBomSession {
            session_id: session_id.to_owned(),
            project_id: record.id.clone(),
            project_name: record.name.clone(),
            original_name: record.original_name.clone(),
            sha256: record.sha256.clone(),
            cache_name: record.cache_name.clone(),
            cache_path,
            kind: record.kind,
            token,
            normalized,
        })
    }

    fn record_active_session(
        &self,
        session_id: &str,
        token: &str,
        designators: BTreeMap<String, DesignatorBinding>,
    ) -> Result<(), CacheError> {
        self.db.connection().execute(
            "UPDATE welding_sessions SET updated_at = ?1 WHERE id = ?2 AND status = 'active'",
            rusqlite::params![crate::db::utc_now(), session_id],
        )?;
        *self
            .active
            .lock()
            .map_err(|_| CacheError::Io(std::io::Error::other("session lock poisoned")))? =
            Some(ActiveSession {
                session_id: session_id.to_owned(),
                token: token.to_owned(),
                designators,
            });
        Ok(())
    }

    fn write_cache_atomically(
        &self,
        target: &Path,
        source: &[u8],
        token: &str,
        designators: &BTreeMap<String, DesignatorBinding>,
        companion: Option<&NormalizedBomDto>,
    ) -> Result<(), CacheError> {
        fs::create_dir_all(&self.cache_dir)?;
        let expected = build_cached_html(source, token, designators, companion)?;
        if same_contents(target, &expected) {
            return Ok(());
        }
        let temp = self.cache_dir.join(format!(
            ".{}.{}.tmp",
            target
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("bom"),
            new_id()
        ));
        let _guard =
            prepare_temp_with(&temp, &expected, |path, contents| fs::write(path, contents))?;
        match atomic_replace(&temp, target) {
            Ok(()) => {}
            Err(_error) if same_contents(target, &expected) => {
                // Another writer completed the same replacement.
            }
            Err(error) => return Err(CacheError::AtomicReplace(error.to_string())),
        }
        Ok(())
    }
}

/// Compare a cached file with the expected bytes without reading a multi-megabyte
/// HTML copy when the size already differs.
fn same_contents(path: &Path, expected: &[u8]) -> bool {
    fs::metadata(path)
        .ok()
        .filter(|meta| meta.is_file() && meta.len() == expected.len() as u64)
        .and_then(|_| fs::read(path).ok())
        .is_some_and(|actual| actual == expected)
}

/// Parse an interactive BOM, and its optional companion CSV, without touching
/// the cache directory or the welding session tables. The BOM analysis workspace
/// uses this so that previewing a file cannot cancel an in-progress session.
pub fn preview_interactive_bom(
    source_path: &Path,
    companion_path: Option<&Path>,
) -> Result<NormalizedBomDto, CacheError> {
    let (bytes, original_name) = read_source(source_path, ProjectKind::Interactive)?;
    parse_interactive_source(&bytes, &original_name, companion_path)
        .map(|(normalized, _)| normalized)
}

/// Read a source file, rejecting oversized interactive exports before they
/// reach memory: callers hold the database lock for the whole command.
fn read_source(source_path: &Path, kind: ProjectKind) -> Result<(Vec<u8>, String), CacheError> {
    let size = fs::metadata(source_path)?.len();
    if kind == ProjectKind::Interactive && size > MAX_INTERACTIVE_BOM_BYTES as u64 {
        return Err(CacheError::Parse(InteractiveHtmlError::TooLarge(
            size as usize,
        )));
    }
    let bytes = fs::read(source_path)?;
    let original_name = source_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_owned();
    Ok((bytes, original_name))
}

fn parse_interactive_source(
    bytes: &[u8],
    original_name: &str,
    companion_path: Option<&Path>,
) -> Result<(NormalizedBomDto, Option<NormalizedBomDto>), CacheError> {
    let source_text = std::str::from_utf8(bytes).map_err(|error| {
        CacheError::Parse(InteractiveHtmlError::UnsupportedInteractiveBom(
            error.to_string(),
        ))
    })?;
    let companion = companion_path
        .map(|path| {
            parse_tabular_bom(path, None).map_err(|error| CacheError::Companion(error.to_string()))
        })
        .transpose()?;
    let normalized = parse_interactive_html_text_with_companion(
        source_text,
        original_name.to_owned(),
        companion.as_ref(),
    )?;
    Ok((normalized, companion))
}

/// Point the single active welding session at `project_id`, cancelling any
/// other active session so exactly one project drives the welding workspace.
fn activate_session(tx: &rusqlite::Transaction<'_>, project_id: &str) -> rusqlite::Result<String> {
    let session_id = tx
        .query_row(
            "SELECT id FROM welding_sessions WHERE project_id = ?1 AND status = 'active' ORDER BY updated_at DESC LIMIT 1",
            [project_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .unwrap_or_else(new_id);
    tx.execute(
        "UPDATE welding_sessions SET status = 'cancelled', updated_at = ?1 WHERE status = 'active' AND id <> ?2",
        rusqlite::params![crate::db::utc_now(), session_id],
    )?;
    let exists = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM welding_sessions WHERE id = ?1)",
        [&session_id],
        |row| row.get::<_, bool>(0),
    )?;
    if exists {
        tx.execute(
            "UPDATE welding_sessions SET status = 'active', updated_at = ?1 WHERE id = ?2",
            rusqlite::params![crate::db::utc_now(), session_id],
        )?;
    } else {
        tx.execute(
            "INSERT INTO welding_sessions (id, project_id, status) VALUES (?1, ?2, 'active')",
            rusqlite::params![session_id, project_id],
        )?;
    }
    Ok(session_id)
}

fn validate_cache_path(cache_dir: &Path, cache_name: &str) -> Result<PathBuf, CacheError> {
    let valid_name = cache_name.len() == 69
        && cache_name.ends_with(".html")
        && cache_name[..64]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        && !cache_name.contains('/')
        && !cache_name.contains('\\');
    if !valid_name {
        return Err(CacheError::TamperedCache(
            "cache name must be a lowercase sha256 .html filename".into(),
        ));
    }
    fs::create_dir_all(cache_dir)?;
    let root = fs::canonicalize(cache_dir)?;
    let target = root.join(cache_name);
    if let Ok(existing) = fs::canonicalize(&target) {
        if !existing.starts_with(&root) {
            return Err(CacheError::TamperedCache(
                "cache path escapes cache directory".into(),
            ));
        }
    }
    if target.parent() != Some(root.as_path()) {
        return Err(CacheError::TamperedCache(
            "cache path escapes cache directory".into(),
        ));
    }
    Ok(target)
}

struct TempFileGuard(PathBuf);

impl Drop for TempFileGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn prepare_temp_with<F>(temp: &Path, contents: &[u8], writer: F) -> std::io::Result<TempFileGuard>
where
    F: FnOnce(&Path, &[u8]) -> std::io::Result<()>,
{
    let guard = TempFileGuard(temp.to_owned());
    writer(temp, contents)?;
    Ok(guard)
}

#[cfg(not(windows))]
fn atomic_replace(temp: &Path, target: &Path) -> std::io::Result<()> {
    fs::rename(temp, target)
}

#[cfg(windows)]
fn atomic_replace(temp: &Path, target: &Path) -> std::io::Result<()> {
    use std::{iter, os::windows::ffi::OsStrExt};
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };
    let temp = temp
        .as_os_str()
        .encode_wide()
        .chain(iter::once(0))
        .collect::<Vec<_>>();
    let target = target
        .as_os_str()
        .encode_wide()
        .chain(iter::once(0))
        .collect::<Vec<_>>();
    let replaced = unsafe {
        MoveFileExW(
            temp.as_ptr(),
            target.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if replaced == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[derive(Serialize)]
struct BridgeBootstrap<'a> {
    token: &'a str,
    designators: Vec<&'a str>,
}

fn bindings(
    normalized: &NormalizedBomDto,
) -> Result<BTreeMap<String, DesignatorBinding>, CacheError> {
    let mut result = BTreeMap::new();
    for group in &normalized.groups {
        for placement in &group.placements {
            result.insert(
                placement.designator.clone(),
                DesignatorBinding {
                    component_key: group.component_key.clone(),
                    side: placement.side.clone(),
                },
            );
        }
        // Tabular BOMs without a board-side column list designators on the
        // group only; the operator declares the side when taking parts.
        for designator in &group.designators {
            result
                .entry(designator.clone())
                .or_insert_with(|| DesignatorBinding {
                    component_key: group.component_key.clone(),
                    side: None,
                });
        }
    }
    Ok(result)
}

fn build_cached_html(
    source: &[u8],
    token: &str,
    designators: &BTreeMap<String, DesignatorBinding>,
    companion: Option<&NormalizedBomDto>,
) -> Result<Vec<u8>, CacheError> {
    let names = designators.keys().map(String::as_str).collect::<Vec<_>>();
    let bootstrap = serde_json::to_string(&BridgeBootstrap {
        token,
        designators: names,
    })
    .map_err(|error| CacheError::TamperedCache(error.to_string()))?
    .replace('<', "\\u003c")
    .replace('>', "\\u003e")
    .replace('&', "\\u0026");
    let mut cached = Vec::with_capacity(source.len() + BRIDGE_JS.len() + bootstrap.len() + 256);
    cached.extend_from_slice(source);
    cached.extend_from_slice(b"\n<!-- partnest bridge-v1 -->\n<script data-partnest-bridge=\"bridge-v1\">window.__PARTNEST_BOM_BRIDGE_V1__=");
    cached.extend_from_slice(bootstrap.as_bytes());
    cached.extend_from_slice(b";</script><script data-partnest-bridge=\"bridge-v1\">\n");
    cached.extend_from_slice(BRIDGE_JS.as_bytes());
    cached.extend_from_slice(b"\n</script>\n");
    if let Some(companion) = companion {
        let companion = serde_json::to_string(companion)
            .map_err(|error| CacheError::TamperedCache(error.to_string()))?
            .replace('<', "\\u003c")
            .replace('>', "\\u003e")
            .replace('&', "\\u0026");
        cached.extend_from_slice(COMPANION_START);
        cached.extend_from_slice(companion.as_bytes());
        cached.extend_from_slice(COMPANION_END);
        cached.extend_from_slice(b"\n");
    }
    Ok(cached)
}

fn read_cached_companion(cached_tail: &[u8]) -> Result<Option<NormalizedBomDto>, CacheError> {
    let Some(start) = cached_tail
        .windows(COMPANION_START.len())
        .position(|window| window == COMPANION_START)
    else {
        return Ok(None);
    };
    let content_start = start + COMPANION_START.len();
    let end = cached_tail[content_start..]
        .windows(COMPANION_END.len())
        .position(|window| window == COMPANION_END)
        .ok_or_else(|| CacheError::TamperedCache("companion CSV marker is incomplete".into()))?;
    serde_json::from_slice(&cached_tail[content_start..content_start + end])
        .map(Some)
        .map_err(|error| CacheError::TamperedCache(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::atomic_replace;
    use super::prepare_temp_with;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn atomic_replace_overwrites_existing_target_without_predelete() {
        let directory = tempdir().unwrap();
        let target = directory.path().join("target.html");
        let temp = directory.path().join("target.tmp");
        fs::write(&target, "valid-old").unwrap();
        fs::write(&temp, "valid-new").unwrap();
        atomic_replace(&temp, &target).unwrap();
        assert_eq!(fs::read_to_string(&target).unwrap(), "valid-new");
        assert!(!temp.exists());
    }

    #[test]
    fn temp_guard_removes_partial_file_when_writer_fails() {
        let directory = tempdir().unwrap();
        let temp = directory.path().join("partial.tmp");
        let result = prepare_temp_with(&temp, b"expected", |path, _| {
            fs::write(path, b"partial")?;
            Err(std::io::Error::other("injected write failure"))
        });
        assert!(result.is_err());
        assert!(!temp.exists());
    }
}
