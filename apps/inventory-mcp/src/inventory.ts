import { createRequire } from "node:module";
const { DatabaseSync } = createRequire(import.meta.url)("node:sqlite") as typeof import("node:sqlite");
import { homedir } from "node:os";
import { join } from "node:path";
import { matchBomGroup, normalizePassivePackage, normalizePassiveValue, passiveKind } from "../../../packages/domain/src/bom/matching";
import type { BomGroup, InventoryPart } from "../../../packages/domain/src/bom/types";

export const SUPPORTED_SCHEMA_VERSION = 16;
export type Part = {
  id: string; name: string; category: string; package: string; manufacturer: string; mpn: string;
  lcsc_code: string; quantity: number; box_id: number | null; box_name: string | null; slot: string | null;
  note: string; version: number; updated_at: string;
};
export type Requirement = {
  kind?: "resistor" | "capacitor"; name?: string; value?: string; package?: string; mpn?: string; lcsc_code?: string;
  quantity: number; constraints?: Record<string, string | number>;
};
export type Search = { query?: string; category?: string; package?: string; kind?: "resistor" | "capacitor"; value?: string; min_quantity?: number; in_stock_only?: boolean; limit: number; offset: number };

export function defaultDatabasePath() {
  const base = process.platform === "win32" ? process.env.APPDATA ?? join(homedir(), "AppData/Roaming")
    : process.platform === "darwin" ? join(homedir(), "Library/Application Support")
    : process.env.XDG_DATA_HOME ?? join(homedir(), ".local/share");
  return join(base, "com.partnest.desktop", "partnest.db");
}

const fold = (text: string) => text.normalize("NFKC").trim().toLowerCase();
const asInventory = (part: Part): InventoryPart => ({ ...part, lcscCode: part.lcsc_code, value: part.name });
const samePackage = (a: string, b: string) => fold(a) === fold(b) || Boolean(normalizePassivePackage(a) && normalizePassivePackage(a) === normalizePassivePackage(b));
const equalNumber = (a: number, b: number) => Math.abs(a - b) <= Math.max(Math.abs(a), Math.abs(b)) * 1e-9;

function groupFor(requirement: Requirement): BomGroup {
  const kind = requirement.kind ?? passiveKind(requirement.package ?? "") ?? (/\d\s*[pnumµμ]?f(?:\s|_|$)/i.test(requirement.value ?? "") ? "capacitor" : "resistor");
  const packageName = requirement.package ?? "";
  const normalized = normalizePassivePackage(packageName);
  return { componentKey: "selection", name: requirement.name ?? "", value: requirement.value ?? "", package: normalized ? `${kind === "capacitor" ? "C" : "R"}_${normalized}` : packageName, mpn: requirement.mpn ?? "", manufacturer: "", lcscCode: requirement.lcsc_code?.trim().toUpperCase() ?? "", placements: [], extraFields: {} };
}

function unknownParameters(requirement: Requirement) {
  const kind = requirement.kind ?? passiveKind(groupFor(requirement).package);
  return [...new Set([...(kind === "capacitor" ? ["voltage_rating", "dielectric", "tolerance", "dc_bias"] : kind === "resistor" ? ["power_rating", "tolerance", "temperature_coefficient"] : ["electrical_ratings", "pin_compatibility"]), ...Object.keys(requirement.constraints ?? {})])];
}

function candidates(requirement: Requirement, parts: Part[]) {
  const match = matchBomGroup(groupFor(requirement), parts.map(asInventory), { allowSpecificationCandidates: true });
  const ids = match.kind === "none" ? [] : match.kind === "candidate" ? match.partIds : [match.partId];
  return {
    match_kind: match.kind,
    candidates: ids.flatMap((id) => {
      const part = parts.find((item) => item.id === id);
      return part ? [{ ...part, match_kind: match.kind, requested_quantity: requirement.quantity,
        sufficient_stock: part.quantity >= requirement.quantity, shortage: Math.max(0, requirement.quantity - part.quantity),
        requires_confirmation: match.kind === "candidate", electrical_compatibility: "unverified",
        requested_constraints: requirement.constraints ?? {}, unknown_parameters: unknownParameters(requirement),
        reasons: match.kind === "candidate" ? match.reasons?.[id] ?? ["名称/数值与封装候选，需核对"] : [match.kind === "exact-lcsc" ? "LCSC 编号完全一致" : "制造商料号完全一致"],
      }] : [];
    }).sort((a, b) => Number(b.sufficient_stock) - Number(a.sufficient_stock) || b.quantity - a.quantity || a.id.localeCompare(b.id)),
  };
}

export class InventoryService {
  constructor(readonly databasePath: string) {}

