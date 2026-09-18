import { invoke } from "@tauri-apps/api/core";
import type { BomGroup, InventoryPart, MatchResult } from "@partnest/domain";
import { matchBomGroup } from "@partnest/domain";
import { desktopApi, type ImportedProject, type Part, type ProjectSummary } from "../../app/tauri";
import { useCallback, useRef, useState } from "react";

export type FieldName = "quantity" | "designators" | "name" | "value" | "package" | "manufacturer" | "mpn" | "lcsc_code" | "side";
export type FieldMapping = Partial<Record<FieldName, string>>;
export type BomGroupDto = {
  component_key?: string;
  componentKey?: string;
  name: string;
  value: string;
  package: string;
  manufacturer?: string;
  mpn?: string;
  lcsc_code?: string;
  lcscCode?: string;
  quantity: number;
  designators?: string[];
  placements?: Array<{ designator: string; side: string | null }>;
  extra_fields?: Record<string, string>;
  extraFields?: Record<string, string>;
};
export type NormalizedBomDto = { source_name: string; groups: BomGroupDto[] };
export type ImportPreview =
  | { kind: "Ready"; bom: NormalizedBomDto }
  | { kind: "NeedsMapping"; headers: string[]; suggestions: FieldMapping }
  | { kind: "Unsupported"; message?: string };

export type ProjectImportApi = {
  inspectTabularBom: (sourcePath: string, mapping?: FieldMapping) => Promise<unknown>;
  /** Read-only analysis of an interactive BOM: nothing is stored or switched. */
  previewInteractiveBom: (sourcePath: string, companionPath?: string) => Promise<unknown>;
  /** Store the analysed file as a project; the only write this page makes. */
  importProject: (input: { source_path: string; name?: string; mapping?: FieldMapping; companion_path?: string }) => Promise<ImportedProject>;
  /** Merge the source an existing project is still missing. */
  supplementProject: (input: { project_id: string; source_path: string; mapping?: FieldMapping }) => Promise<ImportedProject>;
  listParts: () => Promise<Part[]>;
  listProjects?: () => Promise<ProjectSummary[]>;
  analyzeProject?: (id: string) => Promise<unknown>;
  renameProject?: (id: string, name: string) => Promise<ProjectSummary>;
  removeProject?: (id: string) => Promise<void>;
  openProjectWelding?: (id: string) => Promise<unknown>;
};

export type BomAnalysisRow = {
  componentKey: string;
  name: string;
  value: string;
  package: string;
  required: number;
  stock: number;
  shortage: number;
  match: MatchResult;
  status: "exact" | "candidate" | "none";
  confirmed?: boolean;
  partId?: string;
  boxSlot?: string;
  candidateIds?: string[];
};

const supported = new Set([".html", ".csv", ".xlsx"]);
const fieldNames: FieldName[] = ["quantity", "designators", "name", "value", "package", "manufacturer", "mpn", "lcsc_code", "side"];

const defaultApi: ProjectImportApi = {
  inspectTabularBom: (sourcePath, mapping) => invoke("inspect_tabular_bom", { sourcePath, mapping }),
  previewInteractiveBom: (sourcePath, companionPath) => invoke("preview_interactive_bom", { sourcePath, companionPath }),
  importProject: (input) => desktopApi.importProject(input),
  supplementProject: (input) => desktopApi.supplementProject(input),
  listParts: () => desktopApi.listParts(),
  listProjects: () => desktopApi.listProjects(),
  analyzeProject: (id) => desktopApi.analyzeProject(id),
  renameProject: (id, name) => desktopApi.renameProject(id, name),
  removeProject: (id) => desktopApi.removeProject(id),
  openProjectWelding: (id) => desktopApi.openProjectWelding(id),
};


type PreviewPayload = {
  kind?: "Ready" | "NeedsMapping";
  bom?: NormalizedBomDto;
  normalized?: NormalizedBomDto;
  Ready?: NormalizedBomDto;
  NeedsMapping?: { headers: string[]; suggestions: FieldMapping };
  headers?: string[];
  suggestions?: FieldMapping;
  message?: string;
};

