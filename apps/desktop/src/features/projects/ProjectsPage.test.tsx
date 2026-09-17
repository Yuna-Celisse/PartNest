import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { MemoryRouter } from "react-router-dom";
import { confirm as confirmDialog } from "@tauri-apps/plugin-dialog";
import { ProjectsPage, type ProjectImportApi, type ProjectImportSource } from "./ProjectsPage";
import type { ImportedProject, Part, ProjectSummary } from "../../app/tauri";

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

const mixedBom = { source_name: "bom.csv", groups: [
  { component_key: "lcsc:C1", name: "R1", value: "10k", package: "0603", manufacturer: "", mpn: "", lcsc_code: "C1", quantity: 5, designators: ["R1"], placements: [], extra_fields: {} },
  { component_key: "candidate", name: "LED", value: "red", package: "0603", manufacturer: "", mpn: "", lcsc_code: "", quantity: 2, designators: ["D1", "D2"], placements: [], extra_fields: {} },
  { component_key: "none", name: "MCU", value: "x", package: "QFN", manufacturer: "", mpn: "missing", lcsc_code: "", quantity: 1, designators: ["U1"], placements: [], extra_fields: {} },
] };

const candidateBom = { source_name: "bom.csv", groups: [
  { component_key: "candidate", name: "LED", value: "red", package: "0603", manufacturer: "", mpn: "", lcsc_code: "", quantity: 2, designators: ["D1"], placements: [], extra_fields: {} },
] };

const project: ProjectSummary = { id: "p-1", name: "板 A", original_name: "board.csv", kind: "tabular", has_table: true, created_at: "2026-09-14T00:00:00Z", active: false };

/** An interactive export with no parts table yet, so it can be supplemented. */
const bareBoard: ProjectSummary = { ...project, id: "p-2", name: "裸板", original_name: "board.html", kind: "interactive", has_table: false };

const imported = (overrides: Partial<ImportedProject> = {}): ImportedProject => ({
  project_id: "p-1", name: "board", original_name: "board.csv", cache_name: "hash.csv", kind: "tabular", has_table: true, normalized: ready.bom, ...overrides,
});

// The label also carries its hint text, so match the option by its title.
const sourceLabel: Record<ProjectImportSource, RegExp> = {
  interactive: /从可交互式 BOM 导入/,
  tabular: /从普通 BOM 表导入/,
};

function api(overrides: Partial<ProjectImportApi> = {}): ProjectImportApi {
  return {
    inspectTabularBom: vi.fn().mockResolvedValue(ready),
    previewInteractiveBom: vi.fn().mockResolvedValue(ready),
    importProject: vi.fn().mockResolvedValue(imported()),
    supplementProject: vi.fn().mockResolvedValue(imported()),
    listParts: vi.fn().mockResolvedValue([part()]),
    ...overrides,
  };
}

function renderPage(ui: JSX.Element) {
  return render(<MemoryRouter initialEntries={["/projects"]}>{ui}</MemoryRouter>);
}

/** Drive the real import path: open the dialog, choose the source, pick the file, confirm. */
async function importFile(path: string, options: { companion?: string; source?: ProjectImportSource } = {}) {
  const source = options.source ?? (path.toLowerCase().endsWith(".html") ? "interactive" : "tabular");
  fireEvent.click(screen.getByRole("button", { name: "导入项目" }));
  const dialog = await screen.findByRole("dialog", { name: "导入项目" });
  fireEvent.click(within(dialog).getByRole("radio", { name: sourceLabel[source] }));
  fireEvent.click(within(dialog).getByRole("button", { name: "选择文件" }));
  await waitFor(() => expect(within(dialog).getByText(path.split(/[\\/]/).pop() ?? path)).toBeInTheDocument());
  if (options.companion) {
    fireEvent.click(within(dialog).getByRole("button", { name: "配套表格" }));
    await waitFor(() => expect(within(dialog).getByText(`配套：${options.companion}`)).toBeInTheDocument());
  }
  fireEvent.click(within(dialog).getByRole("button", { name: "确定导入" }));
  await waitFor(() => expect(screen.queryByRole("dialog", { name: "导入项目" })).not.toBeInTheDocument());
}

/** The mapping dialog the page opens above the workspace. */
async function mappingDialog() {
  return screen.findByRole("dialog", { name: "字段映射" });
}

