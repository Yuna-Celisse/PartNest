import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import type { BomGroup, InventoryPart, MatchResult } from "@partnest/domain";
import { matchBomGroup } from "@partnest/domain";
import { desktopApi, type Part, type BomFileSummary } from "../../app/tauri";
import { useCallback, useRef, useState } from "react";

export type FieldName = "quantity" | "designators" | "name" | "value" | "package" | "manufacturer" | "mpn" | "lcsc_code" | "side";
export type FieldMapping = Partial<Record<FieldName, string>>;
export type BomGroupDto = BomGroup & {
  component_key?: string;
  lcsc_code?: string;
  quantity: number;
  designators?: string[];
  extra_fields?: Record<string, string>;
};
export type NormalizedBomDto = { source_name: string; groups: BomGroupDto[] };
export type ImportPreview =
  | { kind: "Ready"; bom: NormalizedBomDto }
  | { kind: "NeedsMapping"; headers: string[]; suggestions: FieldMapping }
  | { kind: "Unsupported"; message?: string };

export type BomImportApi = {
  inspectTabularBom: (sourcePath: string, mapping?: FieldMapping) => Promise<unknown>;
  /** Read-only analysis of an interactive BOM: no session switch, but it is recorded in the history. */
  previewInteractiveBom: (sourcePath: string, companionCsvPath?: string) => Promise<unknown>;
  cacheInteractiveBom: (sourcePath: string, displayName: string, companionCsvPath?: string) => Promise<unknown>;
  listParts: () => Promise<Part[]>;
  listBomFiles?: () => Promise<BomFileSummary[]>;
  analyzeBomFile?: (id: string) => Promise<unknown>;
  removeBomFile?: (id: string) => Promise<void>;
  activateImportedBom?: (input: { id?: string; sourcePath?: string; displayName?: string }) => Promise<unknown>;
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

const baseName = (value: string): string => value.split(/[\\/]/).pop() ?? value;

const defaultApi: BomImportApi = {
  inspectTabularBom: (sourcePath, mapping) => invoke("inspect_tabular_bom", { sourcePath, mapping }),
  previewInteractiveBom: (sourcePath, companionCsvPath) => invoke("preview_interactive_bom", { sourcePath, companionCsvPath }),
  cacheInteractiveBom: (sourcePath, displayName, companionCsvPath) => invoke("cache_interactive_bom", { sourcePath, displayName, companionCsvPath }),
  listParts: () => desktopApi.listParts(),
  listBomFiles: () => desktopApi.listBomFiles(),
  analyzeBomFile: (id) => desktopApi.analyzeBomFile(id),
  removeBomFile: (id) => desktopApi.removeBomFile(id),
  activateImportedBom: (input) => desktopApi.activateImportedBom(input),
};

export const defaultPickFile = async (): Promise<string | null> => {
  const selected = await open({ multiple: false, filters: [{ name: "BOM", extensions: ["html", "csv", "xlsx"] }] });
  return typeof selected === "string" ? selected : null;
};

export const defaultPickCompanionFile = async (): Promise<string | null> => {
  const selected = await open({ multiple: false, filters: [{ name: "CSV", extensions: ["csv"] }] });
  return typeof selected === "string" ? selected : null;
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
  return {
    componentKey: group.componentKey ?? group.component_key ?? "",
    name: group.name ?? "",
    value: group.value ?? "",
    package: group.package ?? "",
    manufacturer: group.manufacturer ?? "",
    mpn: group.mpn ?? "",
    lcscCode: group.lcscCode ?? group.lcsc_code ?? "",
    placements: group.placements ?? [],
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

export function useBomImport({ api = defaultApi, pickFile = defaultPickFile, pickCompanionFile = defaultPickCompanionFile }: { api?: BomImportApi; pickFile?: () => Promise<string | null>; pickCompanionFile?: () => Promise<string | null> } = {}) {
  const [status, setStatus] = useState<"idle" | "loading" | "ready" | "needsMapping" | "unsupported" | "error">("idle");
  const [error, setError] = useState("");
  const [path, setPath] = useState("");
  const [companionPath, setCompanionPath] = useState("");
  const [headers, setHeaders] = useState<string[]>([]);
  const [mapping, setMapping] = useState<FieldMapping>({});
  const [bom, setBom] = useState<NormalizedBomDto | null>(null);
  const [parts, setParts] = useState<Part[]>([]);
  const [rows, setRows] = useState<BomAnalysisRow[]>([]);
  const [activated, setActivated] = useState(false);
  const [historyId, setHistoryId] = useState("");
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

  const inspect = useCallback(async (sourcePath: string, supplied?: FieldMapping, suppliedCompanionPath = companionPath) => {
    const extension = sourcePath.slice(sourcePath.lastIndexOf(".")).toLowerCase();
    if (!supported.has(extension)) { setStatus("unsupported"); setError("不支持的 BOM 格式"); return; }
    setPath(sourcePath); setError(""); setStatus("loading"); setActivated(false); setHistoryId("");
    try {
      // 选文件只做分析，“设为活动 BOM”才是显式动作，
      // 避免分析页的副作用中断正在进行的焊接会话。
      const result = extension === ".html"
        ? toPreview(await (suppliedCompanionPath
          ? api.previewInteractiveBom(sourcePath, suppliedCompanionPath)
          : api.previewInteractiveBom(sourcePath)))
        : toPreview(await api.inspectTabularBom(sourcePath, supplied));
      if (result.kind === "NeedsMapping") {
        setHeaders(result.headers); setMapping(result.suggestions ?? {}); setStatus("needsMapping"); return;
      }
      if (result.kind !== "Ready") { setStatus("error"); setError(result.message ?? "BOM 导入失败"); return; }
      const listed = await api.listParts();
      setBom(result.bom); setParts(listed); setRows(createAnalysisRows(result.bom, listed)); setStatus("ready");
      pendingImportRef.current = null;
    } catch (cause) { setStatus("error"); setError(cause instanceof Error ? cause.message : String(cause)); }
  }, [api, companionPath]);

  /** Make the analysed BOM the welding session: an interactive file caches a
   *  bridged copy, a tabular import opens a session from its snapshot. */
  const activate = useCallback(async () => {
    const extension = path.slice(path.lastIndexOf(".")).toLowerCase();
    if (extension === ".csv" || extension === ".xlsx") {
      if (!api.activateImportedBom || (!path && !historyId)) return;
      setStatus("loading"); setError("");
      try {
        const result = toPreview(await api.activateImportedBom({ id: historyId || undefined, sourcePath: path || undefined }));
        if (result.kind !== "Ready") { setStatus("error"); setError(result.kind === "Unsupported" ? result.message ?? "设为活动 BOM 失败" : "设为活动 BOM 失败"); return; }
        const listed = await api.listParts();
        setBom(result.bom); setParts(listed); setRows(createAnalysisRows(result.bom, listed)); setActivated(true); setStatus("ready");
      } catch (cause) { setStatus("error"); setError(cause instanceof Error ? cause.message : String(cause)); }
      return;
    }
    if (!path.toLowerCase().endsWith(".html")) return;
    setStatus("loading"); setError("");
    try {
      // Interactive BOMs are named after their file; renaming is not offered.
      const remark = baseName(path);
      const result = toPreview(await (companionPath
        ? api.cacheInteractiveBom(path, remark, companionPath)
        : api.cacheInteractiveBom(path, remark)));
      if (result.kind !== "Ready") { setStatus("error"); setError(result.kind === "Unsupported" ? result.message ?? "BOM 导入失败" : "缓存交互式 BOM 失败"); return; }
      const listed = await api.listParts();
      setBom(result.bom); setParts(listed); setRows(createAnalysisRows(result.bom, listed)); setActivated(true); setStatus("ready");
    } catch (cause) { setStatus("error"); setError(cause instanceof Error ? cause.message : String(cause)); }
  }, [api, companionPath, historyId, path]);

  /** Reload the analysis from an imported-BOM history row's cached copy. */
  const loadCached = useCallback(async (id: string) => {
    if (!api.analyzeBomFile) return;
    setStatus("loading"); setError(""); setActivated(false); setPath("");
    try {
      const result = toPreview(await api.analyzeBomFile(id));
      if (result.kind !== "Ready") { setStatus("error"); setError(result.kind === "Unsupported" ? result.message ?? "无法从历史记录解析该 BOM" : "历史记录需要重新映射，请重新选择原始文件"); return; }
      const listed = await api.listParts();
      setBom(result.bom); setParts(listed); setRows(createAnalysisRows(result.bom, listed)); setStatus("ready"); setHistoryId(id);
    } catch (cause) { setStatus("error"); setError(cause instanceof Error ? cause.message : String(cause)); }
  }, [api]);

  /** Drop the loaded analysis, for example after its history record is removed. */
  const clearLoaded = useCallback(() => {
    setStatus("idle"); setError(""); setPath(""); setCompanionPath(""); setBom(null); setParts([]); setRows([]); setHistoryId(""); setActivated(false);
  }, []);

  const chooseFile = useCallback(async () => {
    const selected = await pickFile();
    if (!selected) return;
    pendingImportRef.current = { status, error, path, companionPath, headers, mapping, bom, parts, rows };
    setCompanionPath("");
    await inspect(selected, undefined, "");
  }, [bom, companionPath, error, headers, inspect, mapping, parts, path, pickFile, rows, status]);
  const chooseCompanionFile = useCallback(async () => {
    if (!path.toLowerCase().endsWith(".html")) return;
    const selected = await pickCompanionFile();
    if (!selected) return;
    setCompanionPath(selected);
    await inspect(path, undefined, selected);
  }, [inspect, path, pickCompanionFile]);
  const submitMapping = useCallback(async (next = mapping) => { setMapping(next); if (path) await inspect(path, next); }, [inspect, mapping, path]);
  const cancelMapping = useCallback(() => {
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
    setStatus(bom ? "ready" : "idle");
    setHeaders([]);
    setMapping({});
    if (!bom) setPath("");
  }, [bom]);
  const confirmMatch = useCallback((componentKey: string, partId: string) => {
    if (!bom) return;
    const next = createAnalysisRows(bom, parts, Object.fromEntries(rows.map((row) => [row.componentKey, row.partId ?? ""]).filter(([, id]) => id).concat([[componentKey, partId]])));
    setRows(next);
  }, [bom, parts, rows]);
  return { status, error, path, companionPath, headers, mapping, setMapping, bom, rows, activated, historyId, chooseFile, chooseCompanionFile, inspect, activate, loadCached, clearLoaded, submitMapping, cancelMapping, confirmMatch, fieldNames };
}

export { fieldNames };
