import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { MemoryRouter } from "react-router-dom";
import { InventoryPage } from "./InventoryPage";
import { confirm as confirmDialog } from "@tauri-apps/plugin-dialog";

vi.mock("@tauri-apps/plugin-dialog", () => ({ confirm: vi.fn() }));

afterEach(() => { cleanup(); vi.restoreAllMocks(); });
const part = { id: "part-1", name: "10k resistor", category: "resistor", package: "0603", manufacturer: "Acme", mpn: "R-10K", lcsc_code: "C1", quantity: 3, box_id: 1, slot: "A0", note: "", version: 1 };
const box = { id: 1, name: "BOX1", rows: 2, cols: 2, occupied_slots: ["A0"] };

describe("InventoryPage", () => {
  it("shows the complete part form in a drawer", async () => {
    const api = { listParts: vi.fn().mockResolvedValue([]), listBoxes: vi.fn().mockResolvedValue([box]), createPart: vi.fn(), updatePart: vi.fn(), adjustStock: vi.fn() };
    render(<MemoryRouter><InventoryPage api={api} /></MemoryRouter>);
    fireEvent.click(screen.getByRole("button", { name: "新增器件" }));
    expect(screen.getByRole("dialog", { name: "新增器件" })).toBeVisible();
    for (const label of ["名称", "分类", "封装", "制造商", "MPN", "LCSC", "收纳盒", "盒位", "数量", "备注"]) expect(screen.getByLabelText(label)).toBeVisible();
  });

  it("submits the selected box integer ID instead of its visible name and never autocompletes slots", async () => {
    const api = { listParts: vi.fn().mockResolvedValue([]), listBoxes: vi.fn().mockResolvedValue([box]), createPart: vi.fn().mockResolvedValue({ ...part, quantity: 99 }), updatePart: vi.fn(), adjustStock: vi.fn() };
    render(<MemoryRouter><InventoryPage api={api} /></MemoryRouter>);
    fireEvent.click(screen.getByRole("button", { name: "新增器件" }));
    await screen.findByRole("option", { name: "BOX1" });
    fireEvent.change(screen.getByLabelText("收纳盒"), { target: { value: "1" } });
    fireEvent.click(screen.getByRole("button", { name: "选择盒位" }));
    expect(screen.getByRole("button", { name: "A0 已占用" })).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "A1 空闲" }));
    const quantity = screen.getByLabelText("数量");
    expect(quantity).toHaveAttribute("autocomplete", "off");
    fireEvent.change(quantity, { target: { value: "99" } });
    fireEvent.change(screen.getByLabelText("名称"), { target: { value: "new part" } });
    fireEvent.click(screen.getByRole("button", { name: "保存器件" }));
    await waitFor(() => expect(api.createPart).toHaveBeenCalledWith(expect.objectContaining({ box_id: 1, slot: "A1", quantity: 99 })));
  });

  it("refreshes occupied slots before starting the next new part", async () => {
    const listBoxes = vi.fn()
      .mockResolvedValueOnce([{ ...box, occupied_slots: [] }])
      .mockResolvedValueOnce([{ ...box, occupied_slots: ["A0"] }]);
    const api = { listParts: vi.fn().mockResolvedValue([]), listBoxes, createPart: vi.fn().mockResolvedValue({ ...part, slot: "A0" }), updatePart: vi.fn(), adjustStock: vi.fn() };
    render(<MemoryRouter><InventoryPage api={api} /></MemoryRouter>);

    fireEvent.click(screen.getByRole("button", { name: "新增器件" }));
    await screen.findByRole("option", { name: "BOX1" });
    fireEvent.change(screen.getByLabelText("名称"), { target: { value: "first" } });
    fireEvent.change(screen.getByLabelText("收纳盒"), { target: { value: "1" } });
    fireEvent.click(screen.getByRole("button", { name: "选择盒位" }));
    fireEvent.click(screen.getByRole("button", { name: "A0 空闲" }));
    fireEvent.click(screen.getByRole("button", { name: "保存器件" }));
    await waitFor(() => expect(api.createPart).toHaveBeenCalledWith(expect.objectContaining({ slot: "A0" })));

    fireEvent.click(screen.getByRole("button", { name: "新增器件" }));
    fireEvent.change(screen.getByLabelText("收纳盒"), { target: { value: "1" } });
    fireEvent.click(screen.getByRole("button", { name: "选择盒位" }));
    expect(await screen.findByRole("button", { name: "A0 已占用" })).toBeDisabled();
    expect(listBoxes).toHaveBeenCalledTimes(2);
  });

  it("allows zero-stock parts to be saved without occupying a box slot", async () => {
    const createPart = vi.fn().mockResolvedValue({ ...part, quantity: 0, box_id: null, slot: null });
    const api = { listParts: vi.fn().mockResolvedValue([]), listBoxes: vi.fn().mockResolvedValue([box]), createPart, updatePart: vi.fn(), adjustStock: vi.fn() };
    render(<MemoryRouter><InventoryPage api={api} /></MemoryRouter>);
    fireEvent.click(screen.getByRole("button", { name: "新增器件" }));
    fireEvent.change(screen.getByLabelText("名称"), { target: { value: "未入库器件" } });
    fireEvent.click(screen.getByRole("button", { name: "保存器件" }));
    await waitFor(() => expect(createPart).toHaveBeenCalledWith(expect.objectContaining({ quantity: 0, box_id: null, slot: null })));
  });

  it("restocks a depleted part through the ledger, then lets it be shelved again", async () => {
    const depleted = { ...part, quantity: 0, box_id: null, slot: null };
    const restocked = { ...depleted, quantity: 12, version: 2 };
    const adjustStock = vi.fn().mockResolvedValue(restocked);
    const updatePart = vi.fn().mockResolvedValue({ ...restocked, box_id: 1, slot: "A1", version: 3 });
    const api = { listParts: vi.fn().mockResolvedValue([depleted]), listBoxes: vi.fn().mockResolvedValue([box]), createPart: vi.fn(), updatePart, adjustStock };
    render(<MemoryRouter><InventoryPage api={api} /></MemoryRouter>);

    await screen.findByText("10k resistor");
    fireEvent.click(screen.getByRole("button", { name: "调整库存" }));
    fireEvent.change(screen.getByLabelText("数量"), { target: { value: "12" } });
    fireEvent.click(screen.getByRole("button", { name: "确认调整" }));
    await waitFor(() => expect(adjustStock).toHaveBeenCalledWith("part-1", 12, "采购入库"));

    // Stocked again, so the edit form can shelve it; the quantity stays read-only.
    fireEvent.click(screen.getByRole("button", { name: "编辑资料" }));
    expect(screen.getByLabelText("数量")).toHaveAttribute("readonly");
    fireEvent.change(screen.getByLabelText("收纳盒"), { target: { value: "1" } });
    fireEvent.click(screen.getByRole("button", { name: "选择盒位" }));
    fireEvent.click(screen.getByRole("button", { name: "A1 空闲" }));
    fireEvent.click(screen.getByRole("button", { name: "保存器件" }));
    await waitFor(() => expect(updatePart).toHaveBeenCalledWith("part-1", 2, expect.objectContaining({ quantity: 12, box_id: 1, slot: "A1" })));
    expect(await screen.findByRole("cell", { name: "12" })).toBeVisible();
  });

  it("moves an existing part to another box and slot", async () => {
    const secondBox = { ...box, id: 2, name: "BOX2", occupied_slots: [] };
    const updatePart = vi.fn().mockResolvedValue({ ...part, box_id: 2, slot: "B1", version: 2 });
    const api = { listParts: vi.fn().mockResolvedValue([part]), listBoxes: vi.fn().mockResolvedValue([box, secondBox]), createPart: vi.fn(), updatePart, adjustStock: vi.fn() };
    render(<MemoryRouter><InventoryPage api={api} /></MemoryRouter>);
    fireEvent.click(await screen.findByRole("button", { name: "编辑资料" }));
    fireEvent.change(screen.getByLabelText("收纳盒"), { target: { value: "2" } });
    fireEvent.click(screen.getByRole("button", { name: "选择盒位" }));
    fireEvent.click(screen.getByRole("button", { name: "B1 空闲" }));
    fireEvent.click(screen.getByRole("button", { name: "保存器件" }));
    await waitFor(() => expect(updatePart).toHaveBeenCalledWith("part-1", 1, expect.objectContaining({ box_id: 2, slot: "B1" })));
  });

  it("fills only blank fields from an explicit LCSC lookup", async () => {
    const api = {
      listParts: vi.fn().mockResolvedValue([]), listBoxes: vi.fn().mockResolvedValue([box]), createPart: vi.fn(), updatePart: vi.fn(), adjustStock: vi.fn(),
      lookupLcsc: vi.fn().mockResolvedValue({ lcsc_code: "C25804", name: "100kΩ 电阻", category: "电阻", package: "0402", manufacturer: "UNI-ROYAL", mpn: "0402WGF1003TEE" }),
    };
    render(<MemoryRouter><InventoryPage api={api} /></MemoryRouter>);
    fireEvent.click(screen.getByRole("button", { name: "新增器件" }));
    fireEvent.change(screen.getByLabelText("LCSC"), { target: { value: "C25804" } });
    fireEvent.change(screen.getByLabelText("名称"), { target: { value: "手填名称" } });
    fireEvent.click(screen.getByRole("button", { name: "查询 LCSC" }));
    await waitFor(() => expect(api.lookupLcsc).toHaveBeenCalledWith("C25804"));
    expect(screen.getByLabelText("名称")).toHaveValue("手填名称");
    expect(screen.getByLabelText("分类")).toHaveValue("电阻");
    expect(screen.getByLabelText("封装")).toHaveValue("0402");
    expect(screen.getByLabelText("制造商")).toHaveValue("UNI-ROYAL");
    expect(screen.getByLabelText("MPN")).toHaveValue("0402WGF1003TEE");
  });

  it("routes cancel and repeated new through dirty confirmation", async () => {
    const api = { listParts: vi.fn().mockResolvedValue([]), createPart: vi.fn(), updatePart: vi.fn(), adjustStock: vi.fn() };
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(false);
    render(<MemoryRouter><InventoryPage api={api} /></MemoryRouter>);
    fireEvent.click(screen.getByRole("button", { name: "新增器件" }));
    fireEvent.change(screen.getByLabelText("名称"), { target: { value: "pending" } });
    fireEvent.click(screen.getByRole("button", { name: "取消" }));
    expect(confirm).toHaveBeenCalledTimes(1);
    expect(screen.getByRole("dialog", { name: "新增器件" })).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "新增器件" }));
    expect(confirm).toHaveBeenCalledTimes(2);
    expect(screen.getByLabelText("名称")).toHaveValue("pending");
    confirm.mockReturnValue(true);
    fireEvent.click(screen.getByRole("button", { name: "取消" }));
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "新增器件" })).not.toBeInTheDocument());
  });

  it("keeps the table focused on picking information and calls audited adjustment API", async () => {
    const api = { listParts: vi.fn().mockResolvedValue([part]), createPart: vi.fn(), updatePart: vi.fn(), adjustStock: vi.fn().mockResolvedValue({ ...part, quantity: 5, version: 2 }) };
    render(<MemoryRouter><InventoryPage api={api} /></MemoryRouter>);
    expect(await screen.findByText("10k resistor")).toBeInTheDocument();
    expect(screen.getByRole("columnheader", { name: "规格" })).toBeInTheDocument();
    expect(screen.queryByRole("columnheader", { name: "制造商" })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "调整库存" }));
    fireEvent.change(screen.getByLabelText("数量"), { target: { value: "2" } });
    fireEvent.click(screen.getByRole("button", { name: "确认调整" }));
    await waitFor(() => expect(api.adjustStock).toHaveBeenCalledWith("part-1", 2, "采购入库"));
  });

  it("shows LCSC ID as a dedicated inventory column", async () => {
    const api = { listParts: vi.fn().mockResolvedValue([part]), createPart: vi.fn(), updatePart: vi.fn(), adjustStock: vi.fn() };
    render(<MemoryRouter><InventoryPage api={api} /></MemoryRouter>);
    expect(await screen.findByRole("columnheader", { name: "LCSC ID" })).toBeInTheDocument();
    expect(screen.getByRole("cell", { name: "C1" })).toBeInTheDocument();
  });

  it("shows the box name instead of its persisted integer ID in the picking list", async () => {
    const boxId = 42;
    const api = {
      listParts: vi.fn().mockResolvedValue([{ ...part, box_id: boxId, slot: "A0" }]),
      listBoxes: vi.fn().mockResolvedValue([{ ...box, id: boxId, name: "贴片电阻" }]),
      createPart: vi.fn(), updatePart: vi.fn(), adjustStock: vi.fn(),
    };
    render(<MemoryRouter><InventoryPage api={api} /></MemoryRouter>);
    expect(await screen.findByText("贴片电阻 / A0")).toBeInTheDocument();
    expect(screen.queryByText(`盒子 #${boxId} / A0`)).not.toBeInTheDocument();
  });

  it("keeps a compact empty state and a retryable user-facing load failure", async () => {
    const listParts = vi.fn().mockRejectedValue(new Error("Cannot read properties of undefined (reading 'invoke')"));
    const api = { listParts, createPart: vi.fn(), updatePart: vi.fn(), adjustStock: vi.fn() };
    render(<MemoryRouter><InventoryPage api={api} /></MemoryRouter>);
    expect(await screen.findByRole("alert")).toHaveTextContent("读取库存失败");
    expect(screen.queryByText(/Cannot read properties/)).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "重试" }));
    await waitFor(() => expect(listParts).toHaveBeenCalledTimes(2));
  });

  it("saves all editable metadata and keeps quantity read-only during edit", async () => {
    const api = { listParts: vi.fn().mockResolvedValue([part]), listBoxes: vi.fn().mockResolvedValue([box]), createPart: vi.fn(), updatePart: vi.fn().mockResolvedValue({ ...part, name: "edited", version: 2 }), adjustStock: vi.fn() };
    render(<MemoryRouter><InventoryPage api={api} /></MemoryRouter>);
    await screen.findByText("10k resistor");
    fireEvent.click(screen.getByRole("button", { name: "编辑资料" }));
    expect(screen.getByLabelText("数量")).toHaveAttribute("readonly");
    fireEvent.change(screen.getByLabelText("名称"), { target: { value: "edited" } });
    fireEvent.change(screen.getByLabelText("备注"), { target: { value: "test" } });
    fireEvent.click(screen.getByRole("button", { name: "保存器件" }));
    await waitFor(() => expect(api.updatePart).toHaveBeenCalledWith("part-1", 1, expect.objectContaining({ name: "edited", note: "test", quantity: 3 })));
  });

  it("allows zero-stock parts to be stocked while editing", async () => {
    const zero = { ...part, quantity: 0, box_id: null, slot: null };
    const api = { listParts: vi.fn().mockResolvedValue([zero]), listBoxes: vi.fn().mockResolvedValue([{ ...box, occupied_slots: ["A0"] }]), createPart: vi.fn(), updatePart: vi.fn().mockResolvedValue({ ...zero, quantity: 4, box_id: 1, slot: "A1", version: 2 }), adjustStock: vi.fn() };
    render(<MemoryRouter><InventoryPage api={api} /></MemoryRouter>);
    await screen.findByText("10k resistor");
    fireEvent.click(screen.getByRole("button", { name: "编辑资料" }));
    const quantity = screen.getByLabelText("数量");
    expect(quantity).toHaveAttribute("readonly");
    fireEvent.change(quantity, { target: { value: "4" } });
    fireEvent.change(screen.getByLabelText("收纳盒"), { target: { value: "1" } });
    fireEvent.click(screen.getByRole("button", { name: "选择盒位" }));
    fireEvent.click(screen.getByRole("button", { name: "A1 空闲" }));
    fireEvent.click(screen.getByRole("button", { name: "保存器件" }));
    await waitFor(() => expect(api.updatePart).toHaveBeenCalledWith("part-1", 1, expect.objectContaining({ quantity: 0, box_id: 1, slot: "A1" })));
  });
});


