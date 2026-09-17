import { convertFileSrc, invoke } from "@tauri-apps/api/core";

export type Box = {
  id: number;
  name: string;
  rows: number;
  cols: number;
  occupied_slots: string[];
};

export type PartInput = {
  name: string;
  category: string;
  package: string;
  manufacturer: string;
  mpn: string;
  lcsc_code: string;
  quantity: number;
  box_id: number | null;
  slot: string | null;
  note: string;
};

export type Part = PartInput & { id: string; version: number };
export type LcscPartInfo = Pick<PartInput, "lcsc_code" | "name" | "category" | "package" | "manufacturer" | "mpn">;
export type PartDto = Omit<PartInput, "category" | "package" | "manufacturer" | "mpn" | "lcsc_code" | "note"> & {
  id: string;
  category: string | null;
  package: string | null;
  manufacturer: string | null;
  mpn: string | null;
  lcsc_code: string | null;
  note: string | null;
  version: number;
};

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

export type BomSide = "top" | "bottom";
export type ProjectKind = "interactive" | "tabular";
export type BomPlacement = { designator: string; side: BomSide | null; component_key: string };
export type BomGroup = {
  component_key: string;
  name: string;
  value: string;
  package: string;
  manufacturer: string;
  mpn: string;
  lcsc_code: string;
  quantity: number;
  designators: string[];
  placements: BomPlacement[];
  extra_fields: Record<string, string>;
};
export type NormalizedBom = { source_name: string; groups: BomGroup[] };
/** The welding workspace state: one project, plus the bridge that drives its canvas. */
export type CachedBomSession = {
  session_id: string;
  project_id: string;
  project_name: string;
  original_name: string;
  sha256: string;
  cache_name: string;
  /** `null` for tabular projects: there is no interactive canvas to load. */
  cache_path: string | null;
  kind: ProjectKind;
  token: string;
  normalized: NormalizedBom;
};
/** What an import stored: the project row and the analysis behind it. */
export type ImportedProject = {
  project_id: string;
  name: string;
  original_name: string;
  cache_name: string;
  kind: ProjectKind;
  has_table: boolean;
  normalized: NormalizedBom;
};
export type ProjectSummary = { id: string; name: string; original_name: string; kind: ProjectKind; has_table: boolean; created_at: string; active: boolean };
export type ResolvedBomSelection = {
  session_id: string;
  component_key: string;
  side: BomSide | null;
  designators: string[];
};
export type ConfirmTakeInput = {
  session_id: string;
  component_key: string;
  side: BomSide;
  designators: string[];
  bom_quantity: number;
  take_quantity: number;
  part_id: string;
  expected_part_version: number;
};
export type TakeResult = ConfirmTakeInput & {
  movement_id: string;
  required_quantity: number;
  consumed_quantity: number;
  taken_quantity: number;
  status: string;
  part_version: number;
};
export type WeldingProgress = {
  session_id: string;
  component_key: string;
  side: BomSide;
  part_id: string | null;
  required_quantity: number;
  consumed_quantity: number;
  taken_quantity: number;
  /** Designators already charged for this side; a second take must not repeat them. */
  confirmed_designators: string[];
  status: string;
};

export type Movement = {
  id: string;
  part_id: string | null;
  part_name: string | null;
  component: string | null;
  component_key: string | null;
  side: BomSide | null;
  movement_type: string;
  delta: number;
  quantity: number;
  before_quantity: number | null;
  after_quantity: number | null;
  reason: string;
  project_name: string | null;
  session_id: string | null;
  created_at: string;
  reverses_movement_id: string | null;
  reversible: boolean;
};

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
  /** Import a BOM file (interactive HTML, CSV or XLSX) as a project. */
  importProject: (input: { source_path: string; name?: string | null; mapping?: Record<string, string>; companion_csv_path?: string | null }) => Promise<ImportedProject>;
  /** Merge the source a project is still missing (its table, or its canvas). */
  supplementProject: (input: { project_id: string; source_path: string; mapping?: Record<string, string> }) => Promise<ImportedProject>;
  /** Reopen a project's stored analysis snapshot. */
  analyzeProject: (id: string) => Promise<unknown>;
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
  importProject: (input) => invoke("import_project", { input }),
  supplementProject: (input) => invoke("supplement_project", { input }),
  analyzeProject: (id) => invoke("analyze_project", { id }),
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
