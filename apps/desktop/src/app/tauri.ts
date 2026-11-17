import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import type { Box, PartInput, Part, PartDto, LcscPartInfo, CachedBomSession, ImportedProject, ProjectSummary, ResolvedBomSelection, ConfirmTakeInput, TakeResult, WeldingProgress, Movement, FieldMapping, ImportPreview, ImportPreviewWire } from "./contracts";
export type * from "./contracts";

function previewFromWire(value: ImportPreviewWire): ImportPreview {
  if ("Ready" in value) return { kind: "Ready", bom: value.Ready };
  return { kind: "NeedsMapping", ...value.NeedsMapping };
}

/** Normalizes the nullable Rust DTO at the frontend boundary. */
export function normalizePart(part: PartDto | Part): Part {
  return {
    ...part,
    category: part.category ?? "",
    package: part.package ?? "",
    manufacturer: part.manufacturer ?? "",
    mpn: part.mpn ?? "",
    lcsc_code: part.lcsc_code ?? "",
    note: part.note ?? "",
  };
}

/** Converts the Rust cache PathBuf into a Tauri asset URL for webview loads. */
export function cachedBomUrl(cachePath: string): string {
  try {
    return convertFileSrc(cachePath);
  } catch {
    // Browser-only tests and the static Vite preview have no Tauri internals.
    return cachePath;
  }
}

export type DesktopApi = {
  listBoxes: () => Promise<Box[]>;
  createBox: (input: Pick<Box, "name" | "rows" | "cols">) => Promise<Box>;
  resizeBox: (id: number, rows: number, cols: number) => Promise<Box>;
  updateBox: (id: number, input: Pick<Box, "name" | "rows" | "cols">) => Promise<Box>;
  deleteBox: (id: number) => Promise<void>;
  listParts: (search?: string) => Promise<Part[]>;
  createPart: (input: PartInput) => Promise<Part>;
  updatePart: (id: string, expectedVersion: number, input: PartInput) => Promise<Part>;
  adjustStock: (id: string, delta: number, reason: string) => Promise<Part>;
  deletePart: (id: string) => Promise<void>;
  lookupLcsc: (lcscCode: string) => Promise<LcscPartInfo>;
  /** Reopen the project session the database still considers active. */
  restoreActiveWeldingSession: () => Promise<CachedBomSession | null>;
  listProjects: () => Promise<ProjectSummary[]>;
  getProjectPackage: (id: string) => Promise<import("./contracts").ProjectPackage | null>;
  getProjectModel: (id: string) => Promise<ArrayBuffer>;
  getProjectMatches: (projectId: string) => Promise<import("./contracts").ProjectMatches>;
  setProjectMatch: (projectId: string, componentKey: string, partId: string, manual: boolean) => Promise<void>;
  deleteProjectMatch: (projectId: string, componentKey: string) => Promise<void>;
  /** Import a BOM file (interactive HTML, CSV or XLSX) as a project. */
  importProject: (input: { source_path: string; name?: string | null; mapping?: Record<string, string>; companion_path?: string | null }) => Promise<ImportedProject>;
  /** Merge the source a project is still missing (its table, or its canvas). */
  supplementProject: (input: { project_id: string; source_path: string; mapping?: Record<string, string> }) => Promise<ImportedProject>;
  /** Reopen a project's stored analysis snapshot. */
  analyzeProject: (id: string) => Promise<ImportPreview>;
  inspectTabularBom: (sourcePath: string, mapping?: FieldMapping) => Promise<ImportPreview>;
  previewInteractiveBom: (sourcePath: string, companionPath?: string) => Promise<ImportPreview>;
  renameProject: (id: string, name: string) => Promise<ProjectSummary>;
  removeProject: (id: string) => Promise<void>;
  /** Open the welding workspace for one project, making it the active session. */
  openProjectWelding: (id: string) => Promise<CachedBomSession>;
  /** Resolve a selection reported by the BOM frame using the bridge contract. */
  resolveBomSelection: (token: string, designators: string[]) => Promise<ResolvedBomSelection>;
  confirmTake: (input: ConfirmTakeInput) => Promise<TakeResult>;
  reverseTake: (movementId: string) => Promise<TakeResult>;
  /** Leave the current welding session; already taken stock stays deducted. */
  endWeldingSession: (sessionId: string) => Promise<void>;
  getWeldingProgress: (sessionId: string) => Promise<WeldingProgress[]>;
  listMovements: () => Promise<Movement[]>;
  createBackup: () => Promise<string>;
  resetData: () => Promise<string>;
  restoreBackup: (backupPath: string) => Promise<void>;
};