  /** Short read transactions see WAL contents; reopening avoids stale handles after GUI restore. */
  private snapshot<T>(read: (parts: Part[]) => T) {
    const db = new DatabaseSync(this.databasePath, { readOnly: true, allowExtension: false });
    try {
      db.exec("PRAGMA query_only=ON; PRAGMA busy_timeout=2000; BEGIN");
      const schema = db.prepare("SELECT MAX(version) AS version FROM schema_migrations").get() as { version: number } | undefined;
      if (schema?.version !== SUPPORTED_SCHEMA_VERSION) throw new Error(`不支持的库存 schema：${schema?.version ?? "未知"}，此版本要求 ${SUPPORTED_SCHEMA_VERSION}；请更新 PartNest MCP。`);
      const parts = db.prepare("SELECT p.id,p.name,coalesce(p.category,'') AS category,coalesce(p.package,'') AS package,coalesce(p.manufacturer,'') AS manufacturer,coalesce(p.mpn,'') AS mpn,coalesce(p.lcsc_code,'') AS lcsc_code,p.quantity,p.box_id,b.name AS box_name,p.slot,coalesce(p.note,'') AS note,p.version,p.updated_at FROM parts p LEFT JOIN boxes b ON b.id=p.box_id WHERE p.deleted_at IS NULL ORDER BY p.name COLLATE NOCASE,p.id").all() as Part[];
      return { schema_version: schema.version, queried_at: new Date().toISOString(), quantity_semantics: "on_hand_not_reserved", ...read(parts) };
    } finally { db.close(); }
  }

  search(input: Search) {
    return this.snapshot((parts) => {
      const tokens = fold(input.query ?? "").split(/\s+/).filter(Boolean);
      const found = parts.filter((part) => {
        if ((input.in_stock_only ?? true) && part.quantity <= 0) return false;
        if (input.min_quantity !== undefined && part.quantity < input.min_quantity) return false;
        if (input.category && !fold(part.category).includes(fold(input.category))) return false;
        if (input.package && !samePackage(input.package, part.package)) return false;
        const kind = passiveKind(part.package, part.category, part.name);
        if (input.kind && kind !== input.kind) return false;
        if (input.value) {
          if (!kind) return false;
          const expected = normalizePassiveValue(input.value, kind), actual = normalizePassiveValue(part.name, kind);
          if (expected === null || actual === null || !equalNumber(expected, actual)) return false;
        }
        const text = fold([part.name, part.category, part.package, part.mpn, part.manufacturer, part.lcsc_code].join(" "));
        return tokens.every((token) => text.includes(token));
      });
      return { total: found.length, offset: input.offset, limit: input.limit, parts: found.slice(input.offset, input.offset + input.limit) };
    });
  }

  get(id: string) {
    return this.snapshot((parts) => {
      const part = parts.find((item) => item.id === id);
      if (!part) throw new Error("库存器件不存在或已归档");
      return { part, unknown_parameters: ["voltage_rating", "tolerance", "power_rating", "dielectric", "kicad_symbol", "kicad_footprint"] };
    });
  }

  suggest(input: Requirement) {
    return this.snapshot((parts) => {
      const result = candidates(input, parts);
      return { match_kind: result.match_kind, total_candidates: result.candidates.length, candidates: result.candidates.slice(0, 20), policy: "优先现有库存；候选不表示电气等价，未知参数必须核对；选型不扣减也不预留库存。" };
    });
  }

  checkBom(lines: (Requirement & { key: string })[]) {
    return this.snapshot((parts) => {
      const remaining = new Map(parts.map((part) => [part.id, part.quantity]));
      const results = lines.map((line) => {
        const result = candidates(line, parts);
        const exact = result.match_kind === "exact-lcsc" || result.match_kind === "exact-mpn" ? result.candidates[0] : undefined;
        const available = exact ? remaining.get(exact.id) ?? 0 : 0;
        const covered = Math.min(line.quantity, available);
        const choices = result.candidates.slice(0, 10).map((candidate) => {
          const available = remaining.get(candidate.id) ?? 0;
          return { ...candidate, available_after_prior_lines: available, sufficient_stock: available >= line.quantity, shortage: Math.max(0, line.quantity - available) };
        });
        if (exact) remaining.set(exact.id, available - covered);
        return { key: line.key, requested_quantity: line.quantity, match_kind: result.match_kind, allocated_part_id: exact?.id ?? null,
          exact_identity_covered: covered, uncovered_quantity: line.quantity - covered, requires_review: !exact || unknownParameters(line).length > 0,
          candidates: choices, total_candidates: result.candidates.length, allocation_is_advisory: true };
      });
      return { allocation_policy: "input_order_shared_stock_pool_no_reservation", lines: results,
        total_required: lines.reduce((sum, line) => sum + line.quantity, 0),
        exact_identity_covered: results.reduce((sum, line) => sum + line.exact_identity_covered, 0),
        uncovered_quantity: results.reduce((sum, line) => sum + line.uncovered_quantity, 0) };
    });
  }
}
