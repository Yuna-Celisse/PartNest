import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { MemoryRouter } from "react-router-dom";
import { ProjectsPage, type ProjectImportApi } from "./ProjectsPage";

afterEach(cleanup);

const ready = {
  kind: "Ready" as const,
  bom: {
    source_name: "bom.csv",
    groups: [{ component_key: "lcsc:C1", name: "R1", value: "10k", package: "0603", manufacturer: "", mpn: "", lcsc_code: "C1", quantity: 5, designators: ["R1"], placements: [], extra_fields: {} }],
  },
};

const imported = { project_id: "p-1", name: "board", original_name: "board.csv", cache_name: "hash.csv", kind: "tabular" as const, normalized: ready.bom };

/** The page must own its scrolling: the shell stays put while the project list
 *  and the analysis table scroll independently. */
describe("projects page scroll layout contract", () => {
  it("scopes scrolling to the project list and the analysis table", async () => {
    const api: ProjectImportApi = {
      inspectTabularBom: vi.fn().mockResolvedValue(ready),
      previewInteractiveBom: vi.fn(),
      importProject: vi.fn().mockResolvedValue(imported),
      supplementProject: vi.fn().mockRejectedValue(new Error("unused in the layout contract")),
      listParts: vi.fn().mockResolvedValue([]),
      listProjects: vi.fn().mockResolvedValue([]),
    };
    render(<MemoryRouter><ProjectsPage api={api} pickFile={vi.fn().mockResolvedValue("board.csv")} /></MemoryRouter>);
    fireEvent.click(screen.getByRole("button", { name: "导入项目" }));
    const dialog = await screen.findByRole("dialog", { name: "导入项目" });
    fireEvent.click(within(dialog).getByRole("radio", { name: /从普通 BOM 表导入/ }));
    fireEvent.click(within(dialog).getByRole("button", { name: "选择文件" }));
    await waitFor(() => expect(within(dialog).getByText("board.csv")).toBeInTheDocument());
    fireEvent.click(within(dialog).getByRole("button", { name: "确定导入" }));
    const table = await screen.findByRole("table", { name: "BOM 分析表" });

    const layout = document.querySelector(".project-layout");
    expect(layout).toHaveClass("project-layout");
    const list = layout?.querySelector(".project-list");
    const workspace = layout?.querySelector(".project-workspace");
    expect(list).toBeTruthy();
    expect(workspace).toBeTruthy();

    const analysis = workspace?.querySelector(".bom-analysis");
    expect(analysis).toBeTruthy();
    expect(table.parentElement).toHaveClass("pn-table-wrap");
    expect(analysis?.contains(table)).toBe(true);
    expect(list?.contains(analysis as Node)).toBe(false);
  });
});
