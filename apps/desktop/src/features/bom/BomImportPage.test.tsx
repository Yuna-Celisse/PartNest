import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { MemoryRouter } from "react-router-dom";
import { confirm as confirmDialog } from "@tauri-apps/plugin-dialog";
import { BomImportPage, type BomImportApi } from "./BomImportPage";
import type { Part } from "../../app/tauri";

vi.mock("@tauri-apps/plugin-dialog", () => ({ confirm: vi.fn() }));

afterEach(() => { cleanup(); vi.clearAllMocks(); });

const part = (overrides: Partial<Part> = {}): Part => ({
  id: "part-1", name: "10k", category: "", package: "0603", manufacturer: "", mpn: "", lcsc_code: "C1", quantity: 3,
  box_id: 1, slot: "A0", note: "", version: 1, ...overrides,
});

const ready = {
  kind: "Ready" as const,
  bom: {
    source_name: "bom.csv",
    groups: [{ component_key: "lcsc:C1", name: "R1", value: "10k", package: "0603", manufacturer: "", mpn: "", lcsc_code: "C1", quantity: 5, designators: ["R1"], placements: [], extra_fields: {} }],
  },
};

function api(overrides: Partial<BomImportApi> = {}): BomImportApi {
  return {
    inspectTabularBom: vi.fn().mockResolvedValue(ready),
    previewInteractiveBom: vi.fn().mockResolvedValue(ready),
    cacheInteractiveBom: vi.fn(),
    listParts: vi.fn().mockResolvedValue([part()]),
    ...overrides,
  };
}

function renderPage(ui: JSX.Element) {
  return render(<MemoryRouter initialEntries={["/bom"]}>{ui}</MemoryRouter>);
}

