import { expect, it } from "vitest";
import type { BomGroup, WeldingProgress } from "../../app/contracts";
import { summarizeProjectProgress } from "./projectProgress";

const group: BomGroup = { component_key: "R", name: "resistor", value: "10k", package: "0603", manufacturer: "", mpn: "", lcsc_code: "", quantity: 3, designators: ["R1", "R2", "R3"], placements: [{ designator: "R1", side: "top", component_key: "R" }, { designator: "R2", side: "top", component_key: "R" }, { designator: "R3", side: "bottom", component_key: "R" }], extra_fields: {} };

it("reports project totals before selection and counts confirmed references without duplicates", () => {
  expect(summarizeProjectProgress([group], [])).toEqual({ sides: { top: { required: 2, taken: 0 }, bottom: { required: 1, taken: 0 } }, total: 3, taken: 0, unknown: 0 });
  const progress = [{ component_key: "R", side: "top", confirmed_designators: ["R1", "R1", "NOT-IN-BOM"], consumed_quantity: 100 } as WeldingProgress];
  const result = summarizeProjectProgress([group], [...progress, ...progress]);
  expect(result.taken).toBe(1);
  expect(result.sides.top).toEqual({ required: 2, taken: 1 });
});

it("keeps unknown sides explicit and counts overall take progress", () => {
  const result = summarizeProjectProgress([{ ...group, placements: [] }], [{ component_key: "R", side: "bottom", confirmed_designators: ["R1"] } as WeldingProgress]);
  expect(result).toMatchObject({ total: 3, taken: 1, unknown: 3, sides: { top: { required: 0 }, bottom: { required: 0 } } });
});
