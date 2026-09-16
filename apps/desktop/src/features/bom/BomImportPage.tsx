import { FieldMappingDialog } from "./FieldMappingDialog";
import { BomAnalysisTable } from "./BomAnalysisTable";
import { useBomImport, type BomImportApi } from "./useBomImport";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { confirm as confirmDialog } from "@tauri-apps/plugin-dialog";
import { PageToolbar } from "../../components/ui/PageToolbar";
import { StatusBadge } from "../../components/ui/StatusBadge";
import { usePageActions } from "../../app/AppShell";
import { desktopApi, errorMessage, type BomFileSummary } from "../../app/tauri";
import "../../styles/bom.css";

export type { BomImportApi };

export function BomImportPage({ api, pickFile, pickCompanionFile }: { api?: BomImportApi; pickFile?: () => Promise<string | null>; pickCompanionFile?: () => Promise<string | null> }) {
  const state = useBomImport({ api, pickFile, pickCompanionFile });
  const [bomFiles, setBomFiles] = useState<BomFileSummary[]>([]);
  const [menu, setMenu] = useState<{ file: BomFileSummary; x: number; y: number } | null>(null);
  const [listError, setListError] = useState("");
  const menuRef = useRef<HTMLDivElement>(null);
  const listBomFiles = api?.listBomFiles ?? desktopApi.listBomFiles;
  const removeBomFile = api?.removeBomFile ?? desktopApi.removeBomFile;
  const refreshList = useCallback(async () => {
    if (!listBomFiles) return;
    try { setBomFiles(await listBomFiles()); } catch { /* keep the previous list visible */ }
  }, [listBomFiles]);
  useEffect(() => { void refreshList(); }, [refreshList, state.activated, state.historyId, state.status]);
  useEffect(() => {
    if (!menu) return;
    const onPointerDown = (event: MouseEvent) => { if (!menuRef.current?.contains(event.target as Node)) setMenu(null); };
    const onKeyDown = (event: KeyboardEvent) => { if (event.key === "Escape") setMenu(null); };
    document.addEventListener("mousedown", onPointerDown);
    document.addEventListener("keydown", onKeyDown);
    return () => { document.removeEventListener("mousedown", onPointerDown); document.removeEventListener("keydown", onKeyDown); };
  }, [menu]);
  const fileName = state.path.split(/[\\/]/).pop() || "";
  const companionFileName = state.companionPath.split(/[\\/]/).pop() || "";
  const summary = useMemo(() => ({
    groups: state.rows.length,
    designators: state.bom?.groups.reduce((total, group) => total + (group.designators?.length ?? 0), 0) ?? 0,
    shortages: state.rows.filter((row) => row.shortage > 0).length,
    unmatched: state.rows.filter((row) => row.status === "none").length,
  }), [state.bom, state.rows]);
  const extensionOf = (name: string) => name.slice(name.lastIndexOf(".")).toLowerCase();
  // A history row has no local path, so its stored file name decides the format.
  const loadedFile = bomFiles.find((file) => file.id === state.historyId);
  const sourceName = state.path || loadedFile?.original_name || "";
  const isInteractive = extensionOf(sourceName) === ".html";
  const isTabular = [".csv", ".xlsx"].includes(extensionOf(sourceName));
  const canActivate = (isInteractive || isTabular) && state.status === "ready";
  async function removeFile(file: BomFileSummary) {
    setMenu(null);
    if (!removeBomFile) return;
    setListError("");
    try {
      // Destructive actions use the async dialog plugin, not window.confirm.
      const accepted = await confirmDialog(`移除导入记录“${file.display_name}”？该记录的解析结果与缓存副本将被删除。`, { title: "移除导入记录", kind: "warning", okLabel: "移除", cancelLabel: "取消" });
      if (!accepted) return;
      await removeBomFile(file.id);
    } catch (cause) { setListError(errorMessage(cause)); return; }
    if (state.historyId === file.id) state.clearLoaded();
    await refreshList();
  }
  const toolbarActions = useMemo(() => <>
    {fileName && <span className="bom-toolbar__file" title={state.path}>{fileName}</span>}
    <button className="pn-button pn-button--primary" type="button" onClick={() => void state.chooseFile()}>导入 BOM</button>
    {isInteractive && <><button className="pn-button pn-button--ghost" type="button" onClick={() => void state.chooseCompanionFile()}>配套 CSV</button>{companionFileName && <span className="bom-toolbar__file" title={state.companionPath}>{companionFileName}</span>}</>}
    {canActivate && <button className="pn-button pn-button--primary" type="button" onClick={() => void state.activate()}>{state.activated ? "重新设为活动 BOM" : "设为活动 BOM"}</button>}
    {state.status === "loading" && <StatusBadge tone="active">解析中</StatusBadge>}
    {state.status === "ready" && <StatusBadge tone={state.activated ? "success" : "neutral"}>{state.activated ? "已设为活动 BOM" : "仅完成分析"}</StatusBadge>}
  </>, [canActivate, companionFileName, fileName, isInteractive, state.activate, state.activated, state.chooseCompanionFile, state.chooseFile, state.companionPath, state.path, state.status]);
  const inShell = usePageActions(toolbarActions);
  return <section className="bom-layout" aria-label="BOM 分析">
    <aside className="bom-list" aria-label="BOM 列表">
      <h2>已导入 BOM</h2>
      {listError && <p className="bom-list__error" role="alert">{listError}</p>}
      {bomFiles.length === 0 ? <p>暂无已导入 BOM</p> : <>
        <p className="bom-list__hint">点击记录载入分析，右击可移除</p>
        {bomFiles.map((file) => <button
          key={file.id}
          className="bom-list__item"
          type="button"
          onClick={() => void state.loadCached(file.id)}
          onContextMenu={(event) => { event.preventDefault(); setMenu({ file, x: event.clientX, y: event.clientY }); }}
          data-active={file.active || undefined}
          data-current={state.historyId === file.id || undefined}
        ><strong>{file.display_name}</strong><span>{file.original_name}</span>{file.active && <StatusBadge tone="success">活动</StatusBadge>}</button>)}
      </>}
    </aside>
    <div className={state.status === "idle" ? "bom-empty-state" : "bom-workspace"} role="region" aria-label="BOM 工作区">
    {!inShell && <PageToolbar title="BOM 操作" actions={toolbarActions} />}
    {state.status === "idle" && <div className="bom-empty-state__content"><strong>未加载 BOM</strong><span>点击「导入 BOM」选择 HTML、CSV 或 XLSX 后开始缺料分析</span>{bomFiles.length > 0 && <span>或点击左侧「已导入 BOM」中的记录重新载入分析</span>}</div>}
    {state.status !== "idle" && <nav className="bom-progress" aria-label="BOM 导入流程">
      <span data-state="complete"><b>1</b>选择文件</span>
      <span data-state={state.status === "needsMapping" ? "active" : state.status === "loading" ? "active" : "complete"}><b>2</b>解析与映射</span>
      <span data-state={state.status === "ready" ? "active" : "pending"}><b>3</b>分析结果</span>
      {(isInteractive || isTabular) && <span data-state={state.activated ? "complete" : "pending"}><b>4</b>设为活动 BOM</span>}
    </nav>}
    {canActivate && !state.activated && <p className="bom-hint" role="status">分析不会改变焊接工作台的活动 BOM，确认后点击「设为活动 BOM」。</p>}
    {state.error && <div className="bom-error" role="alert"><span>{state.error}</span>{state.path && state.status === "error" && <button className="pn-button pn-button--ghost" type="button" onClick={() => void state.inspect(state.path)}>重新解析</button>}</div>}
    {state.status === "needsMapping" && <FieldMappingDialog headers={state.headers} mapping={state.mapping} onChange={state.setMapping} onConfirm={() => void state.submitMapping()} onCancel={state.cancelMapping} />}
    {state.status === "ready" && <>
      <section className="bom-summary" aria-label="BOM 分析汇总">
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
      className="bom-context-menu"
      role="menu"
      aria-label={`${menu.file.display_name} 操作`}
      style={{ left: menu.x, top: menu.y }}
    ><button type="button" role="menuitem" onClick={() => void removeFile(menu.file)}>移除记录</button></div>}
  </section>;
}