describe("BomImportPage", () => {
  it("gives an unloaded BOM a compact empty workspace", () => {
    renderPage(<BomImportPage api={api()} pickFile={vi.fn()} />);
    const workspace = screen.getByRole("region", { name: "BOM 工作区" });
    expect(workspace).toHaveClass("bom-empty-state");
    expect(workspace).toHaveTextContent("未加载 BOM");
    expect(screen.getByRole("region", { name: "BOM 分析" })).not.toHaveClass("bom-empty-state");
  });

  it("points at the imported-BOM history while the workspace is empty", async () => {
    const listBomFiles = vi.fn().mockResolvedValue([
      { id: "bom-1", display_name: "板 A", original_name: "board.csv", created_at: "2026-09-14T00:00:00Z", active: false },
    ]);
    renderPage(<BomImportPage api={api({ listBomFiles })} pickFile={vi.fn()} />);

    expect(await screen.findByText(/或点击左侧「已导入 BOM」中的记录重新载入分析/)).toBeInTheDocument();
    expect(screen.getByText("点击记录载入分析，右击可移除")).toBeInTheDocument();
  });

  it("keeps the import button in the page toolbar before any BOM is loaded", () => {
    const pickFile = vi.fn().mockResolvedValue(null);
    renderPage(<BomImportPage api={api()} pickFile={pickFile} />);
    const toolbar = screen.getByRole("toolbar", { name: "BOM 操作工具栏" });
    expect(toolbar).toContainElement(screen.getByRole("button", { name: "导入 BOM" }));
    expect(screen.getByText("未加载 BOM").closest(".bom-empty-state__content")?.querySelector("button")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "导入 BOM" }));
    expect(pickFile).toHaveBeenCalledTimes(1);
  });

  it("removes an imported BOM record from its right-click menu", async () => {
    const listBomFiles = vi.fn()
      .mockResolvedValueOnce([{ id: "bom-1", display_name: "板 A", original_name: "board.csv", created_at: "2026-09-14T00:00:00Z", active: false }])
      .mockResolvedValue([]);
    const removeBomFile = vi.fn().mockResolvedValue(undefined);
    vi.mocked(confirmDialog).mockResolvedValue(true);
    renderPage(<BomImportPage api={api({ listBomFiles, removeBomFile })} pickFile={vi.fn()} />);

    fireEvent.contextMenu(await screen.findByRole("button", { name: /板 A/ }), { clientX: 40, clientY: 60 });
    fireEvent.click(screen.getByRole("menuitem", { name: "移除记录" }));

    await waitFor(() => expect(removeBomFile).toHaveBeenCalledWith("bom-1"));
    expect(confirmDialog).toHaveBeenCalledTimes(1);
    await waitFor(() => expect(screen.queryByRole("button", { name: /板 A/ })).not.toBeInTheDocument());
    expect(screen.getByText("暂无已导入 BOM")).toBeInTheDocument();
  });

  it("keeps the record when removing it is cancelled", async () => {
    const listBomFiles = vi.fn().mockResolvedValue([
      { id: "bom-1", display_name: "板 A", original_name: "board.csv", created_at: "2026-09-14T00:00:00Z", active: false },
    ]);
    const removeBomFile = vi.fn().mockResolvedValue(undefined);
    vi.mocked(confirmDialog).mockResolvedValue(false);
    renderPage(<BomImportPage api={api({ listBomFiles, removeBomFile })} pickFile={vi.fn()} />);

    fireEvent.contextMenu(await screen.findByRole("button", { name: /板 A/ }));
    fireEvent.click(screen.getByRole("menuitem", { name: "移除记录" }));

    await waitFor(() => expect(confirmDialog).toHaveBeenCalledTimes(1));
    expect(removeBomFile).not.toHaveBeenCalled();
    expect(screen.getByRole("button", { name: /板 A/ })).toBeInTheDocument();
  });

  it("reports a removal failure and keeps the record", async () => {
    const listBomFiles = vi.fn().mockResolvedValue([
      { id: "bom-1", display_name: "板 A", original_name: "board.csv", created_at: "2026-09-14T00:00:00Z", active: false },
    ]);
    const removeBomFile = vi.fn().mockRejectedValue(new Error("该 BOM 存在焊接会话记录，无法移除"));
    vi.mocked(confirmDialog).mockResolvedValue(true);
    renderPage(<BomImportPage api={api({ listBomFiles, removeBomFile })} pickFile={vi.fn()} />);

    fireEvent.contextMenu(await screen.findByRole("button", { name: /板 A/ }));
    fireEvent.click(screen.getByRole("menuitem", { name: "移除记录" }));

    expect(await screen.findByRole("alert")).toHaveTextContent("无法移除");
    expect(screen.getByRole("button", { name: /板 A/ })).toBeInTheDocument();
  });

  it("reloads an analysis from an imported BOM history row", async () => {
    const listBomFiles = vi.fn().mockResolvedValue([
      { id: "bom-1", display_name: "板 A", original_name: "board.csv", created_at: "2026-09-14T00:00:00Z", active: false },
    ]);
    const analyzeBomFile = vi.fn().mockResolvedValue(ready);
    renderPage(<BomImportPage api={api({ listBomFiles, analyzeBomFile })} pickFile={vi.fn()} />);
    fireEvent.click(await screen.findByRole("button", { name: /板 A/ }));
    expect(analyzeBomFile).toHaveBeenCalledWith("bom-1");
    expect(await screen.findByRole("navigation", { name: "BOM 导入流程" })).toBeVisible();
  });

  it("reopens a history row after the page is left and entered again", async () => {
    const listBomFiles = vi.fn().mockResolvedValue([
      { id: "bom-1", display_name: "板 A", original_name: "board.csv", created_at: "2026-09-14T00:00:00Z", active: false },
    ]);
    const analyzeBomFile = vi.fn().mockResolvedValue(ready);
    const first = renderPage(<BomImportPage api={api({ listBomFiles, analyzeBomFile })} pickFile={vi.fn()} />);
    first.unmount();

    renderPage(<BomImportPage api={api({ listBomFiles, analyzeBomFile })} pickFile={vi.fn()} />);
    fireEvent.click(await screen.findByRole("button", { name: /板 A/ }));
    expect(analyzeBomFile).toHaveBeenCalledWith("bom-1");
    expect(await screen.findByRole("navigation", { name: "BOM 导入流程" })).toBeVisible();
  });

  it("restricts the picker to supported BOM formats and reports unsupported files", async () => {
    const pickFile = vi.fn().mockResolvedValue("board.txt");
    renderPage(<BomImportPage api={api()} pickFile={pickFile} />);

    fireEvent.click(screen.getByRole("button", { name: "导入 BOM" }));
    expect(pickFile).toHaveBeenCalledTimes(1);
    await waitFor(() => expect(screen.getByRole("alert")).toHaveTextContent("不支持的 BOM 格式"));
  });

  it("shows mapping fields and prevents a source column from being mapped twice", async () => {
    const inspectTabularBom = vi.fn().mockResolvedValue({ kind: "NeedsMapping" as const, headers: ["Part", "Qty"], suggestions: {} });
    const pickFile = vi.fn().mockResolvedValue("board.csv");
    renderPage(<BomImportPage api={api({ inspectTabularBom })} pickFile={pickFile} />);
    fireEvent.click(screen.getByRole("button", { name: "导入 BOM" }));
    expect(await screen.findByText("字段映射")).toBeInTheDocument();
    const fields = screen.getAllByRole("combobox");
    fireEvent.change(fields[0], { target: { value: "Part" } });
    fireEvent.change(fields[1], { target: { value: "Part" } });
    expect(fields[1]).toHaveValue("");
  });

  it("analyses an interactive HTML BOM without caching it", async () => {
    const previewInteractiveBom = vi.fn().mockResolvedValue(ready);
    const cacheInteractiveBom = vi.fn();
    renderPage(<BomImportPage api={api({ previewInteractiveBom, cacheInteractiveBom })} pickFile={vi.fn().mockResolvedValue("board.html")} />);
    fireEvent.click(screen.getByRole("button", { name: "导入 BOM" }));
    expect(await screen.findByText("缺料分析")).toBeInTheDocument();
    expect(previewInteractiveBom).toHaveBeenCalledWith("board.html");
    expect(cacheInteractiveBom).not.toHaveBeenCalled();
    expect(screen.getByText("仅完成分析")).toBeInTheDocument();
  });

  it("caches the interactive BOM only when it is made active", async () => {
    const cacheInteractiveBom = vi.fn().mockResolvedValue({ normalized: ready.bom });
    renderPage(<BomImportPage api={api({ cacheInteractiveBom })} pickFile={vi.fn().mockResolvedValue("board.html")} />);
    fireEvent.click(screen.getByRole("button", { name: "导入 BOM" }));
    await screen.findByText("缺料分析");
    fireEvent.click(screen.getByRole("button", { name: "设为活动 BOM" }));
    await waitFor(() => expect(cacheInteractiveBom).toHaveBeenCalledWith("board.html", "board.html"));
    expect(await screen.findByText("已设为活动 BOM")).toBeInTheDocument();
  });

  it("shows import stages and an analysis summary when the BOM is ready", async () => {
    const inspectTabularBom = vi.fn().mockResolvedValue(ready);
    renderPage(<BomImportPage api={api({ inspectTabularBom })} pickFile={vi.fn().mockResolvedValue("board.csv")} />);
    fireEvent.click(screen.getByRole("button", { name: "导入 BOM" }));
    expect(await screen.findByRole("navigation", { name: "BOM 导入流程" })).toBeVisible();
    expect(screen.getByText("1 个器件组")).toBeVisible();
    expect(screen.getByText("1 个位号")).toBeVisible();
    expect(screen.getByText("1 个缺料组")).toBeVisible();
  });

  it("names an activated interactive BOM after its file without a remark field", async () => {
    const cacheInteractiveBom = vi.fn().mockResolvedValue({ normalized: ready.bom });
    renderPage(<BomImportPage api={api({ cacheInteractiveBom })} pickFile={vi.fn().mockResolvedValue("board.html")} />);
    expect(screen.queryByLabelText("BOM备注名")).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "导入 BOM" }));
    await screen.findByText("缺料分析");
    fireEvent.click(screen.getByRole("button", { name: "设为活动 BOM" }));
    await waitFor(() => expect(cacheInteractiveBom).toHaveBeenCalledWith("board.html", "board.html"));
  });

  it("activates a tabular BOM for the welding workspace", async () => {
    const activateImportedBom = vi.fn().mockResolvedValue({ normalized: ready.bom });
    renderPage(<BomImportPage api={api({ activateImportedBom })} pickFile={vi.fn().mockResolvedValue("board.csv")} />);
    fireEvent.click(screen.getByRole("button", { name: "导入 BOM" }));
    await screen.findByText("缺料分析");
    fireEvent.click(screen.getByRole("button", { name: "设为活动 BOM" }));

    await waitFor(() => expect(activateImportedBom).toHaveBeenCalledWith({ id: undefined, sourcePath: "board.csv" }));
    expect(await screen.findByText("已设为活动 BOM")).toBeInTheDocument();
  });

  it("attaches a companion CSV when analyzing and activating interactive HTML", async () => {
    const previewInteractiveBom = vi.fn().mockResolvedValue(ready);
    const cacheInteractiveBom = vi.fn().mockResolvedValue({ normalized: ready.bom });
    renderPage(<BomImportPage
      api={api({ previewInteractiveBom, cacheInteractiveBom })}
      pickFile={vi.fn().mockResolvedValue("board.html")}
      pickCompanionFile={vi.fn().mockResolvedValue("board.csv")}
    />);
    fireEvent.click(screen.getByRole("button", { name: "导入 BOM" }));
    expect(await screen.findByRole("button", { name: "配套 CSV" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "配套 CSV" }));
    await waitFor(() => expect(previewInteractiveBom).toHaveBeenLastCalledWith("board.html", "board.csv"));
    expect(screen.getByTitle("board.csv")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "设为活动 BOM" }));
    await waitFor(() => expect(cacheInteractiveBom).toHaveBeenLastCalledWith("board.html", "board.html", "board.csv"));
  });

  it("analyzes exact, candidate, and unmatched groups with non-negative shortages", async () => {
    const inspectTabularBom = vi.fn().mockResolvedValue({
      kind: "Ready" as const,
      bom: { source_name: "bom.csv", groups: [
        { component_key: "lcsc:C1", name: "R1", value: "10k", package: "0603", manufacturer: "", mpn: "", lcsc_code: "C1", quantity: 5, designators: ["R1"], placements: [], extra_fields: {} },
        { component_key: "candidate", name: "LED", value: "red", package: "0603", manufacturer: "", mpn: "", lcsc_code: "", quantity: 2, designators: ["D1", "D2"], placements: [], extra_fields: {} },
        { component_key: "none", name: "MCU", value: "x", package: "QFN", manufacturer: "", mpn: "missing", lcsc_code: "", quantity: 1, designators: ["U1"], placements: [], extra_fields: {} },
      ] },
    });
    const pickFile = vi.fn().mockResolvedValue("bom.csv");
    renderPage(<BomImportPage api={api({ inspectTabularBom, listParts: vi.fn().mockResolvedValue([
      part({ id: "exact", lcsc_code: "C1", quantity: 8 }),
      part({ id: "candidate-a", name: "red", lcsc_code: "", quantity: 1 }),
      part({ id: "candidate-b", name: "red", lcsc_code: "", quantity: 4 }),
    ]) })} pickFile={pickFile} />);
    fireEvent.click(screen.getByRole("button", { name: "导入 BOM" }));
    expect(await screen.findByText("缺料分析")).toBeInTheDocument();
    expect(screen.getByText("精确匹配")).toBeInTheDocument();
    expect(screen.getByText("候选匹配")).toBeInTheDocument();
    expect(screen.getByText("未匹配")).toBeInTheDocument();
    expect(screen.getAllByRole("cell", { name: "0" }).length).toBeGreaterThan(0);
    expect(screen.getAllByRole("button", { name: "确认匹配" }).length).toBeGreaterThan(0);
  });

  it("keeps a candidate unresolved until the user confirms a part", async () => {
    const inspectTabularBom = vi.fn().mockResolvedValue({
      kind: "Ready" as const,
      bom: { source_name: "bom.csv", groups: [{ component_key: "candidate", name: "LED", value: "red", package: "0603", manufacturer: "", mpn: "", lcsc_code: "", quantity: 2, designators: ["D1"], placements: [], extra_fields: {} }] },
    });
    renderPage(<BomImportPage api={api({ inspectTabularBom, listParts: vi.fn().mockResolvedValue([part({ id: "a", name: "red", lcsc_code: "" }), part({ id: "b", name: "red", lcsc_code: "" })]) })} pickFile={vi.fn().mockResolvedValue("bom.csv")} />);
    fireEvent.click(screen.getByRole("button", { name: "导入 BOM" }));
    expect(await screen.findByText("候选匹配")).toBeInTheDocument();
    fireEvent.click(screen.getAllByRole("button", { name: "确认匹配" })[0]);
    await waitFor(() => expect(screen.queryByText("候选匹配")).not.toBeInTheDocument());
    expect(screen.getByText("精确匹配")).toBeInTheDocument();
    expect(screen.getByText("已确认")).toBeInTheDocument();
  });

  it("keeps import controls in the toolbar and opens mapping as a dialog", async () => {
    const inspectTabularBom = vi.fn().mockResolvedValue({ kind: "NeedsMapping" as const, headers: ["Part", "Qty"], suggestions: {} });
    renderPage(<BomImportPage api={api({ inspectTabularBom })} pickFile={vi.fn().mockResolvedValue("board.csv")} />);

    const toolbar = screen.getByRole("toolbar", { name: "BOM 操作工具栏" });
    expect(toolbar).toContainElement(screen.getByRole("button", { name: "导入 BOM" }));
    fireEvent.click(screen.getByRole("button", { name: "导入 BOM" }));
    expect(await screen.findByRole("dialog", { name: "字段映射" })).toBeVisible();
  });

  it("cancels mapping without importing and keeps duplicate source fields disabled", async () => {
    const inspectTabularBom = vi.fn().mockResolvedValue({ kind: "NeedsMapping" as const, headers: ["Part", "Qty"], suggestions: {} });
    renderPage(<BomImportPage api={api({ inspectTabularBom })} pickFile={vi.fn().mockResolvedValue("board.csv")} />);
    fireEvent.click(screen.getByRole("button", { name: "导入 BOM" }));
    const dialog = await screen.findByRole("dialog", { name: "字段映射" });
    const fields = screen.getAllByRole("combobox");
    fireEvent.change(fields[0], { target: { value: "Part" } });
    expect(screen.getAllByRole("option", { name: "Part" })[1]).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "取消" }));
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "字段映射" })).not.toBeInTheDocument());
    expect(dialog).not.toBeVisible();
    expect(inspectTabularBom).toHaveBeenCalledTimes(1);
  });

  it("renders candidate and shortage values as compact status content", async () => {
    const inspectTabularBom = vi.fn().mockResolvedValue({
      kind: "Ready" as const,
      bom: { source_name: "bom.csv", groups: [{ component_key: "candidate", name: "LED", value: "red", package: "0603", manufacturer: "", mpn: "", lcsc_code: "", quantity: 5, designators: ["D1"], placements: [], extra_fields: {} }] },
    });
    renderPage(<BomImportPage api={api({ inspectTabularBom, listParts: vi.fn().mockResolvedValue([part({ id: "a", name: "red", lcsc_code: "", quantity: 2 }), part({ id: "b", name: "red", lcsc_code: "", quantity: 1 })]) })} pickFile={vi.fn().mockResolvedValue("bom.csv")} />);
    fireEvent.click(screen.getByRole("button", { name: "导入 BOM" }));
    const candidate = await screen.findByText("候选匹配");
    expect(candidate.closest("[role=\"status\"]")).toHaveAttribute("data-tone", "warning");
    expect(screen.getAllByRole("cell", { name: "5" }).length).toBeGreaterThan(0);
  });

  it("restores the previous BOM when a replacement mapping is cancelled", async () => {
    const inspectTabularBom = vi.fn()
      .mockResolvedValueOnce(ready)
      .mockResolvedValueOnce({ kind: "NeedsMapping" as const, headers: ["Part", "Qty"], suggestions: {} });
    const pickFile = vi.fn()
      .mockResolvedValueOnce("old.csv")
      .mockResolvedValueOnce("new.csv");
    renderPage(<BomImportPage api={api({ inspectTabularBom })} pickFile={pickFile} />);

    fireEvent.click(screen.getByRole("button", { name: "导入 BOM" }));
    expect(await screen.findByText("缺料分析")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "导入 BOM" }));
    expect(await screen.findByRole("dialog", { name: "字段映射" })).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "取消" }));

    await waitFor(() => expect(screen.queryByRole("dialog", { name: "字段映射" })).not.toBeInTheDocument());
    expect(screen.getByText("缺料分析")).toBeInTheDocument();
    expect(screen.getByTitle("old.csv")).toBeInTheDocument();
  });

  it("returns to the empty workspace after cancelling the first mapping", async () => {
    const inspectTabularBom = vi.fn().mockResolvedValue({ kind: "NeedsMapping" as const, headers: ["Part", "Qty"], suggestions: {} });
    renderPage(<BomImportPage api={api({ inspectTabularBom })} pickFile={vi.fn().mockResolvedValue("new.csv")} />);
    fireEvent.click(screen.getByRole("button", { name: "导入 BOM" }));
    expect(await screen.findByRole("dialog", { name: "字段映射" })).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "取消" }));

    await waitFor(() => expect(screen.queryByRole("dialog", { name: "字段映射" })).not.toBeInTheDocument());
    expect(screen.queryByTitle("new.csv")).not.toBeInTheDocument();
    expect(screen.queryByText("缺料分析")).not.toBeInTheDocument();
  });

  it("restores an unsupported state when mapping follows an unsupported replacement", async () => {
    const inspectTabularBom = vi.fn()
      .mockResolvedValueOnce(ready)
      .mockResolvedValueOnce({ kind: "NeedsMapping" as const, headers: ["Part", "Qty"], suggestions: {} });
    const pickFile = vi.fn()
      .mockResolvedValueOnce("old.csv")
      .mockResolvedValueOnce("notes.txt")
      .mockResolvedValueOnce("new.csv");
    renderPage(<BomImportPage api={api({ inspectTabularBom })} pickFile={pickFile} />);

    fireEvent.click(screen.getByRole("button", { name: "导入 BOM" }));
    expect(await screen.findByText("缺料分析")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "导入 BOM" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("不支持的 BOM 格式");
    fireEvent.click(screen.getByRole("button", { name: "导入 BOM" }));
    expect(await screen.findByRole("dialog", { name: "字段映射" })).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "取消" }));

    await waitFor(() => expect(screen.queryByRole("dialog", { name: "字段映射" })).not.toBeInTheDocument());
    expect(screen.getByRole("alert")).toHaveTextContent("不支持的 BOM 格式");
    expect(screen.queryByText("缺料分析")).not.toBeInTheDocument();
    expect(screen.getByTitle("old.csv")).toBeInTheDocument();
  });
});
