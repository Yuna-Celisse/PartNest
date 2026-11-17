import type { BomGroup, BomSide, WeldingProgress } from "../../app/contracts";

export function summarizeProjectProgress(groups: BomGroup[], progress: WeldingProgress[]) {
  const required = { top: new Set<string>(), bottom: new Set<string>() };
  const all = new Set<string>();
  const taken = new Set<string>();
  let unnamed = 0;
  for (const group of groups) {
    for (const ref of group.designators) all.add(`${group.component_key}\0${ref}`);
    unnamed += Math.max(0, group.quantity - group.designators.length);
    for (const placement of group.placements) {
      const key = `${group.component_key}\0${placement.designator}`;
      if (placement.side && all.has(key)) required[placement.side].add(key);
    }
  }
  // Count physical references, rather than requested/taken quantity: repeated
  // takes or historical rows must never inflate assembly completion.
  for (const entry of progress) {
    for (const ref of entry.confirmed_designators ?? []) {
      const key = `${entry.component_key}\0${ref}`;
      if (all.has(key)) taken.add(key);
    }
  }
  const known = new Set([...required.top, ...required.bottom]);
  const sides = Object.fromEntries((["top", "bottom"] as BomSide[]).map((side) => [side, {
    required: required[side].size,
    taken: [...required[side]].filter((key) => taken.has(key)).length,
  }])) as Record<BomSide, { required: number; taken: number }>;
  return { sides, total: all.size + unnamed, taken: taken.size, unknown: all.size - known.size + unnamed };
}