function buttonIn(root: HTMLElement, label: string) {
  const match = Array.from(root.querySelectorAll("button")).find((button) => button.textContent === label);
  if (!match) throw new Error(`no ${label} button inside ${root.getAttribute("aria-label") ?? "the dialog"}`);
  return match;
}

describe("ProjectsPage", () => {
  it("gives an unloaded project workspace a compact empty state", () => {
    renderPage(<ProjectsPage api={api()} />);
    const workspace = screen.getByRole("region", { name: "项目工作区" });
    expect(workspace).toHaveClass("project-empty-state");
    expect(workspace).toHaveTextContent("未选择项目");
    expect(screen.getByRole("region", { name: "项目管理" })).not.toHaveClass("project-empty-state");
  });

  it("opens a two-source dialog before anything is picked", async () => {
    const pickFile = vi.fn().mockResolvedValue(null);
    renderPage(<ProjectsPage api={api()} pickFile={pickFile} />);
    const toolbar = screen.getByRole("toolbar", { name: "项目工具栏" });
    expect(toolbar).toContainElement(screen.getByRole("button", { name: "导入项目" }));
    expect(screen.getByText("未选择项目").closest(".project-empty-state__content")?.querySelector("button")).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "导入项目" }));
    const dialog = await screen.findByRole("dialog", { name: "导入项目" });
    expect(pickFile).not.toHaveBeenCalled();
    expect(within(dialog).getByRole("radio", { name: sourceLabel.interactive })).toBeChecked();
    expect(within(dialog).getByRole("radio", { name: sourceLabel.tabular })).not.toBeChecked();
    expect(within(dialog).getByRole("button", { name: "确定导入" })).toBeDisabled();

    fireEvent.click(within(dialog).getByRole("button", { name: "取消" }));
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "导入项目" })).not.toBeInTheDocument());
  });

  it("filters the file picker by the chosen source", async () => {
    const pickFile = vi.fn().mockResolvedValue(null);
    renderPage(<ProjectsPage api={api()} pickFile={pickFile} />);

    fireEvent.click(screen.getByRole("button", { name: "导入项目" }));
    const dialog = await screen.findByRole("dialog", { name: "导入项目" });
    fireEvent.click(within(dialog).getByRole("button", { name: "选择文件" }));
    expect(pickFile).toHaveBeenLastCalledWith("interactive");

    fireEvent.click(within(dialog).getByRole("radio", { name: sourceLabel.tabular }));
    fireEvent.click(within(dialog).getByRole("button", { name: "选择文件" }));
    expect(pickFile).toHaveBeenLastCalledWith("tabular");
  });

  it("blocks a file that does not match the chosen source", async () => {
    const pickFile = vi.fn().mockResolvedValue("notes.txt");
    const importProject = vi.fn();
    renderPage(<ProjectsPage api={api({ importProject })} pickFile={pickFile} />);
    fireEvent.click(screen.getByRole("button", { name: "导入项目" }));
    const dialog = await screen.findByRole("dialog", { name: "导入项目" });
    fireEvent.click(within(dialog).getByRole("radio", { name: sourceLabel.tabular }));
    fireEvent.click(within(dialog).getByRole("button", { name: "选择文件" }));

    expect(await within(dialog).findByRole("alert")).toHaveTextContent("与导入来源不符");
    expect(within(dialog).getByRole("button", { name: "确定导入" })).toBeDisabled();
    expect(importProject).not.toHaveBeenCalled();
  });

  it("imports the confirmed file as a project", async () => {
    const importProject = vi.fn().mockResolvedValue(imported());
    const listProjects = vi.fn().mockResolvedValue([project]);
    const navigate = vi.fn();
    renderPage(<ProjectsPage api={api({ importProject, listProjects })} pickFile={vi.fn().mockResolvedValue("C:/boms/board.csv")} navigate={navigate} />);

    await importFile("C:/boms/board.csv", { source: "tabular" });

    await waitFor(() => expect(importProject).toHaveBeenCalledWith({ source_path: "C:/boms/board.csv" }));
    expect(await screen.findByRole("heading", { name: "缺料分析" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "板 A" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "打开焊接工作台" })).toBeEnabled();
  });

  it("opens the welding workspace for the imported project", async () => {
    const openProjectWelding = vi.fn().mockResolvedValue({ session_id: "s-1" });
    const navigate = vi.fn();
    renderPage(<ProjectsPage api={api({ openProjectWelding })} pickFile={vi.fn().mockResolvedValue("board.csv")} navigate={navigate} />);

    await importFile("board.csv", { source: "tabular" });
    fireEvent.click(await screen.findByRole("button", { name: "打开焊接工作台" }));

    await waitFor(() => expect(openProjectWelding).toHaveBeenCalledWith("p-1"));
    expect(navigate).toHaveBeenCalledWith("/welding");
  });

  it("reports an import failure without pretending the project exists", async () => {
    const importProject = vi.fn().mockRejectedValue(new Error("该文件不是有效的 BOM"));
    renderPage(<ProjectsPage api={api({ importProject })} pickFile={vi.fn().mockResolvedValue("board.csv")} />);

    await importFile("board.csv", { source: "tabular" });

    expect(await screen.findByRole("alert")).toHaveTextContent("不是有效的 BOM");
    expect(screen.queryByRole("button", { name: "打开焊接工作台" })).not.toBeInTheDocument();
  });

  it("lists projects and reloads a stored analysis from a row", async () => {
    const listProjects = vi.fn().mockResolvedValue([project]);
    const analyzeProject = vi.fn().mockResolvedValue(ready);
    renderPage(<ProjectsPage api={api({ listProjects, analyzeProject })} />);

    fireEvent.click(await screen.findByRole("button", { name: /板 A/ }));
    expect(analyzeProject).toHaveBeenCalledWith("p-1");
    expect(await screen.findByRole("heading", { name: "缺料分析" })).toBeInTheDocument();
    expect(screen.getByText("点击项目查看分析，右击可重命名或删除")).toBeInTheDocument();
  });

  it("shows which project is welding right now", async () => {
    const listProjects = vi.fn().mockResolvedValue([{ ...project, active: true }]);
    renderPage(<ProjectsPage api={api({ listProjects })} />);
    expect(await screen.findByText("焊接中")).toBeInTheDocument();
  });

  it("opens welding straight from a project row's right-click menu", async () => {
    const listProjects = vi.fn().mockResolvedValue([project]);
    const openProjectWelding = vi.fn().mockResolvedValue({ session_id: "s-1" });
    const navigate = vi.fn();
    renderPage(<ProjectsPage api={api({ listProjects, openProjectWelding })} navigate={navigate} />);

    fireEvent.contextMenu(await screen.findByRole("button", { name: /板 A/ }));
    fireEvent.click(screen.getByRole("menuitem", { name: "打开焊接工作台" }));
    await waitFor(() => expect(openProjectWelding).toHaveBeenCalledWith("p-1"));
    expect(navigate).toHaveBeenCalledWith("/welding");
  });

  it("deletes a project from its context menu after confirming", async () => {
    const listProjects = vi.fn()
      .mockResolvedValueOnce([project])
      .mockResolvedValue([]);
    const removeProject = vi.fn().mockResolvedValue(undefined);
    vi.mocked(confirmDialog).mockResolvedValue(true);
    renderPage(<ProjectsPage api={api({ listProjects, removeProject })} />);

    fireEvent.contextMenu(await screen.findByRole("button", { name: /板 A/ }));
    fireEvent.click(screen.getByRole("menuitem", { name: "删除项目" }));

    await waitFor(() => expect(removeProject).toHaveBeenCalledWith("p-1"));
    expect(confirmDialog).toHaveBeenCalledTimes(1);
    await waitFor(() => expect(screen.queryByRole("button", { name: /板 A/ })).not.toBeInTheDocument());
    expect(screen.getByText(/暂无项目/)).toBeInTheDocument();
  });

  it("keeps the project when deleting it is cancelled", async () => {
    const listProjects = vi.fn().mockResolvedValue([project]);
    const removeProject = vi.fn().mockResolvedValue(undefined);
    vi.mocked(confirmDialog).mockResolvedValue(false);
    renderPage(<ProjectsPage api={api({ listProjects, removeProject })} />);

    fireEvent.contextMenu(await screen.findByRole("button", { name: /板 A/ }));
    fireEvent.click(screen.getByRole("menuitem", { name: "删除项目" }));

    await waitFor(() => expect(confirmDialog).toHaveBeenCalledTimes(1));
    expect(removeProject).not.toHaveBeenCalled();
    expect(screen.getByRole("button", { name: /板 A/ })).toBeInTheDocument();
  });

  it("reports a deletion the database refuses", async () => {
    const listProjects = vi.fn().mockResolvedValue([project]);
    const removeProject = vi.fn().mockRejectedValue(new Error("该项目已有焊接会话记录，无法删除"));
    vi.mocked(confirmDialog).mockResolvedValue(true);
    renderPage(<ProjectsPage api={api({ listProjects, removeProject })} />);

    fireEvent.contextMenu(await screen.findByRole("button", { name: /板 A/ }));
    fireEvent.click(screen.getByRole("menuitem", { name: "删除项目" }));

    expect(await screen.findByRole("alert")).toHaveTextContent("无法删除");
    expect(screen.getByRole("button", { name: /板 A/ })).toBeInTheDocument();
  });

  it("renames a project from the workspace header", async () => {
    const listProjects = vi.fn()
      .mockResolvedValueOnce([project])
      .mockResolvedValue([{ ...project, name: "主控板" }]);
    const renameProject = vi.fn().mockResolvedValue({ ...project, name: "主控板" });
    renderPage(<ProjectsPage api={api({ listProjects, renameProject, analyzeProject: vi.fn().mockResolvedValue(ready) })} />);

    fireEvent.click(await screen.findByRole("button", { name: /板 A/ }));
    await screen.findByRole("heading", { name: "缺料分析" });
    fireEvent.click(screen.getByRole("button", { name: "重命名" }));
    const dialog = await screen.findByRole("dialog", { name: "重命名项目" });
    fireEvent.change(screen.getByLabelText("项目名称"), { target: { value: "主控板" } });
    fireEvent.click(buttonIn(dialog, "保存"));

    await waitFor(() => expect(renameProject).toHaveBeenCalledWith("p-1", "主控板"));
    expect(await screen.findByRole("button", { name: /主控板/ })).toBeInTheDocument();
  });

  it("keeps mapping to itself until the fields are resolved, then imports", async () => {
    const inspectTabularBom = vi.fn()
      .mockResolvedValueOnce({ kind: "NeedsMapping" as const, headers: ["Part", "Qty"], suggestions: {} })
      .mockResolvedValueOnce(ready);
    const importProject = vi.fn().mockResolvedValue(imported());
    const pickFile = vi.fn().mockResolvedValue("board.csv");
    renderPage(<ProjectsPage api={api({ inspectTabularBom, importProject })} pickFile={pickFile} />);

    fireEvent.click(screen.getByRole("button", { name: "导入项目" }));
    const importDialog = await screen.findByRole("dialog", { name: "导入项目" });
    fireEvent.click(within(importDialog).getByRole("radio", { name: sourceLabel.tabular }));
    fireEvent.click(within(importDialog).getByRole("button", { name: "选择文件" }));
    await waitFor(() => expect(within(importDialog).getByText("board.csv")).toBeInTheDocument());
    fireEvent.click(within(importDialog).getByRole("button", { name: "确定导入" }));

    const dialog = await mappingDialog();
    expect(importProject).not.toHaveBeenCalled();
    const fields = screen.getAllByRole("combobox");
    fireEvent.change(fields[0], { target: { value: "Part" } });
    fireEvent.change(fields[1], { target: { value: "Part" } });
    expect(fields[1]).toHaveValue("");
    fireEvent.click(buttonIn(dialog, "确认映射"));

    await waitFor(() => expect(importProject).toHaveBeenCalledWith({ source_path: "board.csv", mapping: expect.objectContaining({ quantity: "Part" }) }));
  });

  it("cancelling the mapping dialog stores nothing", async () => {
    const inspectTabularBom = vi.fn().mockResolvedValue({ kind: "NeedsMapping" as const, headers: ["Part", "Qty"], suggestions: {} });
    const importProject = vi.fn();
    renderPage(<ProjectsPage api={api({ inspectTabularBom, importProject })} pickFile={vi.fn().mockResolvedValue("board.csv")} />);

    await importFile("board.csv", { source: "tabular" });
    const dialog = await mappingDialog();
    const fields = screen.getAllByRole("combobox");
    fireEvent.change(fields[0], { target: { value: "Part" } });
    expect(screen.getAllByRole("option", { name: "Part" })[1]).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "取消" }));

    await waitFor(() => expect(screen.queryByRole("dialog", { name: "字段映射" })).not.toBeInTheDocument());
    expect(importProject).not.toHaveBeenCalled();
    expect(screen.queryByRole("heading", { name: "缺料分析" })).not.toBeInTheDocument();
    expect(dialog).not.toBeVisible();
  });

  it("attaches a companion CSV to an interactive import", async () => {
    const previewInteractiveBom = vi.fn().mockResolvedValue(ready);
    const importProject = vi.fn().mockResolvedValue(imported({ kind: "interactive", original_name: "board.html", cache_name: "hash.html" }));
    renderPage(<ProjectsPage
      api={api({ previewInteractiveBom, importProject })}
      pickFile={vi.fn().mockResolvedValue("board.html")}
      pickCompanionFile={vi.fn().mockResolvedValue("parts.csv")}
    />);

    await importFile("board.html", { companion: "parts.csv" });

    await waitFor(() => expect(previewInteractiveBom).toHaveBeenLastCalledWith("board.html", "parts.csv"));
    await waitFor(() => expect(importProject).toHaveBeenLastCalledWith({ source_path: "board.html", companion_csv_path: "parts.csv" }));
  });

  it("offers the supplementary import for a project that still lacks a source", async () => {
    const listProjects = vi.fn().mockResolvedValue([bareBoard]);
    const analyzeProject = vi.fn().mockResolvedValue(ready);
    const supplementProject = vi.fn().mockResolvedValue(imported({ project_id: "p-2", name: "裸板", original_name: "board.html", cache_name: "hash.html", kind: "interactive", has_table: true }));
    const pickFile = vi.fn().mockResolvedValue("parts.xlsx");
    renderPage(<ProjectsPage api={api({ listProjects, analyzeProject, supplementProject })} pickFile={pickFile} />);

    fireEvent.click(await screen.findByRole("button", { name: /裸板/ }));
    await screen.findByRole("heading", { name: "缺料分析" });
    fireEvent.click(screen.getByRole("button", { name: "补充导入" }));

    const dialog = await screen.findByRole("dialog", { name: "补充导入" });
    expect(within(dialog).queryByRole("radio")).not.toBeInTheDocument();
    expect(within(dialog).getByText(/补充普通 BOM 表/)).toBeInTheDocument();
    expect(within(dialog).queryByRole("button", { name: "配套表格" })).not.toBeInTheDocument();
    fireEvent.click(within(dialog).getByRole("button", { name: "选择文件" }));
    expect(pickFile).toHaveBeenLastCalledWith("tabular");
    await waitFor(() => expect(within(dialog).getByText("parts.xlsx")).toBeInTheDocument());
    fireEvent.click(within(dialog).getByRole("button", { name: "确定补充导入" }));

    await waitFor(() => expect(supplementProject).toHaveBeenCalledWith({ project_id: "p-2", source_path: "parts.xlsx" }));
    expect(screen.getByRole("heading", { name: "裸板" })).toBeInTheDocument();
  });

  it("hides the supplementary import once both sources are present", async () => {
    const listProjects = vi.fn().mockResolvedValue([{ ...bareBoard, has_table: true }]);
    const analyzeProject = vi.fn().mockResolvedValue(ready);
    renderPage(<ProjectsPage api={api({ listProjects, analyzeProject })} />);

    fireEvent.click(await screen.findByRole("button", { name: /裸板/ }));
    await screen.findByRole("heading", { name: "缺料分析" });
    expect(screen.queryByRole("button", { name: "补充导入" })).not.toBeInTheDocument();
    expect(screen.getByText("交互式 + 表格")).toBeInTheDocument();
  });

  it("shows import stages and an analysis summary for the new project", async () => {
    renderPage(<ProjectsPage api={api()} pickFile={vi.fn().mockResolvedValue("board.csv")} />);

    await importFile("board.csv", { source: "tabular" });

    const steps = await screen.findByRole("navigation", { name: "项目导入流程" });
    expect(within(steps).getByText("解析与映射")).toBeInTheDocument();
    expect(screen.getByText("1 个器件组")).toBeVisible();
    expect(screen.getByText("1 个位号")).toBeVisible();
    expect(screen.getByText("1 个缺料组")).toBeVisible();
  });

  it("analyzes exact, candidate, and unmatched groups with non-negative shortages", async () => {
    const inspectTabularBom = vi.fn().mockResolvedValue({ kind: "Ready" as const, bom: mixedBom });
    const importProject = vi.fn().mockResolvedValue(imported({ normalized: mixedBom }));
    renderPage(<ProjectsPage api={api({ inspectTabularBom, importProject, listParts: vi.fn().mockResolvedValue([
      part({ id: "exact", lcsc_code: "C1", quantity: 8 }),
      part({ id: "candidate-a", name: "red", lcsc_code: "", quantity: 1 }),
      part({ id: "candidate-b", name: "red", lcsc_code: "", quantity: 4 }),
    ]) })} pickFile={vi.fn().mockResolvedValue("bom.csv")} />);

    await importFile("bom.csv", { source: "tabular" });

    expect(await screen.findByRole("heading", { name: "缺料分析" })).toBeInTheDocument();
    expect(screen.getByText("精确匹配")).toBeInTheDocument();
    expect(screen.getByText("候选匹配")).toBeInTheDocument();
    expect(screen.getByText("未匹配")).toBeInTheDocument();
    expect(screen.getAllByRole("cell", { name: "0" }).length).toBeGreaterThan(0);
    expect(screen.getAllByRole("button", { name: "确认匹配" }).length).toBeGreaterThan(0);
  });

  it("keeps a candidate unresolved until the user confirms a part", async () => {
    const bom = { ...candidateBom, groups: [{ ...candidateBom.groups[0], quantity: 5 }] };
    const inspectTabularBom = vi.fn().mockResolvedValue({ kind: "Ready" as const, bom });
    const importProject = vi.fn().mockResolvedValue(imported({ normalized: bom }));
    renderPage(<ProjectsPage api={api({ inspectTabularBom, importProject, listParts: vi.fn().mockResolvedValue([part({ id: "a", name: "red", lcsc_code: "" }), part({ id: "b", name: "red", lcsc_code: "" })]) })} pickFile={vi.fn().mockResolvedValue("bom.csv")} />);

    await importFile("bom.csv", { source: "tabular" });

    expect(await screen.findByText("候选匹配")).toBeInTheDocument();
    fireEvent.click(screen.getAllByRole("button", { name: "确认匹配" })[0]);
    await waitFor(() => expect(screen.queryByText("候选匹配")).not.toBeInTheDocument());
    expect(screen.getByText("精确匹配")).toBeInTheDocument();
    expect(screen.getByText("已确认")).toBeInTheDocument();
  });

  it("restores the previous analysis when a replacement mapping is cancelled", async () => {
    const inspectTabularBom = vi.fn()
      .mockResolvedValueOnce(ready)
      .mockResolvedValueOnce({ kind: "NeedsMapping" as const, headers: ["Part", "Qty"], suggestions: {} });
    const pickFile = vi.fn()
      .mockResolvedValueOnce("old.csv")
      .mockResolvedValueOnce("new.csv");
    renderPage(<ProjectsPage api={api({ inspectTabularBom })} pickFile={pickFile} />);

    await importFile("old.csv", { source: "tabular" });
    expect(await screen.findByRole("heading", { name: "缺料分析" })).toBeInTheDocument();
    await importFile("new.csv", { source: "tabular" });
    expect(await mappingDialog()).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "取消" }));

    await waitFor(() => expect(screen.queryByRole("dialog", { name: "字段映射" })).not.toBeInTheDocument());
    expect(screen.getByRole("heading", { name: "缺料分析" })).toBeInTheDocument();
    expect(screen.getByTitle("old.csv")).toBeInTheDocument();
  });

  it("returns to the empty workspace after cancelling the first mapping", async () => {
    const inspectTabularBom = vi.fn().mockResolvedValue({ kind: "NeedsMapping" as const, headers: ["Part", "Qty"], suggestions: {} });
    renderPage(<ProjectsPage api={api({ inspectTabularBom })} pickFile={vi.fn().mockResolvedValue("new.csv")} />);

    await importFile("new.csv", { source: "tabular" });
    expect(await mappingDialog()).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "取消" }));

    await waitFor(() => expect(screen.queryByRole("dialog", { name: "字段映射" })).not.toBeInTheDocument());
    expect(screen.queryByText("new.csv")).not.toBeInTheDocument();
    expect(screen.queryByRole("heading", { name: "缺料分析" })).not.toBeInTheDocument();
  });
});