function toPreview(value: unknown): ImportPreview {
  const payload = (value && typeof value === "object" ? value : {}) as PreviewPayload;
  if (payload.kind === "Ready" && payload.bom) return { kind: "Ready", bom: payload.bom };
  if (payload.kind === "NeedsMapping" && payload.headers && payload.suggestions) return { kind: "NeedsMapping", headers: payload.headers, suggestions: payload.suggestions };
  if (payload.normalized) return { kind: "Ready", bom: payload.normalized };
  if (payload.Ready) return { kind: "Ready", bom: payload.Ready };
  if (payload.NeedsMapping) return { kind: "NeedsMapping", ...payload.NeedsMapping };
  return { kind: "Unsupported", message: payload.message };
}

function normalizedGroup(group: BomGroupDto): BomGroup {
  const componentKey = group.componentKey ?? group.component_key ?? "";
  return {
    componentKey,
    name: group.name ?? "",
    value: group.value ?? "",
    package: group.package ?? "",
    manufacturer: group.manufacturer ?? "",
    mpn: group.mpn ?? "",
    lcscCode: group.lcscCode ?? group.lcsc_code ?? "",
    placements: (group.placements ?? []).flatMap((placement) =>
      placement.side === "top" || placement.side === "bottom"
        ? [{ designator: placement.designator, side: placement.side, componentKey }]
        : []),
    extraFields: group.extraFields ?? group.extra_fields ?? {},
  };
}

function partAsInventory(part: Part): InventoryPart {
  return { id: part.id, name: part.name, package: part.package ?? "", mpn: part.mpn ?? "", lcscCode: part.lcsc_code ?? "", value: part.name };
}

export function createAnalysisRows(bom: NormalizedBomDto, parts: Part[], confirmed: Record<string, string> = {}): BomAnalysisRow[] {
  const inventory = parts.map(partAsInventory);
  const byId = new Map(parts.map((part) => [part.id, part]));
  return bom.groups.map((group) => {
    const key = group.componentKey ?? group.component_key ?? `${group.name}|${group.value}|${group.package}`;
    const selected = confirmed[key];
    const rawMatch = selected ? { kind: "exact-mpn" as const, partId: selected } : matchBomGroup(normalizedGroup(group), inventory);
    const match = rawMatch;
    const partId = match.kind === "exact-lcsc" || match.kind === "exact-mpn" ? match.partId : undefined;
    const part = partId ? byId.get(partId) : undefined;
    const required = Math.max(Number(group.quantity) || 0, 0);
    const stock = part?.quantity ?? 0;
    return {
      componentKey: key, name: group.name, value: group.value, package: group.package,
      required, stock, shortage: Math.max(required - stock, 0), match,
      status: match.kind === "candidate" ? "candidate" : match.kind === "none" ? "none" : "exact", confirmed: Boolean(selected),
      partId, boxSlot: part?.slot ?? undefined, candidateIds: match.kind === "candidate" ? match.partIds : undefined,
    };
  });
}

