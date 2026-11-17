import type { BomGroup, InventoryPart, MatchResult } from "./types";

const trim = (value: string): string => value.trim();
const normalizeMpn = (value: string): string => trim(value).toLowerCase();
const normalizeCandidate = (value: string): string => trim(value).toLowerCase();

type PassiveKind = "resistor" | "capacitor";

export function passiveKind(packageName: string, category = "", name = ""): PassiveKind | null {
  if (/capacitor|电容/i.test(category) || /^(?:Capacitor[^:]*:)?C_?\d{4}(?:_|$)/i.test(packageName) || /\bC\d{4}\b/i.test(name)) return "capacitor";
  if (/resistor|电阻/i.test(category) || /^(?:Resistor[^:]*:)?R_?\d{4}(?:_|$)/i.test(packageName) || /\bR\d{4}\b/i.test(name)) return "resistor";
  return null;
}

export function normalizePassivePackage(value: string): string {
  const footprint = value.trim().split(":").pop() ?? "";
  const match = footprint.match(/^(?:[RCL]_?)?(0201|0402|0603|0805|1206|1210|1812|2010|2512)(?:_|$)/i);
  return match?.[1] ?? "";
}

export function normalizePassiveValue(value: string, kind: PassiveKind): number | null {
  const text = value.trim().replace(/(\d)([RrKkM])(\d)/, "$1.$3$2");
  if (kind === "capacitor") {
    const match = text.match(/^(\d+(?:\.\d+)?)\s*([pnumµμ]?)f(?=$|[\s_])/i);
    if (!match) return null;
    const units: Record<string, number> = { p: 1e-12, n: 1e-9, u: 1e-6, "µ": 1e-6, "μ": 1e-6, m: 1e-3, "": 1 };
    return Number(match[1]) * units[match[2].toLowerCase()];
  }
  const match = text.match(/^(\d+(?:\.\d+)?)([rRkKmM]?)(?:Ω|ohms?)?(?=$|[\s_])/i);
  if (!match) return null;
  const units: Record<string, number> = { r: 1, R: 1, k: 1e3, K: 1e3, M: 1e6, m: 1e-3, "": 1 };
  return Number(match[1]) * units[match[2]];
}

function specificationCandidates(group: BomGroup, inventory: InventoryPart[]): MatchResult {
  const kind = passiveKind(group.package);
  const packageName = normalizePassivePackage(group.package);
  const value = kind ? normalizePassiveValue(group.value || group.name, kind) : null;
  if (!kind || !packageName || value === null) return { kind: "none" };
  const matches = inventory.filter((part) => {
    if (passiveKind(part.package, part.category, part.name) !== kind || normalizePassivePackage(part.package) !== packageName) return false;
    const actual = normalizePassiveValue(part.value || part.name, kind);
    return actual !== null && Math.abs(actual - value) <= Math.max(Math.abs(actual), Math.abs(value)) * 1e-9;
  });
  if (!matches.length) return { kind: "none" };
  return { kind: "candidate", partIds: matches.map((part) => part.id), reasons: Object.fromEntries(matches.map((part) => [part.id, [
    `同${kind === "resistor" ? "阻值" : "容值"}、同 ${packageName} 封装`,
    ...(group.mpn && normalizeMpn(group.mpn) !== normalizeMpn(part.mpn) ? [`型号不同：${group.mpn} → ${part.mpn || "未填写"}`] : []),
    kind === "capacitor" ? "请核对耐压、介质、精度与直流偏压" : "请核对功率、精度与温度系数",
  ]])) };
}

export function matchBomGroup(group: BomGroup, inventory: InventoryPart[], options: { allowSpecificationCandidates?: boolean } = {}): MatchResult {
  const lcscCode = trim(group.lcscCode);
  if (lcscCode !== "") {
    const lcscMatches = inventory
      .filter((part) => trim(part.lcscCode) === lcscCode)
      .map((part) => part.id);
    if (lcscMatches.length === 1) return { kind: "exact-lcsc", partId: lcscMatches[0] };
    return lcscMatches.length > 1 ? { kind: "candidate", partIds: lcscMatches } : { kind: "none" };
  }

  const mpn = normalizeMpn(group.mpn);
  if (mpn !== "") {
    const mpnMatches = inventory
      .filter((part) => normalizeMpn(part.mpn) === mpn)
      .map((part) => part.id);
    if (mpnMatches.length === 1) return { kind: "exact-mpn", partId: mpnMatches[0] };
    if (mpnMatches.length > 1) return { kind: "candidate", partIds: mpnMatches };
    // A supplied manufacturer part number is authoritative. Do not turn a
    // conflicting identifier into a name/value match.
    return options.allowSpecificationCandidates ? specificationCandidates(group, inventory) : { kind: "none" };
  }

  const value = normalizeCandidate(group.value);
  const name = normalizeCandidate(group.name);
  const packageName = normalizeCandidate(group.package);
  const candidateIds = inventory
    .filter((part) => {
      const partName = normalizeCandidate(part.name);
      const partValue = normalizeCandidate(part.value ?? "");
      const identityMatches = (value !== "" && (partName === value || partValue === value))
        || (name !== "" && (partName === name || partValue === name));
      return identityMatches && packageName !== "" && normalizeCandidate(part.package) === packageName;
    })
    .map((part) => part.id);

  if (candidateIds.length > 0) return { kind: "candidate", partIds: candidateIds };
  return options.allowSpecificationCandidates ? specificationCandidates(group, inventory) : { kind: "none" };
}
