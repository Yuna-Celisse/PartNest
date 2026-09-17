import { useCallback, useEffect, useMemo, useRef, useState, type KeyboardEvent } from "react";
import { confirm as confirmDialog } from "@tauri-apps/plugin-dialog";
import { cachedBomUrl, desktopApi, errorMessage, normalizePart, type BomSide, type CachedBomSession, type ConfirmTakeInput, type DesktopApi, type Part, type ProjectSummary, type ResolvedBomSelection, type WeldingProgress } from "../../app/tauri";
import { matchBomGroup, type InventoryPart } from "@partnest/domain";
import { ProjectChooser } from "./ProjectChooser";
import { BomFrame } from "./BomFrame";
import { ComponentTray } from "./ComponentTray";
import { TakePanel } from "./TakePanel";
import { useBomBridge } from "./useBomBridge";
import { useResizableColumns } from "./useResizableColumns";
import { StatusBadge } from "../../components/ui/StatusBadge";

export type WeldingApi = Pick<DesktopApi, "restoreActiveWeldingSession" | "resolveBomSelection" | "listParts" | "confirmTake" | "getWeldingProgress">
  & Partial<Pick<DesktopApi, "listProjects" | "openProjectWelding" | "endWeldingSession">>;

/** 板面页签的顺序，同时是键盘导航的环形顺序。 */
const sideOrder: (BomSide | "all")[] = ["top", "bottom", "all"];

