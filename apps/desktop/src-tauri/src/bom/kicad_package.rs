//! Read-only KiCad archive import. Source files are copied byte-for-byte;
//! schematic interpretation is delegated to KiCad's BOM exporter.
use super::{
    cache::ImportedProject,
    projects::{self, ProjectKind},
    tabular::inspect_tabular_bom,
    types::{FieldMapping, ImportPreview, NormalizedBomDto},
};
use crate::db::Database;
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs,
    io::{Cursor, Read},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

const MAX_ARCHIVE: u64 = 128 * 1024 * 1024;
const MAX_EXPANDED: u64 = 256 * 1024 * 1024;
const MAX_FILE: u64 = 32 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageFile {
    pub path: String,
    pub kind: String,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageManifest {
    pub files: Vec<PackageFile>,
    pub bom_source: String,
    pub warnings: Vec<String>,
    pub front_svg: Option<String>,
    pub back_svg: Option<String>,
    pub positions: Vec<ComponentPosition>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentPosition {
    pub reference: String,
    pub value: String,
    pub package: String,
    pub x_mm: f64,
    pub y_mm: f64,
    pub rotation: f64,
    pub side: String,
}

pub struct PreparedPackage {
    path: PathBuf,
    bytes: Vec<u8>,
    normalized: NormalizedBomDto,
    manifest: PackageManifest,
}

fn extension(path: &Path) -> String {
    path.extension()
        .and_then(|v| v.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
}

fn file_kind(path: &Path) -> &'static str {
    match extension(path).as_str() {
        "kicad_pro" => "工程",
        "kicad_sch" => "原理图",
        "kicad_pcb" => "PCB",
        "gbr" | "ger" | "gtl" | "gbl" | "gts" | "gbs" | "gto" | "gbo" | "gm1" | "gko" => "Gerber",
        "gbrjob" => "Gerber 作业",
        "drl" | "xln" => "钻孔",
        "csv" | "xls" | "xlsx" => "表格",
        "pos" => "贴片坐标",
        "step" | "stp" | "wrl" => "3D 模型",
        "pdf" => "文档",
        _ => "其他",
    }
}

fn unpack(bytes: &[u8], destination: &Path) -> Result<Vec<PackageFile>, String> {
    let mut archive =
        zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| format!("无法读取 ZIP：{e}"))?;
    if archive.len() > 2048 {
        return Err("ZIP 文件数量超过 2048 个".into());
    }
    let mut total = 0u64;
    let mut names = BTreeSet::new();
    let mut files = Vec::new();
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|e| format!("无法读取压缩项：{e}"))?;
        let raw = entry.name().replace('\\', "/");
        if raw.starts_with('/')
            || raw
                .split('/')
                .any(|part| part == ".." || part.contains(':'))
            || entry.is_symlink()
        {
            return Err("ZIP 含不安全的路径或符号链接".into());
        }
        let relative = entry.enclosed_name().ok_or("ZIP 含不安全的路径")?;
        if entry.is_dir() {
            continue;
        }
        total = total.checked_add(entry.size()).ok_or("ZIP 解压体积过大")?;
        if entry.size() > MAX_FILE || total > MAX_EXPANDED {
            return Err("ZIP 解压体积超限（单文件 32 MiB / 总计 256 MiB）".into());
        }
        let key = raw.to_lowercase();
        if !names.insert(key) {
            return Err("ZIP 含重复文件路径".into());
        }
        let mut content = Vec::new();
        entry
            .by_ref()
            .take(MAX_FILE + 1)
            .read_to_end(&mut content)
            .map_err(|e| e.to_string())?;
        if content.len() as u64 != entry.size() {
            return Err("ZIP 文件大小不符或超限".into());
        }
        // Only design sources and tables are needed by the exporter. Gerbers,
        // libraries and other attachments remain safely inside the stored ZIP.
        if matches!(
            extension(&relative).as_str(),
            "kicad_pro"
                | "kicad_sch"
                | "kicad_pcb"
                | "csv"
                | "xls"
                | "xlsx"
                | "step"
                | "stp"
                | "wrl"
        ) {
            let output = destination.join(&relative);
            if let Some(parent) = output.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            fs::write(output, content).map_err(|e| e.to_string())?;
        }
        files.push(PackageFile {
            path: raw,
            kind: file_kind(&relative).into(),
            size: entry.size(),
        });
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
}