describe("inventory deletion", () => {
  const deletionApi = () => ({
    listParts: vi.fn().mockResolvedValue([part]),
    listBoxes: vi.fn().mockResolvedValue([box]),
    createPart: vi.fn(), updatePart: vi.fn(), adjustStock: vi.fn(),
    deletePart: vi.fn().mockResolvedValue(undefined),
  });


  it.each([false, true])("waits for native asynchronous confirmation before deletion: %s", async (accepted) => {
    const api = deletionApi();
    let decide!: (value: boolean) => void;
    const decision = new Promise<boolean>((resolve) => { decide = resolve; });
    vi.mocked(confirmDialog).mockReturnValue(decision);
    render(<MemoryRouter><InventoryPage api={api} /></MemoryRouter>);
    fireEvent.click(await screen.findByRole("button", { name: /更多操作$/ }));

    fireEvent.click(await screen.findByRole("menuitem", { name: "删除器件" }));
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(api.deletePart).not.toHaveBeenCalled();
    expect(screen.getByText(part.name)).toBeVisible();
    decide(accepted);
    if (accepted) {
      await waitFor(() => expect(screen.queryByText(part.name)).not.toBeInTheDocument());
      expect(api.deletePart).toHaveBeenCalledWith(part.id);
    } else {
      await new Promise((resolve) => setTimeout(resolve, 0));
      expect(api.deletePart).not.toHaveBeenCalled();
      expect(screen.getByText(part.name)).toBeVisible();
    }
  });

  it("warns about remaining stock and leaves inventory unchanged when cancelled", async () => {
    const api = deletionApi();
    const confirm = vi.mocked(confirmDialog).mockResolvedValue(false);
    render(<MemoryRouter><InventoryPage api={api} /></MemoryRouter>);
    fireEvent.click(await screen.findByRole("button", { name: /更多操作$/ }));

    fireEvent.click(await screen.findByRole("menuitem", { name: "删除器件" }));
    expect(confirm.mock.calls.at(-1)?.[0]).toContain("10k resistor");
    expect(confirm.mock.calls.at(-1)?.[0]).toContain("剩余库存 3");
    expect(confirm.mock.calls.at(-1)?.[0]).toContain("历史取用将无法撤销");
    expect(confirm.mock.calls.at(-1)?.[1]).toEqual({ title: "删除器件", kind: "warning", okLabel: "删除", cancelLabel: "取消" });
    expect(api.deletePart).not.toHaveBeenCalled();
    expect(screen.getByText(part.name)).toBeVisible();
  });

  it("removes a part only after success and refreshes the released slot", async () => {
    const api = deletionApi();
    api.listBoxes.mockResolvedValueOnce([box]).mockResolvedValue([{ ...box, occupied_slots: [] }]);
    let resolveDelete!: () => void;
    api.deletePart.mockImplementation(() => new Promise<void>((resolve) => { resolveDelete = resolve; }));
    vi.mocked(confirmDialog).mockResolvedValue(true);
    render(<MemoryRouter><InventoryPage api={api} /></MemoryRouter>);
    fireEvent.click(await screen.findByRole("button", { name: /更多操作$/ }));

    fireEvent.click(await screen.findByRole("menuitem", { name: "删除器件" }));
    await waitFor(() => expect(api.deletePart).toHaveBeenCalledWith(part.id));
    expect(screen.getByText(part.name)).toBeVisible();
    resolveDelete();
    await waitFor(() => expect(screen.queryByText(part.name)).not.toBeInTheDocument());
    await waitFor(() => expect(api.listBoxes).toHaveBeenCalledTimes(2));
    fireEvent.click(screen.getByRole("button", { name: "新增器件" }));
    fireEvent.change(screen.getByLabelText("收纳盒"), { target: { value: "1" } });
    fireEvent.click(screen.getByRole("button", { name: "选择盒位" }));
    expect(screen.getByRole("button", { name: "A0 空闲" })).toBeEnabled();
  });

  it("retains the inventory row and shows a deletion failure", async () => {
    const api = deletionApi();
    api.deletePart.mockRejectedValue({ message: "删除失败，数据库忙" });
    vi.mocked(confirmDialog).mockResolvedValue(true);
    render(<MemoryRouter><InventoryPage api={api} /></MemoryRouter>);
    fireEvent.click(await screen.findByRole("button", { name: /更多操作$/ }));

    fireEvent.click(await screen.findByRole("menuitem", { name: "删除器件" }));
    expect(await screen.findByText("删除失败，数据库忙")).toBeVisible();
    expect(screen.getByText(part.name)).toBeVisible();
    expect(api.listBoxes).toHaveBeenCalledTimes(1);
  });

  it("reports a refresh failure without pretending deletion failed", async () => {
    const api = deletionApi();
    api.listBoxes.mockResolvedValueOnce([box]).mockRejectedValue(new Error("offline"));
    vi.mocked(confirmDialog).mockResolvedValue(true);
    render(<MemoryRouter><InventoryPage api={api} /></MemoryRouter>);
    fireEvent.click(await screen.findByRole("button", { name: /更多操作$/ }));

    fireEvent.click(await screen.findByRole("menuitem", { name: "删除器件" }));
    expect(await screen.findByText("器件已删除，但盒位刷新失败，请刷新页面后重试")).toBeVisible();
    expect(screen.queryByText(part.name)).not.toBeInTheDocument();
  });

  it("shows confirmation errors without deleting inventory", async () => {
    const api = deletionApi();
    vi.mocked(confirmDialog).mockRejectedValue(new Error("确认框不可用"));
    render(<MemoryRouter><InventoryPage api={api} /></MemoryRouter>);
    fireEvent.click(await screen.findByRole("button", { name: /更多操作$/ }));

    fireEvent.click(await screen.findByRole("menuitem", { name: "删除器件" }));
    expect(await screen.findByText("确认框不可用")).toBeVisible();
    expect(api.deletePart).not.toHaveBeenCalled();
    expect(screen.getByText(part.name)).toBeVisible();
  });

});