export function WeldingPage({ api = desktopApi, navigate }: { api?: WeldingApi; navigate?: (path: string) => void }) {
  const frameRef = useRef<HTMLIFrameElement>(null);
  const sideTabsRef = useRef<HTMLDivElement>(null);
  const [session, setSession] = useState<CachedBomSession | null>(null);
  const [parts, setParts] = useState<Part[]>([]);
  const [progress, setProgress] = useState<WeldingProgress[]>([]);
  const [selection, setSelection] = useState<ResolvedBomSelection | null>(null);
  const [selectedPartId, setSelectedPartId] = useState<string | null>(null);
  const [side, setSide] = useState<BomSide | "all">("top");
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState("");
  const [trayCollapsed, setTrayCollapsed] = useState(false);
  const [chooserOpen, setChooserOpen] = useState(false);
  const [chooserProjects, setChooserProjects] = useState<ProjectSummary[]>([]);
  const [chooserError, setChooserError] = useState("");
  const [activating, setActivating] = useState(false);
  const [loading, setLoading] = useState(true);
  const [retryNonce, setRetryNonce] = useState(0);
  const columns = useResizableColumns({ component: 180, package: 120, quantity: 96, side: 100 });

  useEffect(() => {
    let active = true;
    setLoading(true);
    setError("");
    void Promise.all([api.restoreActiveWeldingSession(), api.listParts()]).then(([restored, listed]) => {
      if (!active) return;
      setSession(restored);
      setParts(listed.map((part) => normalizePart(part)));
      if (restored) {
        void api.getWeldingProgress(restored.session_id)
          .then((items) => active && setProgress(items))
          .catch((cause) => active && setError(errorMessage(cause)));
      } else {
        setProgress([]);
      }
    }).catch((cause) => {
      if (!active) return;
      setSession(null);
      setParts([]);
      setProgress([]);
      setError(errorMessage(cause));
    }).finally(() => active && setLoading(false));
    return () => { active = false; };
  }, [api, retryNonce]);

  const selectedGroup = useMemo(() => selection && session?.normalized.groups.find((group) => group.component_key === selection.component_key) || null, [selection, session]);
  const authoritativeMatch = useMemo(() => {
    if (!selectedGroup) return { kind: "none" as const };
    const inventory: InventoryPart[] = parts.map((part) => ({
      id: part.id,
      name: part.name,
      package: part.package ?? "",
      mpn: part.mpn ?? "",
      lcscCode: part.lcsc_code ?? "",
      value: part.name,
    }));
    return matchBomGroup({
      componentKey: selectedGroup.component_key,
      name: selectedGroup.name,
      value: selectedGroup.value,
      package: selectedGroup.package,
      manufacturer: selectedGroup.manufacturer,
      mpn: selectedGroup.mpn,
      lcscCode: selectedGroup.lcsc_code,
      placements: [],
      extraFields: selectedGroup.extra_fields,
    }, inventory);
  }, [parts, selectedGroup]);
  const selectedPart = useMemo(() => {
    if (!selectedGroup) return null;
    if (selectedPartId) return parts.find((part) => part.id === selectedPartId) ?? null;
    if (authoritativeMatch.kind === "exact-lcsc" || authoritativeMatch.kind === "exact-mpn") {
      return parts.find((part) => part.id === authoritativeMatch.partId) ?? null;
    }
    return null;
  }, [authoritativeMatch, parts, selectedGroup, selectedPartId]);
  const selectableParts = useMemo(() => authoritativeMatch.kind === "candidate"
    ? parts.filter((part) => authoritativeMatch.partIds.includes(part.id))
    : parts, [authoritativeMatch, parts]);
  // 一次选择只属于一个板面。切换页签时丢弃选择，而不是拿另一面的位号顶替，
  // 否则会扣减操作者根本没选的位号。BOM 未标注板面时，选择归属于当前页签。
  const selectionSide = selection?.side ?? side;
  const selectedDesignators = selectedGroup && selection && side !== "all" && side === selectionSide
    ? selection.designators
    : [];
  const confirmedForSide = selection && side !== "all"
    ? progress.find((item) => item.component_key === selection.component_key && item.side === side)?.confirmed_designators ?? []
    : [];
  const pendingDesignators = selectedDesignators.filter((designator) => !confirmedForSide.includes(designator));
  const hasSideInfo = useMemo(() => (session?.normalized.groups ?? []).some((group) => group.placements.some((placement) => placement.side)), [session]);
  // Table BOMs have no document to render; the tray drives their selection.
  const canvasSrc = session?.cache_path ? cachedBomUrl(session.cache_path) : null;
  // 查看器靠点它自己的行来持续高亮元件、靠 layer-switch 翻面，所以把当前选择和板面一起推进画布。
  const designatorKey = selectedDesignators.join(",");
  const highlightRef = useRef<{ designators: string[]; side: BomSide | "all" }>({ designators: [], side: selectionSide });
  highlightRef.current = { designators: selectedDesignators, side: selectionSide };
  const postView = useCallback((action: "fit" | "reset") => {
    const content = frameRef.current?.contentWindow;
    if (!content || !session) return;
    content.postMessage({ type: "partnest:bom-view", token: session.token, action }, "*");
  }, [session]);
  const postHighlight = useCallback(() => {
    const content = frameRef.current?.contentWindow;
    if (!content || !session || highlightRef.current.designators.length === 0) return;
    content.postMessage({
      type: "partnest:bom-highlight",
      token: session.token,
      designators: highlightRef.current.designators,
      side: highlightRef.current.side,
    }, "*");
  }, [session]);
  useEffect(() => { postHighlight(); }, [postHighlight, designatorKey]);
  const sideTotals = useMemo(() => {
    const totals: Record<BomSide, number> = { top: 0, bottom: 0 };
    for (const group of session?.normalized.groups ?? []) {
      for (const placement of group.placements) {
        if (placement.side) totals[placement.side] += 1;
      }
    }
    return totals;
  }, [session]);

  const onResolved = useCallback((resolved: ResolvedBomSelection) => {
    setSelection(resolved);
    // A table BOM may not record a board side; the operator's tab decides then.
    if (resolved.side) setSide(resolved.side);
    setSelectedPartId(null);
    setError("");
    setNotice("");
  }, []);
  const chooseSide = useCallback((value: BomSide | "all") => {
    setSide(value);
    setNotice("");
    if (value === "all") return;
    // 全部只是查看双面状态。切换到另一面时必须丢弃原选择，
    // 否则会把操作者没有选的位号提交上去。
    if (selection && selection.side !== value) {
      setSelection(null);
      setSelectedPartId(null);
    }
  }, [selection]);
  const onBridgeError = useCallback((cause: unknown) => setError(errorMessage(cause)), []);
  /* 页签组采用 ARIA 的漫游焦点约定：只有当前页签能被 Tab 键停住，
     左右箭头与 Home/End 在页签之间移动焦点并同步切换板面。 */
  const focusSideTab = useCallback((index: number) => {
    const tabs = sideTabsRef.current?.querySelectorAll<HTMLElement>("[role='tab']");
    tabs?.item(index)?.focus();
  }, []);
  const onSideTabsKeyDown = useCallback((event: KeyboardEvent<HTMLDivElement>) => {
    const current = sideOrder.indexOf(side);
    const next = event.key === "ArrowRight" ? (current + 1) % sideOrder.length
      : event.key === "ArrowLeft" ? (current - 1 + sideOrder.length) % sideOrder.length
      : event.key === "Home" ? 0
      : event.key === "End" ? sideOrder.length - 1 : -1;
    if (next < 0) return;
    event.preventDefault();
    chooseSide(sideOrder[next]);
    focusSideTab(next);
  }, [chooseSide, focusSideTab, side]);
  const onSelectDesignators = useCallback((designators: string[]) => {
    if (!session || designators.length === 0) return;
    // 位号自己带着板面归属，宿主解析后由 onResolved 切到对应页签。
    void api.resolveBomSelection(session.token, designators)
      .then(onResolved)
      .catch(onBridgeError);
  }, [api, onBridgeError, onResolved, session]);
  useBomBridge({ frameRef, session: canvasSrc ? session : null, api, onResolved, onError: onBridgeError });

  async function confirm(quantity: number) {
    if (!session || !selection || !selectedGroup || !selectedPart || side === "all" || selectedDesignators.length === 0) return;
    setBusy(true); setError(""); setNotice("");
    const input: ConfirmTakeInput = {
      session_id: session.session_id,
      component_key: selection.component_key,
      side,
      designators: selectedDesignators,
      bom_quantity: selectedDesignators.length,
      take_quantity: quantity,
      part_id: selectedPart.id,
      expected_part_version: selectedPart.version,
    };
    try {
      const result = await api.confirmTake(input);
      const [listed, updatedProgress] = await Promise.all([api.listParts(), api.getWeldingProgress(session.session_id)]);
      setParts(listed.map((item) => {
        const normalized = normalizePart(item);
        return normalized.id === result.part_id ? { ...normalized, version: Math.max(normalized.version, result.part_version) } : normalized;
      }));
      setProgress(updatedProgress);
      setNotice("已确认取用");
    } catch (cause) {
      setError(errorMessage(cause));
    } finally { setBusy(false); }
  }

  async function openChooser() {
    setChooserError(""); setChooserProjects([]); setChooserOpen(true);
    if (!api.listProjects) return;
    try { setChooserProjects(await api.listProjects()); }
    catch (cause) { setChooserError(errorMessage(cause)); }
  }

  async function pickProject(project: ProjectSummary) {
    if (!api.openProjectWelding) return;
    setActivating(true); setChooserError("");
    let opened: CachedBomSession;
    try {
      opened = await api.openProjectWelding(project.id);
    } catch (cause) {
      setActivating(false); setChooserError(errorMessage(cause)); return;
    }
    setActivating(false); setChooserOpen(false);
    setSession(opened); setSelection(null); setSelectedPartId(null); setError(""); setNotice("");
    try { setProgress(await api.getWeldingProgress(opened.session_id)); }
    catch (cause) { setError(errorMessage(cause)); }
  }

  async function exitSession() {
    if (!session || !api.endWeldingSession) return;
    setError(""); setNotice("");
    const name = session.project_name || session.original_name;
    try {
      // Destructive actions use the async dialog plugin, not window.confirm.
      const accepted = await confirmDialog(
        `退出项目「${name}」的当前焊接？已取用的器件不会退回库存；如需纠正请在「库存流水」撤销对应取用。重新打开该项目的焊接会开始新的会话。`,
        { title: "退出当前焊接", kind: "warning", okLabel: "退出", cancelLabel: "取消" },
      );
      if (!accepted) return;
      await api.endWeldingSession(session.session_id);
    } catch (cause) { setError(errorMessage(cause)); return; }
    setSession(null); setProgress([]); setSelection(null); setSelectedPartId(null);
    setNotice(""); setError("");
  }

  const noSession = error ? <>
    <strong>项目焊接会话加载失败</strong>
    <p role="alert">{error}</p>
    <div className="welding-empty-state__actions">
      <button className="pn-button pn-button--primary" type="button" onClick={() => setRetryNonce((value) => value + 1)}>重试</button>
      <button className="pn-button pn-button--secondary" type="button" onClick={() => void openChooser()}>选择项目</button>
      {navigate && <button className="pn-button pn-button--secondary" type="button" onClick={() => navigate("/projects")}>去项目页重新导入</button>}
    </div>
  </> : <>
    <strong>暂无进行中的焊接项目</strong>
    <p className="welding-empty-state__hint">从已导入的项目里选一个开始焊接；需要新的项目时先到「项目」页导入 BOM。</p>
    <div className="welding-empty-state__actions">
      <button className="pn-button pn-button--primary" type="button" onClick={() => void openChooser()}>选择项目</button>
      {navigate && <button className="pn-button pn-button--secondary" type="button" onClick={() => navigate("/projects")}>去项目页</button>}
    </div>
  </>;

  return <section aria-label="焊接工作台">
    {loading ? <div className="welding-empty-state" role="status"><strong>正在加载焊接项目</strong></div> : !session ? <div className="welding-empty-state">{noSession}</div> : <div className="welding-workspace" data-testid="welding-layout" data-split="65-35">
      <header className="welding-workspace__header">
        <div>
          <p className="welding-workspace__eyebrow">焊接工作台</p>
          <h1>{session.project_name || session.original_name}</h1>
          <p className="welding-workspace__file" title={session.original_name}>{session.original_name}</p>
        </div>
        <div className="welding-workspace__meta" aria-label="BOM概览"><span>{session.normalized.groups.length} 个器件组</span><span>{session.normalized.groups.reduce((total, group) => total + group.designators.length, 0)} 个位号</span></div>
        <div className="welding-workspace__actions"><button className="pn-button pn-button--secondary" type="button" onClick={() => void openChooser()}>切换项目</button><button className="pn-button pn-button--secondary" type="button" onClick={() => void exitSession()}>退出当前焊接</button></div>
      </header>
      <div className="welding-bom bom-canvas-light" data-bom-canvas>{canvasSrc ? <BomFrame src={canvasSrc} frameRef={frameRef} onLoad={postHighlight} /> : <div className="welding-canvas-note" role="status"><strong>表格 BOM 没有交互式画布</strong><span>请在下方「器件列表」点击器件行选择位号，再在右侧确认取用。</span></div>}{canvasSrc ? <div className="welding-canvas-controls"><button className="pn-button pn-button--secondary" type="button" disabled={selectedDesignators.length === 0} onClick={() => postView("fit")}>缩放居中</button><button className="pn-button pn-button--ghost" type="button" onClick={() => postView("reset")}>复位视图</button></div> : null}</div>
      <div className="welding-right" id="welding-side-panel" role="tabpanel" aria-labelledby={`welding-side-tab-${side}`} tabIndex={0}>
        <div className="welding-sides" role="tablist" aria-label="板面" aria-orientation="horizontal" ref={sideTabsRef} onKeyDown={onSideTabsKeyDown}>{sideOrder.map((value) => <button key={value} id={`welding-side-tab-${value}`} className={`pn-button ${side === value ? "pn-button--primary" : "pn-button--secondary"}`} type="button" role="tab" aria-controls="welding-side-panel" aria-selected={side === value} tabIndex={side === value ? 0 : -1} onClick={() => chooseSide(value)}>{value === "top" ? "顶层" : value === "bottom" ? "底层" : "全部"}</button>)}</div>
        <div className="welding-progress-summary" aria-label="焊接进度">
          {(["top", "bottom"] as BomSide[]).map((value) => {
            const item = selectedGroup ? progress.find((entry) => entry.component_key === selectedGroup.component_key && entry.side === value) : undefined;
            const status = item?.status ?? "pending";
            const label = status === "taken" ? "已取用" : status === "partial" ? "部分取用" : "待取用";
            return <span key={value}><span>{value === "top" ? "顶层" : "底层"}</span><strong>{item ? `${item.consumed_quantity}/${item.required_quantity}` : "0/—"}</strong><StatusBadge tone={status === "taken" ? "success" : status === "partial" ? "warning" : "neutral"}>{label}</StatusBadge></span>;
          })}
        </div>
        {error && (side === "all" || !selection || !selectedGroup) && <p className="welding-feedback welding-feedback--error" role="alert">{error}</p>}
        {selection && selectedGroup ? <>
          <section className="welding-selection-card" aria-label="当前选择">
            <div><span className="welding-selection-card__label">当前器件</span><strong>{selectedGroup.name || selectedGroup.value || selectedGroup.component_key}</strong><span className="welding-selection-card__key">{selectedGroup.component_key}</span></div>
            <p className="welding-selection">当前选择：{selectedDesignators.length ? selectedDesignators.join(", ") : "当前面无器件"}</p>
          </section>
          <TakePanel group={selectedGroup} side={side} designators={selectedDesignators} pendingDesignators={pendingDesignators} part={selectedPart} parts={selectableParts} progress={progress} onPartChange={setSelectedPartId} onConfirm={confirm} error={error} busy={busy} />
          {notice && <p className="welding-feedback welding-feedback--success" role="status">{notice}</p>}
        </> : <><p>请在 BOM 中选择器件</p>{side !== "all" && hasSideInfo && sideTotals[side] === 0 && <p className="welding-empty-side" role="status">当前面无器件</p>}</>}
      </div>
      <ComponentTray groups={session.normalized.groups} side={side} activeComponentKey={selection?.component_key ?? null} widths={columns.widths} onResizeStart={columns.startResize} onResizeKey={columns.adjustWidth} onSelectDesignators={onSelectDesignators} collapsed={trayCollapsed} onToggle={() => setTrayCollapsed((value) => !value)} />
    </div>}
    <ProjectChooser open={chooserOpen} projects={chooserProjects} busy={activating} error={chooserError} onRequestClose={() => setChooserOpen(false)} onPick={(project) => void pickProject(project)} />
  </section>;
}