fn cli_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("KICAD_CLI") {
        return Some(path.into());
    }
    if let Some(paths) = std::env::var_os("PATH") {
        for directory in std::env::split_paths(&paths) {
            for name in ["kicad-cli.exe", "kicad-cli"] {
                let path = directory.join(name);
                if path.is_file() {
                    return Some(path);
                }
            }
        }
    }
    let base = PathBuf::from(std::env::var_os("ProgramFiles")?).join("KiCad");
    for version in ["10.0", "9.0", "8.0"] {
        let path = base.join(version).join("bin/kicad-cli.exe");
        if path.is_file() {
            return Some(path);
        }
    }
    None
}

pub fn export_model(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let directory = tempfile::tempdir().map_err(|e| e.to_string())?;
    let files = unpack(bytes, directory.path())?;
    let project = primary_file(&files, "工程", None)?;
    let expected = project.map(|p| Path::new(&p.path).with_extension("kicad_pcb"));
    let board = primary_file(&files, "PCB", expected.as_deref())?.ok_or("工程包未包含 PCB 文件")?;
    let cli = cli_path().ok_or("未找到 kicad-cli，请安装 KiCad 10 或更新版以生成 3D 预览")?;
    let output = directory.path().join("partnest-model.glb");
    let mut command = Command::new(cli);
    command
        .args([
            "pcb",
            "export",
            "glb",
            "--subst-models",
            "--include-pads",
            "--include-silkscreen",
            "--include-soldermask",
            "--output",
        ])
        .arg(&output)
        .arg(directory.path().join(&board.path))
        .current_dir(directory.path());
    run_cli(&mut command)
        .map_err(|e| format!("KiCad 3D 模型导出失败（需要 KiCad 10+ 和工程中的模型资源）：{e}"))?;
    if fs::metadata(&output).map_err(|e| e.to_string())?.len() > 64 * 1024 * 1024 {
        return Err("3D 模型超过 64 MiB".into());
    }
    let bytes = fs::read(output).map_err(|e| e.to_string())?;
    if !bytes.starts_with(b"glTF") {
        return Err("KiCad 未生成有效 GLB 模型".into());
    }
    Ok(bytes)
}

fn export_bom(root: &Path, schematic: &Path) -> Result<PathBuf, String> {
    let cli = cli_path().ok_or(
        "压缩包没有 BOM 表格。请安装 KiCad 8 或更新版以从原理图导出 BOM，或在 ZIP 中加入 BOM 表格",
    )?;
    let output = root.join("partnest-export.csv");
    let mut command = Command::new(cli);
    command
        .args([
            "sch",
            "export",
            "bom",
            "--fields",
            "Reference,Value,Footprint,QUANTITY,MPN,Manufacturer,LCSC",
            "--labels",
            "designators,value,package,quantity,mpn,manufacturer,lcsc_code",
            "--ref-range-delimiter",
            "",
            "--exclude-dnp",
            "--output",
        ])
        .arg(&output)
        .arg(schematic)
        .current_dir(root);
    run_cli(&mut command)?;
    Ok(output)
}

fn run_cli(command: &mut Command) -> Result<(), String> {
    let errors = tempfile::tempfile().map_err(|e| e.to_string())?;
    command.stdout(Stdio::null()).stderr(errors);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command
        .spawn()
        .map_err(|e| format!("无法启动 KiCad BOM 导出：{e}"))?;
    let started = Instant::now();
    loop {
        match child.try_wait().map_err(|e| e.to_string())? {
            Some(status) if status.success() => return Ok(()),
            Some(_) => return Err("KiCad 无法从原理图导出 BOM，请检查工程完整性和 KiCad 版本，或在 ZIP 中加入 BOM 表格".into()),
            None if started.elapsed() > Duration::from_secs(60) => {
                let _ = child.kill(); let _ = child.wait();
                return Err("KiCad BOM 导出超时".into());
            }
            None => std::thread::sleep(Duration::from_millis(100)),
        }
    }
}

