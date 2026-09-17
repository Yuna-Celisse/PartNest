import { ImportProjectDialog, defaultPickCompanionFile, defaultPickFile, type ProjectImportSource } from "./ImportProjectDialog";
import { ProjectRenameDialog } from "./ProjectRenameDialog";
import { BomAnalysisTable } from "./BomAnalysisTable";
import { FieldMappingDialog } from "./FieldMappingDialog";
import { useProjectImport, type ProjectImportApi } from "./useProjectImport";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { confirm as confirmDialog } from "@tauri-apps/plugin-dialog";
import { PageToolbar } from "../../components/ui/PageToolbar";
import { StatusBadge } from "../../components/ui/StatusBadge";
import { usePageActions } from "../../app/AppShell";
import { desktopApi, errorMessage, type ProjectSummary } from "../../app/tauri";
import "../../styles/projects.css";

export type { ProjectImportApi };
export type { ProjectImportSource };

const kindLabel: Record<ProjectSummary["kind"], string> = {
  interactive: "交互式",
  tabular: "表格",
};

/** Which source the project still owes: a table for a bare export, a canvas for
 *  a table-only project, or nothing once both are present. */
const missingSource = (project: ProjectSummary): ProjectImportSource | null =>
  project.kind === "interactive" ? (project.has_table ? null : "tabular") : "interactive";

const sourcesLabel = (project: ProjectSummary): string =>
  project.kind === "interactive" && project.has_table ? "交互式 + 表格" : kindLabel[project.kind];

