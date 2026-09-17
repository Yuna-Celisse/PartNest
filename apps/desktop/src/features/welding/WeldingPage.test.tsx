import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { convertFileSrc } from "@tauri-apps/api/core";
import { confirm as confirmDialog } from "@tauri-apps/plugin-dialog";
import { WeldingPage, type WeldingApi } from "./WeldingPage";
import type { Part, ResolvedBomSelection } from "../../app/tauri";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
  convertFileSrc: vi.fn((path: string) => `asset://localhost/${encodeURIComponent(path)}`),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ confirm: vi.fn() }));

afterEach(() => { cleanup(); vi.clearAllMocks(); });

const part = (overrides: Partial<Part> = {}): Part => ({
  id: "part-1", name: "10k", category: "resistor", package: "0603", manufacturer: "Acme", mpn: "R-10K",
  lcsc_code: "C1", quantity: 8, box_id: 1, slot: "A0", note: "", version: 1, ...overrides,
});

const session = {
  session_id: "session-1", project_id: "p-1", project_name: "主板", original_name: "board.html",
  sha256: "hash", cache_name: "hash.html", cache_path: "C:\\Users\\test\\AppData\\Roaming\\PartNest\\interactive-bom-cache\\hash.html", kind: "interactive", token: "token-1",
  normalized: {
    source_name: "board.html",
    groups: [{ component_key: "C1", name: "10k", value: "10k", package: "0603", manufacturer: "Acme", mpn: "R-10K", lcsc_code: "C1", quantity: 3,
      designators: ["R1", "R2", "R3"],
      placements: [
        { designator: "R1", side: "top", component_key: "C1" },
        { designator: "R2", side: "top", component_key: "C1" },
        { designator: "R3", side: "bottom", component_key: "C1" },
      ], extra_fields: {} }],
  },
};

function makeApi(overrides: Partial<WeldingApi> = {}): WeldingApi {
  return {
    restoreActiveWeldingSession: vi.fn().mockResolvedValue(session),
    resolveBomSelection: vi.fn().mockResolvedValue({ session_id: "session-1", component_key: "C1", side: "top", designators: ["R1", "R2"] }),
    listParts: vi.fn().mockResolvedValue([part()]),
    confirmTake: vi.fn().mockResolvedValue({ movement_id: "move-1", session_id: "session-1", component_key: "C1", side: "top", part_id: "part-1", take_quantity: 2, required_quantity: 2, consumed_quantity: 2, taken_quantity: 2, status: "taken", part_version: 2 }),
    getWeldingProgress: vi.fn().mockResolvedValue([]),
    ...overrides,
  };
}

/** A CSV/XLSX import: no canvas and, in this case, no recorded board sides. */
const tabularSession = {
  ...session,
  session_id: "session-2", project_id: "p-2", project_name: "板 B", original_name: "board.csv",
  sha256: "hash2", cache_name: "hash2.csv", cache_path: null, kind: "tabular",
  normalized: {
    source_name: "board.csv",
    groups: [{ component_key: "C1", name: "10k", value: "10k", package: "0603", manufacturer: "Acme", mpn: "R-10K", lcsc_code: "C1", quantity: 2,
      designators: ["R1", "R2"], placements: [], extra_fields: {} }],
  },
};

function selectInBom(designators: string[], source?: MessageEventSource | null) {
  const frame = screen.getByTitle("交互式 BOM");
  window.dispatchEvent(new MessageEvent("message", { source: source ?? (frame as HTMLIFrameElement).contentWindow, data: { type: "partnest:bom-selection", token: "token-1", designators } }));
}

