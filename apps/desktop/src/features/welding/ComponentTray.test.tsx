import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import type { BomGroup } from "../../app/contracts";
import { useResizableColumns } from "../../components/ui/useResizableColumns";
import { ComponentTray } from "./ComponentTray";

afterEach(cleanup);
const group = (name: string, quantity: number, side: "top" | "bottom", packageName: string): BomGroup => ({ component_key: name, name, value: name, package: packageName, quantity, manufacturer: "", mpn: "", lcsc_code: "", extra_fields: {}, designators: Array.from({ length: quantity }, (_, i) => `${name}-${i}`), placements: Array.from({ length: quantity }, (_, i) => ({ designator: `${name}-${i}`, side, component_key: name })) });
const groups = [group("R10", 10, "bottom", "0805"), group("R2", 2, "top", "0603")];
function Harness({ onSelect = vi.fn() }: { onSelect?: (refs: string[]) => void }) {
  const columns = useResizableColumns({ component: 180, package: 120, quantity: 96, side: 100 }, { maxWidth: 2400 });
  return <ComponentTray groups={groups} side="all" activeComponentKey={null} widths={columns.widths} onResizeStart={columns.startResize} onResizeKey={columns.adjustWidth} onSelectDesignators={onSelect} collapsed={false} onToggle={vi.fn()} />;
}

it.each([
  ["器件", ["R2", "R10"]], ["封装", ["R2", "R10"]], ["数量", ["R2", "R10"]], ["板面", ["R10", "R2"]],
] as const)("sorts %s in both directions without changing source data or row selection", (column, ascending) => {
  const onSelect = vi.fn(); render(<Harness onSelect={onSelect} />);
  const order = () => screen.getAllByRole("row").slice(1).map((row) => within(row).getAllByRole("cell")[0].textContent);
  fireEvent.click(screen.getByRole("button", { name: column }));
  expect(order()).toEqual(ascending);
  expect(screen.getByRole("columnheader", { name: new RegExp(`^${column}`) })).toHaveAttribute("aria-sort", "ascending");
  fireEvent.click(screen.getByRole("button", { name: column }));
  expect(order()).toEqual([...ascending].reverse());
  expect(onSelect).not.toHaveBeenCalled();
  fireEvent.click(screen.getByTestId("tray-row-R2"));
  expect(onSelect).toHaveBeenCalledWith(groups[1].designators);
  expect(groups.map((g) => g.name)).toEqual(["R10", "R2"]);
});

it("starts resizing from measured widths, preserves other columns and supports fitting again", () => {
  render(<Harness />);
  const headers = screen.getAllByRole("columnheader");
  const rendered = [600, 700, 150, 180];
  headers.forEach((header, index) => vi.spyOn(header, "getBoundingClientRect").mockImplementation(() => ({ width: header.style.flexGrow === "0" ? parseFloat(header.style.width) : rendered[index] } as DOMRect)));
  const separator = screen.getByRole("separator", { name: "调整器件列宽" });
  fireEvent.mouseDown(separator, { clientX: 100 });
  fireEvent.mouseMove(document, { clientX: 130 }); fireEvent.mouseUp(document);
  expect(headers[0]).toHaveStyle({ width: "630px", flexGrow: "0" });
  expect(headers[1]).toHaveStyle({ width: "700px", flexGrow: "0" });
  expect(headers[2]).toHaveStyle({ width: "150px" });
  expect(headers[3]).toHaveStyle({ width: "180px" });
  fireEvent.keyDown(separator, { key: "ArrowRight" });
  expect(headers[0]).toHaveStyle({ width: "638px" });
  expect(headers[0]).toHaveAttribute("aria-sort", "none");
  fireEvent.click(screen.getByRole("button", { name: "适应宽度" }));
  expect(headers[0].style.flexGrow).not.toBe("0");
  expect(headers[0].style.flexShrink).toBe("1");
});