export function ProjectsPage({ api, pickFile, pickCompanionFile, navigate }: { api?: ProjectImportApi; pickFile?: (source: ProjectImportSource) => Promise<string | null>; pickCompanionFile?: () => Promise<string | null>; navigate?: (path: string) => void }) {
  const state = useProjectImport({ api });
  const [projects, setProjects] = useState<ProjectSummary[]>([]);
  const [menu, setMenu] = useState<{ project: ProjectSummary; x: number; y: number } | null>(null);
  const [renaming, setRenaming] = useState<{ id: string; name: string } | null>(null);
  const [importing, setImporting] = useState<{ project?: { id: string; name: string }; source?: ProjectImportSource } | null>(null);
  const [listError, setListError] = useState("");
  const [busy, setBusy] = useState(false);
  const menuRef = useRef<HTMLDivElement>(null);
  const listProjects = api?.listProjects ?? desktopApi.listProjects;
  const removeProject = api?.removeProject ?? desktopApi.removeProject;
  const renameProject = api?.renameProject ?? desktopApi.renameProject;
  const refreshList = useCallback(async () => {
    try { setProjects(await listProjects()); } catch { /* keep the previous list visible */ }
  }, [listProjects]);
  useEffect(() => { void refreshList(); }, [refreshList, state.project, state.status]);
  useEffect(() => {
    if (!menu) return;
    const onPointerDown = (event: MouseEvent) => { if (!menuRef.current?.contains(event.target as Node)) setMenu(null); };
    const onKeyDown = (event: KeyboardEvent) => { if (event.key === "Escape") setMenu(null); };
    document.addEventListener("mousedown", onPointerDown);
    document.addEventListener("keydown", onKeyDown);
    return () => { document.removeEventListener("mousedown", onPointerDown); document.removeEventListener("keydown", onKeyDown); };
  }, [menu]);
  const fileName = state.path.split(/[\\/]/).pop() || "";
  const summary = useMemo(() => ({
    groups: state.rows.length,
    designators: state.bom?.groups.reduce((total, group) => total + (group.designators?.length ?? 0), 0) ?? 0,
    shortages: state.rows.filter((row) => row.shortage > 0).length,
    unmatched: state.rows.filter((row) => row.status === "none").length,
  }), [state.bom, state.rows]);
  // The list is the single source for what a project is called right now.
  const loaded = projects.find((project) => project.id === state.project?.id) ?? null;
  const loadedName = loaded?.name ?? state.project?.name ?? "";

  async function deleteProject(project: ProjectSummary) {
    setMenu(null);
    setListError("");
    // Destructive actions use the async dialog plugin, not window.confirm.
    const accepted = await confirmDialog(`删除项目“${project.name}”？该项目的解析结果与缓存画布都会被移除；已有焊接会话记录的项目无法删除。`, { title: "删除项目", kind: "warning", okLabel: "删除", cancelLabel: "取消" });
    if (!accepted) return;
    try { await removeProject(project.id); }
    catch (cause) { setListError(errorMessage(cause)); return; }
    if (state.project?.id === project.id) state.clearLoaded();
    await refreshList();
  }

  async function submitRename(name: string) {
    if (!renaming) return;
    setListError("");
    try { await renameProject(renaming.id, name); }
    catch (cause) { setListError(errorMessage(cause)); return; }
    setRenaming(null);
    await refreshList();
  }

  const openProject = useCallback(async (project: { id: string }) => {
    setMenu(null); setListError(""); setBusy(true);
    const opened = await state.openWelding(project.id);
    setBusy(false);
    if (opened) navigate?.("/welding");
  }, [navigate, state.openWelding]);
  const openLoadedProject = useCallback(() => {
    if (state.project) void openProject(state.project);
  }, [openProject, state.project]);

  const toolbarActions = useMemo(() => <>
    {loadedName && <span className="project-toolbar__name" title={loadedName}>{loadedName}</span>}
    {!loadedName && fileName && <span className="project-toolbar__name" title={state.path}>{fileName}</span>}
    <button className="pn-button pn-button--primary" type="button" onClick={() => setImporting({})}>导入项目</button>
    {state.status === "loading" && <StatusBadge tone="active">处理中</StatusBadge>}
    {state.project && <StatusBadge tone="success">已导入</StatusBadge>}
  </>, [fileName, loadedName, state.path, state.project, state.status]);
  const inShell = usePageActions(toolbarActions);
  return <section className="project-layout" aria-label="项目管理">
    <aside className="project-list" aria-label="项目列表">
      <h2>项目列表</h2>
      {listError && <p className="project-list__error" role="alert">{listError}</p>}
      {projects.length === 0 ? <p>暂无项目，点击「导入项目」开始。</p> : <>
        <p className="project-list__hint">点击项目查看分析，右击可重命名或删除</p>
        {projects.map((project) => <button
          key={project.id}
          className="project-list__item"
          type="button"
          onClick={() => void state.loadProject(project)}
          onContextMenu={(event) => { event.preventDefault(); setMenu({ project, x: event.clientX, y: event.clientY }); }}
          data-active={project.active || undefined}
          data-current={state.project?.id === project.id || undefined}
        >
          <strong>{project.name}</strong>
          <span title={project.original_name}>{project.original_name}</span>
          <span className="project-list__meta">{sourcesLabel(project)}{project.active && <StatusBadge tone="success">焊接中</StatusBadge>}</span>
        </button>)}
      </>}
    </aside>
    <div className={state.status === "idle" ? "project-empty-state" : "project-workspace"} role="region" aria-label="项目工作区">
    {!inShell && <PageToolbar title="项目" actions={toolbarActions} />}
    {state.status === "idle" && <div className="project-empty-state__content"><strong>未选择项目</strong><span>点击「导入项目」选择交互式 BOM（HTML）或 BOM 表格（CSV、XLSX）。</span>{projects.length > 0 && <span>也可以点击左侧项目载入它已存储的分析结果。</span>}</div>}
    {state.status !== "idle" && <nav className="bom-progress" aria-label="项目导入流程">
      <span data-state={state.path || state.project ? "complete" : "active"}><b>1</b>选择文件</span>
      <span data-state={state.status === "needsMapping" || state.status === "loading" ? "active" : state.path || state.project ? "complete" : "pending"}><b>2</b>解析与映射</span>
      <span data-state={state.bom ? "complete" : "pending"}><b>3</b>缺料分析</span>
      <span data-state={state.project ? "complete" : "pending"}><b>4</b>存为项目</span>
    </nav>}
    {state.error && <div className="bom-error" role="alert"><span>{state.error}</span>{state.path && state.status === "error" && <button className="pn-button pn-button--ghost" type="button" onClick={() => void state.inspect(state.path)}>重新解析</button>}</div>}
    {state.status === "needsMapping" && <FieldMappingDialog headers={state.headers} mapping={state.mapping} onChange={state.setMapping} onConfirm={() => void state.submitMapping()} onCancel={state.cancelMapping} />}
    {state.bom && <>
      {state.project && <header className="project-header">
        <div><p className="project-header__eyebrow">项目</p><h2>{loadedName}</h2></div>
        <div className="project-header__actions">
          <button className="pn-button pn-button--primary" type="button" disabled={busy} onClick={openLoadedProject}>{loaded?.active ? "继续焊接" : "打开焊接工作台"}</button>
          {loaded && missingSource(loaded) && <button className="pn-button pn-button--secondary" type="button" onClick={() => setImporting({ project: { id: loaded.id, name: loaded.name }, source: missingSource(loaded)! })}>补充导入</button>}
          <button className="pn-button pn-button--ghost" type="button" onClick={() => setRenaming(state.project!)}>重命名</button>
        </div>
      </header>}
      <section className="bom-summary" aria-label="项目分析汇总">
        <div><span>器件组</span><strong>{summary.groups} 个器件组</strong></div>
        <div><span>位号</span><strong>{summary.designators} 个位号</strong></div>
        <div data-tone={summary.shortages > 0 ? "danger" : "success"}><span>缺料组</span><strong>{summary.shortages} 个缺料组</strong></div>
        <div data-tone={summary.unmatched > 0 ? "warning" : "success"}><span>未匹配组</span><strong>{summary.unmatched} 个未匹配组</strong></div>
      </section>
      <BomAnalysisTable rows={state.rows} onConfirmMatch={state.confirmMatch} />
    </>}
    </div>
    {menu && <div
      ref={menuRef}
      className="project-context-menu"
      role="menu"
      aria-label={`${menu.project.name} 操作`}
      style={{ left: menu.x, top: menu.y }}
    >
      <button type="button" role="menuitem" onClick={() => { const target = menu.project; setMenu(null); setRenaming(target); }}>重命名</button>
      <button type="button" role="menuitem" onClick={() => void openProject(menu.project)}>{menu.project.active ? "继续焊接" : "打开焊接工作台"}</button>
      <button type="button" role="menuitem" onClick={() => void deleteProject(menu.project)}>删除项目</button>
    </div>}
    {importing && <ImportProjectDialog
      supplement={importing.project && importing.source ? { kind: importing.source, projectName: importing.project.name } : undefined}
      pickFile={pickFile ?? defaultPickFile}
      pickCompanionFile={pickCompanionFile ?? defaultPickCompanionFile}
      onCancel={() => setImporting(null)}
      onConfirm={({ path, companionPath }) => {
        const target = importing.project;
        setImporting(null);
        void (target && importing.source ? state.supplementFrom(target.id, path) : state.importFrom(path, companionPath));
      }}
    />}
    {renaming && <ProjectRenameDialog project={renaming} busy={busy} onCancel={() => setRenaming(null)} onSubmit={(name) => { setBusy(true); void submitRename(name).finally(() => setBusy(false)); }} />}
  </section>;
}
