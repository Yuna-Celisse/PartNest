import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { MemoryRouter } from "react-router-dom";
import { BomImportPage, type BomImportApi } from "./BomImportPage";

afterEach(cleanup);

const ready = {
  kind: "Ready" as const,
  bom: {
    source_name: "bom.csv",
    groups: [{ component_key: "lcsc:C1", name: "R1", value: "10k", package: "0603", manufacturer: "", mpn: "", lcsc_code: "C1", quantity: 5, designators: ["R1"], placements: [], extra_fields: {} }],
  },
};

/** The page must own its scrolling: the shell stays put while the history
 *  panel and the analysis table scroll independently. */
describe("bom page scroll layout contract", () => {
  it("scopes scrolling to the history panel and the analysis table", async () => {
    const api: BomImportApi = {
      inspectTabularBom: vi.fn().mockResolvedValue(ready),
      previewInteractiveBom: vi.fn(),
      cacheInteractiveBom: vi.fn(),
      listParts: vi.fn().mockResolvedValue([]),
      listBomFiles: vi.fn().mockResolvedValue([]),
    };
    render(<MemoryRouter><BomImportPage api={api} pickFile={vi.fn().mockResolvedValue("board.csv")} /></MemoryRouter>);
    fireEvent.click(screen.getByRole("button", { name: "导入 BOM" }));
    const table = await screen.findByRole("table", { name: "BOM 分析表" });

    const layout = document.querySelector(".bom-layout");
    expect(layout).toHaveClass("bom-layout");
    const history = layout?.querySelector(".bom-list");
    const workspace = layout?.querySelector(".bom-workspace");
    expect(history).toBeTruthy();
    expect(workspace).toBeTruthy();

    const analysis = workspace?.querySelector(".bom-analysis");
    expect(analysis).toBeTruthy();
    expect(table.parentElement).toHaveClass("pn-table-wrap");
    expect(analysis?.contains(table)).toBe(true);
    expect(history?.contains(analysis as Node)).toBe(false);
  });
});