describe("WeldingPage", () => {
  it("shows a visible restore error and retries loading the workspace", async () => {
    const restore = vi.fn()
      .mockRejectedValueOnce(new Error("缓存 BOM 已损坏"))
      .mockResolvedValueOnce(session);
    const api = makeApi({ restoreActiveWeldingSession: restore });
    render(<WeldingPage api={api} />);

    expect(await screen.findByRole("alert")).toHaveTextContent("缓存 BOM 已损坏");
    fireEvent.click(screen.getByRole("button", { name: "重试" }));
    await screen.findByTitle("交互式 BOM");
    expect(restore).toHaveBeenCalledTimes(2);
  });

  it("keeps the empty state when no active BOM is restored", async () => {
    const api = makeApi({ restoreActiveWeldingSession: vi.fn().mockResolvedValue(null) });
    const navigate = vi.fn();
    render(<WeldingPage api={api} navigate={navigate} />);

    expect(await screen.findByText("暂无进行中的焊接项目")).toBeInTheDocument();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "选择项目" }));
    expect(await screen.findByRole("dialog", { name: "选择项目" })).toHaveTextContent("暂无项目");
    fireEvent.click(screen.getByRole("button", { name: "去项目页" }));
    expect(navigate).toHaveBeenCalledWith("/projects");
  });

  it("opens a picked project straight from the chooser", async () => {
    const api = makeApi({
      restoreActiveWeldingSession: vi.fn().mockResolvedValue(null),
      listProjects: vi.fn().mockResolvedValue([
        { id: "p-2", name: "板 B", original_name: "board.csv", created_at: "2026-09-14T00:00:00Z", active: false },
      ]),
      openProjectWelding: vi.fn().mockResolvedValue(tabularSession),
    });
    render(<WeldingPage api={api} />);

    fireEvent.click(await screen.findByRole("button", { name: "选择项目" }));
    const dialog = await screen.findByRole("dialog", { name: "选择项目" });
    expect(within(dialog).getByText("board.csv")).toBeInTheDocument();
    fireEvent.click(within(dialog).getByRole("button", { name: /板 B/ }));

    await waitFor(() => expect(api.openProjectWelding).toHaveBeenCalledWith("p-2"));
    expect(await screen.findByText("表格 BOM 没有交互式画布")).toBeInTheDocument();
    expect(screen.queryByRole("dialog", { name: "选择项目" })).not.toBeInTheDocument();
  });

  it("keeps the chooser open with the reason when opening fails", async () => {
    const api = makeApi({
      restoreActiveWeldingSession: vi.fn().mockResolvedValue(null),
      listProjects: vi.fn().mockResolvedValue([
        { id: "p-3", name: "旧板", original_name: "board.html", created_at: "2026-09-14T00:00:00Z", active: false },
      ]),
      openProjectWelding: vi.fn().mockRejectedValue(new Error("该项目的画布缓存已丢失，请在「项目」页重新导入原始 BOM 文件")),
    });
    render(<WeldingPage api={api} />);

    fireEvent.click(await screen.findByRole("button", { name: "选择项目" }));
    const dialog = await screen.findByRole("dialog", { name: "选择项目" });
    fireEvent.click(within(dialog).getByRole("button", { name: /旧板/ }));

    expect(await within(dialog).findByRole("alert")).toHaveTextContent("缓存已丢失");
    expect(screen.getByText("暂无进行中的焊接项目")).toBeInTheDocument();
    expect(screen.getByRole("dialog", { name: "选择项目" })).toBeInTheDocument();
  });

  it("leaves the welding session without returning taken stock", async () => {
    const endWeldingSession = vi.fn().mockResolvedValue(undefined);
    const api = makeApi({ endWeldingSession });
    vi.mocked(confirmDialog).mockResolvedValue(true);
    render(<WeldingPage api={api} />);
    await screen.findByTitle("交互式 BOM");

    fireEvent.click(screen.getByRole("button", { name: "退出当前焊接" }));

    await waitFor(() => expect(endWeldingSession).toHaveBeenCalledWith("session-1"));
    expect(confirmDialog).toHaveBeenCalledTimes(1);
    expect(vi.mocked(confirmDialog).mock.calls[0][0]).toContain("不会退回库存");
    expect(api.confirmTake).not.toHaveBeenCalled();
    expect(await screen.findByText("暂无进行中的焊接项目")).toBeInTheDocument();
    expect(screen.queryByTitle("交互式 BOM")).not.toBeInTheDocument();
  });

  it("stays in the session when leaving is cancelled", async () => {
    const endWeldingSession = vi.fn();
    vi.mocked(confirmDialog).mockResolvedValue(false);
    render(<WeldingPage api={makeApi({ endWeldingSession })} />);
    await screen.findByTitle("交互式 BOM");

    fireEvent.click(screen.getByRole("button", { name: "退出当前焊接" }));

    await waitFor(() => expect(confirmDialog).toHaveBeenCalledTimes(1));
    expect(endWeldingSession).not.toHaveBeenCalled();
    expect(screen.getByTitle("交互式 BOM")).toBeInTheDocument();
  });

  it("keeps the light BOM canvas inside the dark 65/35 workspace", async () => {
    const api = makeApi();
    render(<WeldingPage api={api} />);
    const frame = await screen.findByTitle("交互式 BOM");
    expect(frame).toHaveAttribute("sandbox", "allow-scripts allow-same-origin");
    expect(frame.closest("[data-bom-canvas]")).toHaveClass("bom-canvas-light");
    expect(screen.getByTestId("welding-layout")).toHaveAttribute("data-split", "65-35");
  });

  it("shows the project context and side progress summary", async () => {
    const api = makeApi({
      getWeldingProgress: vi.fn().mockResolvedValue([{ session_id: "session-1", component_key: "C1", side: "top", part_id: "part-1", required_quantity: 2, consumed_quantity: 1, taken_quantity: 1, status: "partial" }]),
    });
    render(<WeldingPage api={api} />);
    await screen.findByTitle("交互式 BOM");
    expect(screen.getByRole("heading", { name: "主板" })).toBeVisible();
    expect(screen.getByText("board.html")).toBeVisible();
    selectInBom(["R1", "R2"]);
    expect(await screen.findByText("顶层 · 部分取用")).toBeVisible();
  });

  it("shows the full BOM in a collapsible tray and highlights the active group", async () => {
    const extraGroup = { ...session.normalized.groups[0], component_key: "C2", name: "1k", value: "1k", designators: ["R4"], placements: [{ designator: "R4", side: "top", component_key: "C2" }] };
    const api = makeApi({ restoreActiveWeldingSession: vi.fn().mockResolvedValue({ ...session, normalized: { ...session.normalized, groups: [...session.normalized.groups, extraGroup] } }) });
    render(<WeldingPage api={api} />);
    await screen.findByTitle("交互式 BOM");
    expect(screen.getByRole("region", { name: "器件列表" })).toBeInTheDocument();
    expect(screen.getByText("1k")).toBeInTheDocument();
    selectInBom(["R1", "R2"]);
    await screen.findByText("当前选择：R1, R2");
    expect(screen.getByTestId("tray-row-C1")).toHaveAttribute("data-active", "true");
    fireEvent.click(screen.getByRole("button", { name: "收起器件列表" }));
    expect(screen.getByRole("region", { name: "器件列表" })).toHaveAttribute("data-collapsed", "true");
  });

  it("keeps an empty current side informational and never submits a take", async () => {
    const topOnlySession = {
      ...session,
      normalized: {
        ...session.normalized,
        groups: [{ ...session.normalized.groups[0], quantity: 2, designators: ["R1", "R2"], placements: [
          { designator: "R1", side: "top" as const, component_key: "C1" },
          { designator: "R2", side: "top" as const, component_key: "C1" },
        ] }],
      },
    };
    const api = makeApi({ restoreActiveWeldingSession: vi.fn().mockResolvedValue(topOnlySession) });
    render(<WeldingPage api={api} />);
    await screen.findByTitle("交互式 BOM");
    selectInBom(["R1", "R2"]);
    await screen.findByText("当前选择：R1, R2");

    fireEvent.click(screen.getByRole("tab", { name: "底层" }));
    expect(screen.getByText("当前面无器件", { selector: ".welding-empty-side" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /确认取用/ })).not.toBeInTheDocument();
    expect(api.confirmTake).not.toHaveBeenCalled();
  });

  it("switches board side tabs with the keyboard", async () => {
    const api = makeApi();
    render(<WeldingPage api={api} />);
    await screen.findByTitle("交互式 BOM");

    const tablist = screen.getByRole("tablist", { name: "板面" });
    const topTab = screen.getByRole("tab", { name: "顶层" });
    const bottomTab = screen.getByRole("tab", { name: "底层" });
    topTab.focus();
    expect(topTab).toHaveAttribute("tabindex", "0");
    expect(bottomTab).toHaveAttribute("tabindex", "-1");

    fireEvent.keyDown(tablist, { key: "ArrowRight" });
    expect(bottomTab).toHaveAttribute("aria-selected", "true");
    expect(bottomTab).toHaveFocus();

    fireEvent.keyDown(tablist, { key: "End" });
    expect(screen.getByRole("tab", { name: "全部" })).toHaveAttribute("aria-selected", "true");

    fireEvent.keyDown(tablist, { key: "Home" });
    expect(topTab).toHaveAttribute("aria-selected", "true");
    expect(topTab).toHaveFocus();
  });

  it("uses the tray as a selection fallback without injecting styles into the BOM frame", async () => {
    const api = makeApi();
    render(<WeldingPage api={api} />);
    const frame = await screen.findByTitle("交互式 BOM");
    const trayRow = screen.getByTestId("tray-row-C1");
    expect(screen.getByRole("table", { name: "BOM 器件" })).toHaveAttribute("data-scroll-container", "true");
    expect(trayRow.tagName).toBe("DIV");
    fireEvent.click(trayRow);
    await waitFor(() => expect(api.resolveBomSelection).toHaveBeenCalledWith("token-1", ["R1", "R2"]));
    expect(frame).not.toHaveAttribute("srcdoc");
    expect(frame).not.toHaveAttribute("style");
    expect(frame.children).toHaveLength(0);
  });

  it("resolves a single designator selection from the BOM bridge", async () => {
    const api = makeApi({ resolveBomSelection: vi.fn().mockResolvedValue({ session_id: "session-1", component_key: "C1", side: "top", designators: ["R1"] }) });
    render(<WeldingPage api={api} />);
    await screen.findByTitle("交互式 BOM");
    selectInBom(["R1"]);
    await waitFor(() => expect(api.resolveBomSelection).toHaveBeenCalledWith("token-1", ["R1"]));
    expect(await screen.findByText("当前选择：R1")).toBeInTheDocument();
  });

  it("records the selected designator subset as the BOM quantity", async () => {
    const api = makeApi({ resolveBomSelection: vi.fn().mockResolvedValue({ session_id: "session-1", component_key: "C1", side: "top", designators: ["R1"] }) });
    render(<WeldingPage api={api} />);
    await screen.findByTitle("交互式 BOM");
    selectInBom(["R1"]);
    fireEvent.click(await screen.findByRole("button", { name: /确认取用/ }));
    await waitFor(() => expect(api.confirmTake).toHaveBeenCalledWith(expect.objectContaining({ designators: ["R1"], bom_quantity: 1 })));
  });

  it("resolves a BOM selection without confirming or mutating stock", async () => {
    const api = makeApi();
    render(<WeldingPage api={api} />);
    await screen.findByTitle("交互式 BOM");
    selectInBom(["R1", "R2"]);
    await waitFor(() => expect(api.resolveBomSelection).toHaveBeenCalledWith("token-1", ["R1", "R2"]));
    expect(api.confirmTake).not.toHaveBeenCalled();
    expect(screen.getByText("R1, R2")).toBeInTheDocument();
  });

  it("converts the raw cached path through the production Tauri adapter", async () => {
    const api = makeApi();
    render(<WeldingPage api={api} />);
    const frame = await screen.findByTitle("交互式 BOM");
    expect(convertFileSrc).toHaveBeenCalledWith(session.cache_path);
    expect(frame).toHaveAttribute("src", `asset://localhost/${encodeURIComponent(session.cache_path)}`);
  });

  it("uses the edited quantity only after explicit confirmation", async () => {
    const api = makeApi();
    render(<WeldingPage api={api} />);
    await screen.findByTitle("交互式 BOM");
    selectInBom(["R1", "R2"]);
    const input = await screen.findByLabelText("取用数量");
    fireEvent.change(input, { target: { value: "4" } });
    expect(api.confirmTake).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "确认取用（−4）" }));
    await waitFor(() => expect(api.confirmTake).toHaveBeenCalledWith(expect.objectContaining({ take_quantity: 4, bom_quantity: 2, side: "top", expected_part_version: 1 })));
  });

  it("groups the selected part, stock and confirmation into a compact take card", async () => {
    const api = makeApi();
    render(<WeldingPage api={api} />);
    await screen.findByTitle("交互式 BOM");
    selectInBom(["R1", "R2"]);
    const panel = await screen.findByRole("region", { name: "取用面板" });
    expect(panel).toHaveClass("take-panel");
    expect(screen.getByRole("heading", { name: "取用信息" })).toBeVisible();
    expect(panel.querySelector("dl")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "确认取用（−2）" })).toBeVisible();
  });

  it("keeps bottom pending after a top confirmation", async () => {
    const api = makeApi({
      getWeldingProgress: vi.fn().mockResolvedValue([{ session_id: "session-1", component_key: "C1", side: "top", part_id: "part-1", required_quantity: 2, consumed_quantity: 2, taken_quantity: 2, status: "taken" }]),
    });
    render(<WeldingPage api={api} />);
    await screen.findByTitle("交互式 BOM");
    selectInBom(["R1", "R2"]);
    fireEvent.click(await screen.findByRole("button", { name: /确认取用/ }));
    await waitFor(() => expect(api.getWeldingProgress).toHaveBeenCalledWith("session-1"));
    fireEvent.click(screen.getByRole("tab", { name: "全部" }));
    expect(screen.getByText("底层：待取用")).toBeInTheDocument();
  });

  it("preserves edited input after insufficient stock failure", async () => {
    const api = makeApi({ confirmTake: vi.fn().mockRejectedValue(new Error("库存不足")) });
    render(<WeldingPage api={api} />);
    await screen.findByTitle("交互式 BOM");
    selectInBom(["R1", "R2"]);
    const input = await screen.findByLabelText("取用数量");
    fireEvent.change(input, { target: { value: "99" } });
    fireEvent.click(screen.getByRole("button", { name: "确认取用（−99）" }));
    await waitFor(() => expect(screen.getByRole("alert")).toHaveTextContent("库存不足"));
    expect(input).toHaveValue(99);
  });

  it("resets column widths after unmount and remount", async () => {
    const api = makeApi();
    const view = render(<WeldingPage api={api} />);
    await screen.findByTitle("交互式 BOM");
    selectInBom(["R1", "R2"]);
    await screen.findByText("R1, R2");
    const separator = screen.getByRole("separator", { name: "调整器件列宽" });
    expect(screen.getByTestId("component-column")).toHaveStyle({ width: "180px" });
    fireEvent.mouseDown(separator, { clientX: 100 });
    fireEvent.mouseMove(document, { clientX: 250 });
    fireEvent.mouseUp(document);
    expect(screen.getByTestId("component-column")).toHaveStyle({ width: "330px" });
    expect(screen.getByTestId("component-cell")).toHaveStyle({ width: "330px" });
    fireEvent.keyDown(separator, { key: "ArrowLeft" });
    expect(separator).toHaveAttribute("aria-valuenow", "322");
    view.unmount();
    render(<WeldingPage api={api} />);
    await screen.findByTitle("交互式 BOM");
    selectInBom(["R1", "R2"]);
    await screen.findByText("R1, R2");
    expect(screen.getByTestId("component-column")).toHaveStyle({ width: "180px" });
  });

  it("rejects messages from a different window before resolving", async () => {
    const api = makeApi();
    render(<WeldingPage api={api} />);
    const frame = await screen.findByTitle("交互式 BOM");
    const foreign = document.createElement("iframe");
    document.body.appendChild(foreign);
    selectInBom(["R1", "R2"], foreign.contentWindow);
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(api.resolveBomSelection).not.toHaveBeenCalled();
    frame.remove(); foreign.remove();
  });

  it("rejects invalid message shapes before resolving", async () => {
    const api = makeApi();
    render(<WeldingPage api={api} />);
    const frame = await screen.findByTitle("交互式 BOM");
    window.dispatchEvent(new MessageEvent("message", { source: (frame as HTMLIFrameElement).contentWindow, data: { type: "partnest:bom-selection", token: "token-1", designators: "R1" } }));
    window.dispatchEvent(new MessageEvent("message", { source: (frame as HTMLIFrameElement).contentWindow, data: { type: "other", token: "token-1", designators: ["R1"] } }));
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(api.resolveBomSelection).not.toHaveBeenCalled();
  });

  it("ignores a stale bridge response after a newer selection", async () => {
    const pending: Array<(selection: ResolvedBomSelection) => void> = [];
    const api = makeApi({ resolveBomSelection: vi.fn((_token: string, designators: string[]) => new Promise<ResolvedBomSelection>((resolve) => pending.push(() => resolve({ session_id: "session-1", component_key: "C1", side: "top", designators })))) });
    render(<WeldingPage api={api} />);
    await screen.findByTitle("交互式 BOM");
    selectInBom(["R1", "R2"]);
    selectInBom(["R1"]);
    await waitFor(() => expect(pending).toHaveLength(2));
    pending[0]({ session_id: "session-1", component_key: "C1", side: "top", designators: ["R1", "R2"] });
    pending[1]({ session_id: "session-1", component_key: "C1", side: "top", designators: ["R1"] });
    expect(await screen.findByText("当前选择：R1")).toBeInTheDocument();
    expect(screen.queryByText("当前选择：R1, R2")).not.toBeInTheDocument();
  });

  it("ignores an in-flight bridge response after unmount", async () => {
    let resolveSelection: ((selection: ResolvedBomSelection) => void) | undefined;
    const api = makeApi({ resolveBomSelection: vi.fn(() => new Promise<ResolvedBomSelection>((resolve) => { resolveSelection = resolve; })) });
    const view = render(<WeldingPage api={api} />);
    await screen.findByTitle("交互式 BOM");
    selectInBom(["R1"]);
    await waitFor(() => expect(api.resolveBomSelection).toHaveBeenCalled());
    view.unmount();
    expect(() => resolveSelection?.({ session_id: "session-1", component_key: "C1", side: "top", designators: ["R1"] })).not.toThrow();
  });

  it("selects a bottom-side component from the top tab and switches sides", async () => {
    const mixed = {
      ...session,
      normalized: {
        source_name: "board.html",
        groups: [
          { ...session.normalized.groups[0], component_key: "TOP", name: "top-part", designators: ["R1"], placements: [{ designator: "R1", side: "top" as const, component_key: "TOP" }] },
          { ...session.normalized.groups[0], component_key: "BOT", name: "bottom-part", designators: ["C1"], placements: [{ designator: "C1", side: "bottom" as const, component_key: "BOT" }] },
        ],
      },
    };
    const resolveBomSelection = vi.fn().mockResolvedValue({ session_id: "session-1", component_key: "BOT", side: "bottom", designators: ["C1"] });
    const confirmTake = vi.fn();
    render(<WeldingPage api={makeApi({ restoreActiveWeldingSession: vi.fn().mockResolvedValue(mixed), resolveBomSelection, confirmTake })} />);

    const topRow = await screen.findByTestId("tray-row-TOP");
    const bottomRow = screen.getByTestId("tray-row-BOT");
    expect(within(topRow).getByText("顶层")).toBeInTheDocument();
    expect(within(bottomRow).getByText("底层")).toBeInTheDocument();
    expect(bottomRow).toHaveAttribute("data-selectable", "true");

    fireEvent.click(bottomRow);

    await waitFor(() => expect(resolveBomSelection).toHaveBeenCalledWith("token-1", ["C1"]));
    expect(screen.getByRole("tab", { name: "底层" })).toHaveAttribute("aria-selected", "true");
    expect(await screen.findByText("当前选择：C1")).toBeInTheDocument();
    expect(screen.queryByText("当前面无器件")).not.toBeInTheDocument();
  });

  it("pushes the current selection into the canvas for a persistent highlight", async () => {
    const api = makeApi();
    render(<WeldingPage api={api} />);
    const frame = await screen.findByTitle("交互式 BOM");
    const post = vi.spyOn((frame as HTMLIFrameElement).contentWindow as Window, "postMessage").mockImplementation(() => undefined);

    selectInBom(["R1", "R2"]);

    await waitFor(() => expect(post).toHaveBeenCalledWith(
      { type: "partnest:bom-highlight", token: "token-1", designators: ["R1", "R2"], side: "top" }, "*"));
  });

  it("canvas controls zoom to the selection or reset the view on demand", async () => {
    const api = makeApi();
    render(<WeldingPage api={api} />);
    const frame = await screen.findByTitle("交互式 BOM");
    const post = vi.spyOn((frame as HTMLIFrameElement).contentWindow as Window, "postMessage").mockImplementation(() => undefined);
    const fit = screen.getByRole("button", { name: "缩放居中" });
    const reset = screen.getByRole("button", { name: "复位视图" });

    expect(fit).toBeDisabled();
    fireEvent.click(reset);
    expect(post).toHaveBeenCalledWith({ type: "partnest:bom-view", token: "token-1", action: "reset" }, "*");

    selectInBom(["R1", "R2"]);
    await waitFor(() => expect(fit).toBeEnabled());
    fireEvent.click(fit);
    expect(post).toHaveBeenCalledWith({ type: "partnest:bom-view", token: "token-1", action: "fit" }, "*");
  });

  it("sandboxes the BOM viewer to scripting and its own origin only", async () => {
    const api = makeApi();
    render(<WeldingPage api={api} />);
    const frame = await screen.findByTitle("交互式 BOM");
    // The EasyEDA viewer boots WebGL and keeps its own storage, which needs a
    // real origin; everything that could reach the host or the user stays off.
    const sandbox = frame.getAttribute("sandbox") ?? "";
    expect(sandbox.split(/\s+/)).toContain("allow-scripts");
    expect(sandbox.split(/\s+/)).toContain("allow-same-origin");
    for (const forbidden of ["allow-top-navigation", "allow-popups", "allow-forms", "allow-downloads", "allow-modals"]) {
      expect(sandbox).not.toContain(forbidden);
    }
  });

  it("does not silently use a same-named part when an authoritative LCSC is unmatched", async () => {
    const api = makeApi({
      restoreActiveWeldingSession: vi.fn().mockResolvedValue({
        ...session,
        normalized: {
          ...session.normalized,
          groups: [{ ...session.normalized.groups[0], lcsc_code: "C-MISSING" }],
        },
      }),
    });
    render(<WeldingPage api={api} />);
    await screen.findByTitle("交互式 BOM");
    selectInBom(["R1", "R2"]);
    expect(await screen.findByLabelText("选择器件")).toHaveValue("");
    expect(screen.getByRole("button", { name: /确认取用/ })).toBeDisabled();
    expect(api.confirmTake).not.toHaveBeenCalled();
  });

  it("drops the BOM selection when switching to the other board side", async () => {
    const api = makeApi();
    render(<WeldingPage api={api} />);
    await screen.findByTitle("交互式 BOM");
    selectInBom(["R1", "R2"]);
    await screen.findByText("当前选择：R1, R2");

    fireEvent.click(screen.getByRole("tab", { name: "底层" }));
    expect(screen.getByText("请在 BOM 中选择器件")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /确认取用/ })).not.toBeInTheDocument();
    expect(api.confirmTake).not.toHaveBeenCalled();

    // 底层仍可取用，但必须重新选择位号。
    fireEvent.click(screen.getByTestId("tray-row-C1"));
    await waitFor(() => expect(api.resolveBomSelection).toHaveBeenLastCalledWith("token-1", ["R3"]));
  });

  it("blocks a take whose selected designators were all consumed already", async () => {
    const api = makeApi({
      getWeldingProgress: vi.fn().mockResolvedValue([{
        session_id: "session-1", component_key: "C1", side: "top", part_id: "part-1",
        required_quantity: 2, consumed_quantity: 2, taken_quantity: 2,
        confirmed_designators: ["R1", "R2"], status: "taken",
      }]),
    });
    render(<WeldingPage api={api} />);
    await screen.findByTitle("交互式 BOM");
    selectInBom(["R1", "R2"]);

    expect(await screen.findByText("所选位号在当前板面均已取用")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /确认取用/ })).toBeDisabled();
    expect(api.confirmTake).not.toHaveBeenCalled();
  });

  it("drives a tabular BOM from the tray because it has no interactive canvas", async () => {
    const api = makeApi({
      restoreActiveWeldingSession: vi.fn().mockResolvedValue(tabularSession),
      resolveBomSelection: vi.fn().mockResolvedValue({ session_id: "session-2", component_key: "C1", side: null, designators: ["R1", "R2"] }),
    });
    render(<WeldingPage api={api} />);

    expect(await screen.findByText("表格 BOM 没有交互式画布")).toBeInTheDocument();
    expect(screen.queryByTitle("交互式 BOM")).not.toBeInTheDocument();

    const row = screen.getByTestId("tray-row-C1");
    expect(row).toHaveAttribute("data-selectable", "true");
    expect(within(row).getByText("未标注")).toBeInTheDocument();
    fireEvent.click(row);

    await waitFor(() => expect(api.resolveBomSelection).toHaveBeenCalledWith("token-1", ["R1", "R2"]));
    expect(await screen.findByText((content) => content.includes("当前选择：R1, R2"))).toBeInTheDocument();
    // Without a recorded side the operator's current tab decides the take.
    expect(screen.getByRole("tab", { name: "顶层" })).toHaveAttribute("aria-selected", "true");
  });
});
