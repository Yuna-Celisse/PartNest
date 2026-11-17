export type BomSide = "top" | "bottom";

export interface BomPlacement {
  designator: string;
  side: BomSide;
  componentKey: string;
}

export interface BomGroup {
  componentKey: string;
  name: string;
  value: string;
  package: string;
  manufacturer: string;
  mpn: string;
  lcscCode: string;
  placements: BomPlacement[];
  extraFields: Record<string, string>;
}

export interface NormalizedBom {
  sourceName: string;
  groups: BomGroup[];
}

export interface InventoryPart {
  id: string;
  name: string;
  package: string;
  mpn: string;
  lcscCode: string;
  value?: string;
  category?: string;
}

export type MatchResult =
  | { kind: "exact-lcsc" | "exact-mpn"; partId: string }
  | { kind: "candidate"; partIds: string[]; reasons?: Record<string, string[]> }
  | { kind: "none" };