fn export_board(root: &Path, board: &Path, manifest: &mut PackageManifest) -> Result<(), String> {
    let cli = cli_path().ok_or("未找到 kicad-cli，无法生成 PCB 预览和器件坐标")?;
    for (side, layers) in [
        ("front", "F.Cu,F.SilkS,F.Fab,Edge.Cuts"),
        ("back", "B.Cu,B.SilkS,B.Fab,Edge.Cuts"),
    ] {
        let output = root.join(format!("partnest-{side}.svg"));
        let mut command = Command::new(&cli);
        command
            .args([
                "pcb",
                "export",
                "svg",
                "--layers",
                layers,
                "--page-size-mode",
                "2",
                "--mode-single",
                "--exclude-drawing-sheet",
                "--output",
            ])
            .arg(&output)
            .arg(board)
            .current_dir(root);
        run_cli(&mut command)?;
        if fs::metadata(&output).map_err(|e| e.to_string())?.len() > 16 * 1024 * 1024 {
            return Err("PCB 预览过大".into());
        }
        let svg = fs::read_to_string(output).map_err(|e| e.to_string())?;
        if side == "front" {
            manifest.front_svg = Some(svg);
        } else {
            manifest.back_svg = Some(svg);
        }
    }
    let output = root.join("partnest-positions.csv");
    let mut command = Command::new(cli);
    command
        .args([
            "pcb", "export", "pos", "--format", "csv", "--units", "mm", "--side", "both",
            "--output",
        ])
        .arg(&output)
        .arg(board)
        .current_dir(root);
    run_cli(&mut command)?;
    let mut reader = csv::Reader::from_path(output).map_err(|e| e.to_string())?;
    let headers = reader.headers().map_err(|e| e.to_string())?.clone();
    for row in reader.records() {
        let row = row.map_err(|e| e.to_string())?;
        let value = |header: &str| -> Result<&str, String> {
            headers
                .iter()
                .position(|h| h.eq_ignore_ascii_case(header))
                .and_then(|i| row.get(i))
                .ok_or_else(|| format!("坐标文件缺少列 {header}"))
        };
        let number = |header: &str| -> Result<f64, String> {
            let n: f64 = value(header)?.parse().map_err(|_| "坐标数字无效")?;
            if n.is_finite() {
                Ok(n)
            } else {
                Err("坐标数字无效".into())
            }
        };
        manifest.positions.push(ComponentPosition {
            reference: value("Ref")?.into(),
            value: value("Val")?.into(),
            package: value("Package")?.into(),
            x_mm: number("PosX")?,
            y_mm: number("PosY")?,
            rotation: number("Rot")?,
            side: value("Side")?.into(),
        });
    }
    Ok(())
}

fn primary_file<'a>(
    files: &'a [PackageFile],
    kind: &str,
    expected: Option<&Path>,
) -> Result<Option<&'a PackageFile>, String> {
    if let Some(expected) = expected {
        if let Some(file) = files
            .iter()
            .find(|file| file.kind == kind && Path::new(&file.path) == expected)
        {
            return Ok(Some(file));
        }
    }
    let candidates: Vec<_> = files.iter().filter(|file| file.kind == kind).collect();
    let shallowest = candidates
        .iter()
        .map(|file| Path::new(&file.path).components().count())
        .min();
    let mut main = candidates
        .into_iter()
        .filter(|file| Some(Path::new(&file.path).components().count()) == shallowest);
    let selected = main.next();
    if main.next().is_some() {
        return Err(format!(
            "ZIP 同一目录层级包含多个{kind}，无法确定主工程，请每次导入一个工程包"
        ));
    }
    Ok(selected)
}

