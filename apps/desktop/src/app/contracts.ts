// IPC DTOs. Keep serialized field names identical to Rust.
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
export type ProjectMatches = Record<string, { part_id: string; manual: boolean }>;
export type ProjectPackage = {
  files: { path: string; kind: string; size: number }[];
  bom_source: string;
  warnings: string[];
  front_svg: string | null;
  back_svg: string | null;
  positions: { reference: string; value: string; package: string; x_mm: number; y_mm: number; rotation: number; side: string }[];
};
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
export type TakeResult = {
  session_id: string;
  component_key: string;
  side: BomSide;
  part_id: string;
  take_quantity: number;
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

export type FieldName = "quantity" | "designators" | "name" | "value" | "package" | "manufacturer" | "mpn" | "lcsc_code" | "side";
export type FieldMapping = Partial<Record<FieldName, string>>;
/** Rust serde's externally tagged enum; adapted once at the IPC boundary. */
export type ImportPreviewWire =
  | { Ready: NormalizedBom }
  | { NeedsMapping: { headers: string[]; suggestions: FieldMapping } };
export type ImportPreview =
  | { kind: "Ready"; bom: NormalizedBom }
  | { kind: "NeedsMapping"; headers: string[]; suggestions: FieldMapping };