export const desktopApi: DesktopApi = {
  listBoxes: () => invoke("list_boxes"),
  createBox: (input) => invoke("create_box", { input }),
  resizeBox: (id, rows, cols) => invoke("resize_box", { id, rows, cols }),
  updateBox: (id, input) => invoke("update_box", { id, input }),
  deleteBox: (id) => invoke("delete_box", { id }),
  listParts: async (search) => (await invoke<PartDto[]>("list_parts", { search })).map(normalizePart),
  createPart: async (input) => normalizePart(await invoke<PartDto>("create_part", { input })),
  updatePart: async (id, expectedVersion, input) => normalizePart(await invoke<PartDto>("update_part", { id, expectedVersion, input })),
  adjustStock: async (id, delta, reason) => normalizePart(await invoke<PartDto>("adjust_stock", { id, delta, reason })),
  deletePart: (id) => invoke("delete_part", { id }),
  lookupLcsc: (lcscCode) => invoke("lookup_lcsc", { lcscCode }),
  restoreActiveWeldingSession: () => invoke("restore_active_welding_session"),
  listProjects: () => invoke("list_projects"),
  getProjectPackage: (id) => invoke("get_project_package", { id }),
  getProjectModel: (id) => invoke("get_project_model", { id }),
  getProjectMatches: (projectId) => invoke("get_project_matches", { projectId }),
  setProjectMatch: (projectId, componentKey, partId, manual) => invoke("set_project_match", { projectId, componentKey, partId, manual }),
  deleteProjectMatch: (projectId, componentKey) => invoke("delete_project_match", { projectId, componentKey }),
  importProject: (input) => invoke("import_project", { input }),
  supplementProject: (input) => invoke("supplement_project", { input }),
  analyzeProject: async (id) => previewFromWire(await invoke<ImportPreviewWire>("analyze_project", { id })),
  inspectTabularBom: async (sourcePath, mapping) => previewFromWire(await invoke<ImportPreviewWire>("inspect_tabular_bom", { sourcePath, mapping })),
  previewInteractiveBom: async (sourcePath, companionPath) => previewFromWire(await invoke<ImportPreviewWire>("preview_interactive_bom", { sourcePath, companionPath })),
  renameProject: (id, name) => invoke("rename_project", { id, name }),
  removeProject: (id) => invoke("remove_project", { id }),
  openProjectWelding: (id) => invoke("open_project_welding", { id }),
  // 把原始桥接消息原样交给 Rust 校验契约，不在 webview 层重新实现一套规则。
  resolveBomSelection: (token, designators) => invoke("resolve_bom_selection", { message: { type: "partnest:bom-selection", token, designators } }),
  confirmTake: (input) => invoke("confirm_take", { input }),
  reverseTake: (movementId) => invoke("reverse_take", { movementId }),
  endWeldingSession: (sessionId) => invoke("end_welding_session", { sessionId }),
  getWeldingProgress: (sessionId) => invoke("get_welding_progress", { sessionId }),
  listMovements: () => invoke("list_movements"),
  createBackup: () => invoke("create_backup"),
  resetData: () => invoke("reset_data"),
  restoreBackup: (backupPath) => invoke("restore_backup", { backupPath }),
};

export function errorMessage(error: unknown): string {
  if (typeof error === "string") return error;
  if (error && typeof error === "object") {
    if ("code" in error && error.code === "Conflict") return "数据已被其他操作修改，请刷新后重试";
    if ("message" in error && typeof error.message === "string") return error.message;
  }
  return "操作失败，请稍后重试";
}