pub fn prepare(path: &Path, mapping: Option<&FieldMapping>) -> Result<PreparedPackage, String> {
    let file = fs::File::open(path).map_err(|e| e.to_string())?;
    if file.metadata().map_err(|e| e.to_string())?.len() > MAX_ARCHIVE {
        return Err("ZIP 超过 128 MiB".into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_ARCHIVE + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_ARCHIVE {
        return Err("ZIP 超过 128 MiB".into());
    }
    let directory = tempfile::tempdir().map_err(|e| e.to_string())?;
    let files = unpack(&bytes, directory.path())?;
    let project = primary_file(&files, "工程", None)?;
    let expected_board = project.map(|p| Path::new(&p.path).with_extension("kicad_pcb"));
    let board = primary_file(&files, "PCB", expected_board.as_deref())?
        .map(|file| directory.path().join(&file.path));
    if !files
        .iter()
        .any(|f| matches!(f.kind.as_str(), "工程" | "PCB" | "原理图"))
    {
        return Err("ZIP 中未找到 KiCad 工程、PCB 或原理图文件".into());
    }
    let tables: Vec<_> = files
        .iter()
        .filter(|f| {
            f.kind == "表格"
                && Path::new(&f.path)
                    .file_stem()
                    .and_then(|v| v.to_str())
                    .unwrap_or_default()
                    .to_lowercase()
                    .contains("bom")
        })
        .collect();
    if tables.len() > 1 {
        return Err("ZIP 含多份 BOM 表格，请只保留该工程使用的一份 BOM".into());
    }
    let (bom_path, bom_source) = if let Some(table) = tables.first() {
        (directory.path().join(&table.path), table.path.clone())
    } else {
        let expected = project.map(|p| Path::new(&p.path).with_extension("kicad_sch"));
        let main = primary_file(&files, "原理图", expected.as_deref())?
        .ok_or("无法确定主原理图。请包含与工程同名的原理图，或在 ZIP 中加入一份文件名含 BOM 的器件表格")?;
        let output = export_bom(directory.path(), &directory.path().join(&main.path))?;
        (output, format!("{}（KiCad 导出）", main.path))
    };
    let mut normalized = match inspect_tabular_bom(&bom_path, mapping).map_err(|e| e.to_string())? {
        ImportPreview::Ready(bom) => bom,
        ImportPreview::NeedsMapping { .. } => return Err("ZIP 内 BOM 列名无法识别，请使用 quantity、designators、value、package、lcsc_code 等标准列名，或单独导入表格完成映射".into()),
    };
    if normalized.groups.is_empty() {
        return Err("工程包中没有可用于分析的 BOM 器件".into());
    }
    normalized.source_name = path
        .file_name()
        .and_then(|v| v.to_str())
        .unwrap_or("KiCad.zip")
        .into();
    let selected_project = project.map(|p| p.path.clone());
    let has_copies = files.iter().filter(|f| f.kind == "工程").count() > 1;
    let mut manifest = PackageManifest {
        files,
        bom_source,
        warnings: Vec::new(),
        front_svg: None,
        back_svg: None,
        positions: Vec::new(),
    };
    if has_copies {
        manifest.warnings.push(format!(
            "包内包含工程副本，已使用主工程 {}；其余文件随原始 ZIP 保留。",
            selected_project.unwrap_or_default()
        ));
    }
    if let Some(board) = board {
        if let Err(error) = export_board(directory.path(), &board, &mut manifest) {
            manifest.warnings.push(error);
        }
    } else {
        manifest
            .warnings
            .push("包内未包含 PCB 文件，无法提供板图和器件位置".into());
    }
    apply_positions(&mut normalized, &manifest.positions);
    Ok(PreparedPackage {
        path: path.into(),
        bytes,
        normalized,
        manifest,
    })
}

/// Complete board-side bindings from exported PCB placements, including old snapshots.
pub fn apply_positions(normalized: &mut NormalizedBomDto, positions: &[ComponentPosition]) {
    for group in &mut normalized.groups {
        for designator in &group.designators {
            let side = positions
                .iter()
                .find(|p| p.reference == *designator)
                .and_then(|p| match p.side.as_str() {
                    "top" | "front" => Some(super::types::BomSide::Top),
                    "bottom" | "back" => Some(super::types::BomSide::Bottom),
                    _ => None,
                });
            if let Some(side) = side {
                if let Some(placement) = group
                    .placements
                    .iter_mut()
                    .find(|p| p.designator == *designator)
                {
                    if placement.side.is_none() {
                        placement.side = Some(side);
                    }
                } else {
                    group.placements.push(super::types::BomPlacementDto {
                        designator: designator.clone(),
                        side: Some(side),
                        component_key: group.component_key.clone(),
                    });
                }
            }
        }
    }
}

pub fn commit(
    db: &Database,
    prepared: PreparedPackage,
    name: Option<&str>,
) -> Result<ImportedProject, String> {
    let tx = db.transaction().map_err(|e| e.to_string())?;
    let digest = hex::encode(Sha256::digest(&prepared.bytes));
    let record = projects::record_project_with_digest(
        db,
        &prepared.path,
        &digest,
        name,
        ProjectKind::Tabular,
        true,
        &prepared.normalized,
    )
    .map_err(|e| e.user_message())?;
    let manifest = serde_json::to_string(&prepared.manifest).map_err(|e| e.to_string())?;
    db.connection()
        .execute(
            "DELETE FROM project_models WHERE project_id=?1",
            [&record.id],
        )
        .map_err(|e| e.to_string())?;
    db.connection().execute("INSERT INTO project_packages (project_id, archive, manifest_json) VALUES (?1, ?2, ?3) ON CONFLICT(project_id) DO UPDATE SET archive=excluded.archive, manifest_json=excluded.manifest_json", rusqlite::params![record.id, prepared.bytes, manifest]).map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(ImportedProject {
        project_id: record.id,
        name: record.name,
        original_name: record.original_name,
        cache_name: record.cache_name,
        kind: record.kind,
        has_table: true,
        normalized: prepared.normalized,
    })
}

pub fn manifest(db: &Database, id: &str) -> Result<Option<PackageManifest>, String> {
    let json: Option<String> = db
        .connection()
        .query_row(
            "SELECT manifest_json FROM project_packages WHERE project_id=?1",
            [id],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    json.map(|value| serde_json::from_str(&value).map_err(|e| e.to_string()))
        .transpose()
}