export function useProjectImport({ api = defaultApi }: { api?: ProjectImportApi } = {}) {
  const [status, setStatus] = useState<"idle" | "loading" | "ready" | "imported" | "needsMapping" | "error">("idle");
  const [error, setError] = useState("");
  const [path, setPath] = useState("");
  const [companionPath, setCompanionPath] = useState("");
  const [headers, setHeaders] = useState<string[]>([]);
  const [mapping, setMapping] = useState<FieldMapping>({});
  const [bom, setBom] = useState<NormalizedBomDto | null>(null);
  const [parts, setParts] = useState<Part[]>([]);
  const [rows, setRows] = useState<BomAnalysisRow[]>([]);
  const [project, setProject] = useState<{ id: string; name: string } | null>(null);
  /** The previous analysis, restored when the operator cancels the mapping dialog. */
  const pendingImportRef = useRef<{
    status: typeof status;
    error: string;
    path: string;
    companionPath: string;
    headers: string[];
    mapping: FieldMapping;
    bom: NormalizedBomDto | null;
    parts: Part[];
    rows: BomAnalysisRow[];
  } | null>(null);
  /** 「导入项目」continues through the mapping dialog; cancelling stops before any write. */
  const autoImportRef = useRef(false);

  const showAnalysis = useCallback((next: { bom: NormalizedBomDto; rows: BomAnalysisRow[]; parts: Part[] }) => {
    setBom(next.bom); setParts(next.parts); setRows(next.rows); setStatus("ready");
    pendingImportRef.current = null;
  }, []);

  /** What the confirmed file is for: a new project, or one that needs a source. */
  const targetRef = useRef<{ mode: "import" } | { mode: "supplement"; projectId: string }>({ mode: "import" });

  /** Store the confirmed file, either as a new project or merged into one. */
  const finishImport = useCallback(async (sourcePath: string, supplied?: FieldMapping, suppliedCompanionPath = companionPath) => {
    setStatus("loading"); setError("");
    const target = targetRef.current;
    try {
      const imported = target.mode === "import"
        ? await api.importProject({
          source_path: sourcePath,
          ...(supplied ? { mapping: supplied } : {}),
          ...(suppliedCompanionPath ? { companion_path: suppliedCompanionPath } : {}),
        })
        : await api.supplementProject({
          project_id: target.projectId,
          source_path: sourcePath,
          ...(supplied ? { mapping: supplied } : {}),
        });
      const listed = await api.listParts();
      setBom(imported.normalized); setParts(listed); setRows(createAnalysisRows(imported.normalized, listed));
      setProject({ id: imported.project_id, name: imported.name });
      setStatus("imported");
      targetRef.current = { mode: "import" };
      return true;
    } catch (cause) { setStatus("error"); setError(cause instanceof Error ? cause.message : String(cause)); return false; }
  }, [api, companionPath]);

  /** Analyse a picked file. Previewing stores nothing and never switches sessions. */
  const inspect = useCallback(async (sourcePath: string, supplied?: FieldMapping, suppliedCompanionPath = companionPath, thenImport = false) => {
    const extension = sourcePath.slice(sourcePath.lastIndexOf(".")).toLowerCase();
    if (!supported.has(extension)) { setStatus("error"); setError("不支持的 BOM 格式，请选择 HTML、CSV 或 XLSX 文件"); return false; }
    setPath(sourcePath); setError(""); setStatus("loading"); setProject(null);
    try {
      const result = extension === ".html"
        ? toPreview(await (suppliedCompanionPath
          ? api.previewInteractiveBom(sourcePath, suppliedCompanionPath)
          : api.previewInteractiveBom(sourcePath)))
        : toPreview(await api.inspectTabularBom(sourcePath, supplied));
      if (result.kind === "NeedsMapping") {
        setHeaders(result.headers); setMapping(result.suggestions ?? {}); setStatus("needsMapping"); return false;
      }
      if (result.kind !== "Ready") { setStatus("error"); setError(result.message ?? "BOM 解析失败"); return false; }
      const listed = await api.listParts();
      showAnalysis({ bom: result.bom, parts: listed, rows: createAnalysisRows(result.bom, listed) });
      // A file the operator picked to import is stored as soon as it parses.
      return thenImport ? finishImport(sourcePath, supplied, suppliedCompanionPath) : true;
    } catch (cause) { setStatus("error"); setError(cause instanceof Error ? cause.message : String(cause)); return false; }
  }, [api, companionPath, finishImport, showAnalysis]);

  /** Open a stored project's analysis; the original file is never needed again. */
  const loadProject = useCallback(async (summary: ProjectSummary) => {
    if (!api.analyzeProject) return;
    autoImportRef.current = false;
    setStatus("loading"); setError(""); setPath(""); setCompanionPath("");
    try {
      const result = toPreview(await api.analyzeProject(summary.id));
      if (result.kind !== "Ready") { setStatus("error"); setError(result.kind === "Unsupported" ? result.message ?? "无法读取该项目的解析结果" : "该项目的解析结果需要重新映射，请重新导入原始文件"); return; }
      const listed = await api.listParts();
      setProject({ id: summary.id, name: summary.name });
      showAnalysis({ bom: result.bom, parts: listed, rows: createAnalysisRows(result.bom, listed) });
    } catch (cause) { setStatus("error"); setError(cause instanceof Error ? cause.message : String(cause)); }
  }, [api, showAnalysis]);

  /** Make a project the welding session; the workspace navigates on success. */
  const openWelding = useCallback(async (projectId: string): Promise<boolean> => {
    if (!api.openProjectWelding) return false;
    setError("");
    try {
      await api.openProjectWelding(projectId);
      return true;
    } catch (cause) { setError(cause instanceof Error ? cause.message : String(cause)); return false; }
  }, [api]);

  /** Drop the loaded analysis, for example after its project is deleted. */
  const clearLoaded = useCallback(() => {
    autoImportRef.current = false;
    targetRef.current = { mode: "import" };
    setStatus("idle"); setError(""); setPath(""); setCompanionPath(""); setBom(null); setParts([]); setRows([]); setProject(null);
  }, []);

  /** Import what the dialog confirmed: analyse the file, then store it as a project. */
  const importFrom = useCallback(async (sourcePath: string, suppliedCompanionPath = "") => {
    pendingImportRef.current = { status, error, path, companionPath, headers, mapping, bom, parts, rows };
    targetRef.current = { mode: "import" };
    autoImportRef.current = true;
    setCompanionPath(suppliedCompanionPath);
    await inspect(sourcePath, undefined, suppliedCompanionPath, true);
  }, [bom, companionPath, error, headers, inspect, mapping, parts, path, rows, status]);

  /** Complete an existing project with the source it was imported without. */
  const supplementFrom = useCallback(async (projectId: string, sourcePath: string) => {
    pendingImportRef.current = { status, error, path, companionPath, headers, mapping, bom, parts, rows };
    targetRef.current = { mode: "supplement", projectId };
    autoImportRef.current = true;
    setCompanionPath("");
    await inspect(sourcePath, undefined, "", true);
  }, [bom, companionPath, error, headers, inspect, mapping, parts, path, rows, status]);

  const submitMapping = useCallback(async (next = mapping) => {
    setMapping(next);
    if (path) await inspect(path, next, companionPath, autoImportRef.current);
  }, [companionPath, inspect, mapping, path]);

  const cancelMapping = useCallback(() => {
    autoImportRef.current = false;
    targetRef.current = { mode: "import" };
    const previous = pendingImportRef.current;
    if (previous) {
      setStatus(previous.status);
      setError(previous.error);
      setPath(previous.path);
      setCompanionPath(previous.companionPath);
      setHeaders(previous.headers);
      setMapping(previous.mapping);
      setBom(previous.bom);
      setParts(previous.parts);
      setRows(previous.rows);
      pendingImportRef.current = null;
      return;
    }
    setStatus(project || bom ? "ready" : "idle");
    setHeaders([]);
    setMapping({});
    if (!bom) setPath("");
  }, [bom, project]);

  const confirmMatch = useCallback((componentKey: string, partId: string) => {
    if (!bom) return;
    const next = createAnalysisRows(bom, parts, Object.fromEntries(rows.map((row) => [row.componentKey, row.partId ?? ""]).filter(([, id]) => id).concat([[componentKey, partId]])));
    setRows(next);
  }, [bom, parts, rows]);

  return { status, error, path, headers, mapping, setMapping, bom, rows, project, importFrom, supplementFrom, inspect, loadProject, openWelding, clearLoaded, submitMapping, cancelMapping, confirmMatch, fieldNames };
}

export { fieldNames };
