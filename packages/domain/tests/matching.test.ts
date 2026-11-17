import { describe, expect, test } from "vitest";
import type { BomGroup, InventoryPart } from "../src/bom/types";
import { matchBomGroup } from "../src/bom/matching";

const group = (overrides: Partial<BomGroup> = {}): BomGroup => ({
  componentKey: "R1",
  name: "",
  value: "",
  package: "",
  manufacturer: "",
  mpn: "",
  lcscCode: "",
  placements: [],
  extraFields: {},
  ...overrides,
});

const part = (overrides: Partial<InventoryPart> = {}): InventoryPart => ({
  id: "part-1",
  name: "",
  package: "",
  mpn: "",
  lcscCode: "",
  ...overrides,
});

describe("matchBomGroup", () => {
  test("normalizes KiCad footprints and capacitance units as reviewable candidates", () => {
    const result = matchBomGroup(group({ value: "0.1uF", package: "Capacitor_SMD:C_0402_1005Metric" }), [part({ name: "100nF C0402", package: "0402", category: "Capacitors" })], { allowSpecificationCandidates: true });
    expect(result).toMatchObject({ kind: "candidate", partIds: ["part-1"] });
    if (result.kind === "candidate") expect(result.reasons?.["part-1"].join(" ")).toContain("耐压");
  });

  test("permits different resistor MPNs only as explicitly enabled specification candidates", () => {
    const bom = group({ value: "3.9k", package: "Resistor_SMD:R_0603_1608Metric", mpn: "0603WAF3901T5E" });
    const stock = [part({ name: "3.9kΩ R0603", package: "0603", mpn: "RC0603FR-073K9L", category: "Resistors" })];
    expect(matchBomGroup(bom, stock)).toEqual({ kind: "none" });
    const result = matchBomGroup(bom, stock, { allowSpecificationCandidates: true });
    expect(result.kind).toBe("candidate");
    if (result.kind === "candidate") expect(result.reasons?.["part-1"].join(" ")).toContain("型号不同");
  });

  test("keeps package, component type, units and explicit supplier identifiers authoritative", () => {
    const options = { allowSpecificationCandidates: true };
    expect(matchBomGroup(group({ value: "100nF", package: "Capacitor_SMD:C_0603_1608Metric" }), [part({ name: "100nF C0402", package: "0402", category: "Capacitors" })], options)).toEqual({ kind: "none" });
    expect(matchBomGroup(group({ value: "1M", package: "Resistor_SMD:R_0402_1005Metric" }), [part({ name: "1mΩ R0402", package: "0402", category: "Resistors" })], options)).toEqual({ kind: "none" });
    expect(matchBomGroup(group({ value: "10", package: "Resistor_SMD:R_0402_1005Metric" }), [part({ name: "10", package: "0402", category: "Capacitors" })], options)).toEqual({ kind: "none" });
    expect(matchBomGroup(group({ value: "10k", package: "Resistor_SMD:R_0402_1005Metric", lcscCode: "C999" }), [part({ name: "10kΩ R0402", package: "0402", category: "Resistors", lcscCode: "C1" })], options)).toEqual({ kind: "none" });
  });

  test("prefers exact LCSC matching", () => {
    expect(matchBomGroup(group({ lcscCode: " C1 " }), [part({ lcscCode: "C1", id: "lcsc" })])).toEqual({
      kind: "exact-lcsc",
      partId: "lcsc",
    });
  });

  test("matches MPN with trim and Unicode lowercase", () => {
    expect(matchBomGroup(group({ mpn: "  ÄBC  " }), [part({ mpn: "äbc", id: "mpn" })])).toEqual({
      kind: "exact-mpn",
      partId: "mpn",
    });
  });

  test("returns candidates for matching value or name and package", () => {
    expect(matchBomGroup(group({ value: "10k", package: "0603" }), [part({ name: "10k", package: "0603", id: "candidate" })])).toEqual({
      kind: "candidate",
      partIds: ["candidate"],
    });
  });

  test("does not return a candidate when package differs", () => {
    expect(matchBomGroup(group({ value: "10k", package: "0402" }), [part({ name: "10k", package: "0603" })])).toEqual({ kind: "none" });
  });

  test("does not use MPN when a BOM LCSC code is present", () => {
    expect(matchBomGroup(group({ lcscCode: "C1", mpn: "ABC" }), [part({ mpn: "abc" })])).toEqual({ kind: "none" });
  });

  test("does not choose arbitrarily when MPN matches multiple parts", () => {
    expect(matchBomGroup(group({ mpn: "ABC" }), [
      part({ id: "one", mpn: "abc" }),
      part({ id: "two", mpn: "ABC" }),
    ])).toEqual({ kind: "candidate", partIds: ["one", "two"] });
  });

  test("does not fall back to name when a supplied MPN conflicts", () => {
    expect(matchBomGroup(group({ name: "10k", package: "0603", mpn: "MISSING" }), [
      part({ id: "same-name", name: "10k", package: "0603", mpn: "OTHER" }),
    ])).toEqual({ kind: "none" });
  });
});
