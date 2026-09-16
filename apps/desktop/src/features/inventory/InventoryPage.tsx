import { useEffect, useMemo, useRef, useState } from "react";
import { confirm as confirmDialog } from "@tauri-apps/plugin-dialog";
import { Box, DesktopApi, desktopApi, errorMessage, LcscPartInfo, normalizePart, Part, PartInput } from "../../app/tauri";
import { DataTable, type DataColumn } from "../../components/ui/DataTable";
import { Dialog } from "../../components/ui/Overlay";
import { FormField } from "../../components/ui/FormField";
import { usePageActions } from "../../app/AppShell";
import { PartDrawer } from "./PartDrawer";

const blankPart: PartInput = { name: "", category: "", package: "", manufacturer: "", mpn: "", lcsc_code: "", quantity: 0, box_id: null, slot: null, note: "" };
type InventoryApi = Pick<DesktopApi, "listParts" | "createPart" | "updatePart" | "adjustStock"> & Partial<Pick<DesktopApi, "deletePart" | "listBoxes" | "lookupLcsc">>;
const toInput = (part: Part): PartInput => ({ ...part, category: part.category ?? "", package: part.package ?? "", manufacturer: part.manufacturer ?? "", mpn: part.mpn ?? "", lcsc_code: part.lcsc_code ?? "", note: part.note ?? "" });

export function InventoryPage({ api = desktopApi }: { api?: InventoryApi }): JSX.Element {
  const [parts, setParts] = useState<Part[]>([]);
  const [boxes, setBoxes] = useState<Box[]>([]);
  const [search, setSearch] = useState("");
  const [form, setForm] = useState<PartInput>(blankPart);
  const [editing, setEditing] = useState<Part | null>(null);
  const [drawerOpen, setDrawerOpen] = useState(false);
  const drawerCloseRef = useRef<(() => Promise<boolean>) | null>(null);
  const [adjusting, setAdjusting] = useState<Part | null>(null);
  const [adjustmentMode, setAdjustmentMode] = useState<"increase" | "decrease">("increase");
  const [adjustmentQuantity, setAdjustmentQuantity] = useState("1");
  const [adjustmentReason, setAdjustmentReason] = useState("采购入库");
  const [morePartId, setMorePartId] = useState<string | null>(null);
  const [error, setError] = useState("");
  const [loadFailed, setLoadFailed] = useState(false);
  const [quantityText, setQuantityText] = useState("0");
  const load = async () => {
    try {
      const [partResult, boxResult] = await Promise.all([api.listParts(search), api.listBoxes ? api.listBoxes() : Promise.resolve([])]);
      setParts(partResult.map(normalizePart));
      setBoxes(boxResult);
      setLoadFailed(false);
    } catch {
      setLoadFailed(true);
    }
  };
  useEffect(() => { void load(); }, [search]);
  const setField = (field: keyof PartInput, value: string | number | null) => setForm((current) => ({ ...current, [field]: value }));
  const beginCreate = () => { setEditing(null); setForm({ ...blankPart }); setQuantityText("0"); setError(""); setDrawerOpen(true); };
  const openCreate = () => {
    if (drawerOpen) { const requestClose = drawerCloseRef.current; if (requestClose) void requestClose().then((closed) => { if (closed) beginCreate(); }); return; }
    beginCreate();
  };
  const openEdit = (part: Part) => { setEditing(part); setForm(toInput(part)); setQuantityText(String(part.quantity)); setError(""); setDrawerOpen(true); };
  const closeDrawer = () => { setDrawerOpen(false); setEditing(null); setForm({ ...blankPart }); setQuantityText("0"); };
  const boxNames = useMemo(() => new Map(boxes.map((box) => [box.id, box.name])), [boxes]);
  async function save() {
    setError("");
    try {
      if (!editing && !/^\d+$/.test(quantityText)) { setError("数量必须是非负整数"); return; }
      const input = { ...form, quantity: editing ? editing.quantity : Number(quantityText) };
      const saved = normalizePart(editing ? await api.updatePart(editing.id, editing.version, input) : await api.createPart(input));
      setParts((current) => editing ? current.map((part) => part.id === saved.id ? saved : part) : [...current, saved]);
      if (api.listBoxes) {
        try { setBoxes(await api.listBoxes()); } catch { /* 保存已完成，盒位刷新失败时保留当前数据 */ }
      }
      closeDrawer();
    } catch (cause) { setError(errorMessage(cause)); }
  }
  async function adjust() {
    if (!adjusting || !/^\d+$/.test(adjustmentQuantity) || Number(adjustmentQuantity) <= 0) { setError("数量必须是大于 0 的整数"); return; }
    const delta = (adjustmentMode === "increase" ? 1 : -1) * Number(adjustmentQuantity);
    setError("");
    try {
      const updated = normalizePart(await api.adjustStock(adjusting.id, delta, adjustmentReason));
      setParts((current) => current.map((item) => item.id === updated.id ? updated : item));
      setAdjusting(null);
      setAdjustmentQuantity("1");
    } catch (cause) { setError(errorMessage(cause)); }
  }
  async function remove(part: Part) {
    if (!api.deletePart) return;
    setError("");
    try {
      // Use the supported async plugin API, not the legacy window.confirm bridge.
      const accepted = await confirmDialog(`删除器件“${part.name}”？剩余库存 ${part.quantity} 将移出可用库存，盒位将释放。历史流水和焊接记录保留，但该器件的历史取用将无法撤销。`, {
        title: "删除器件", kind: "warning", okLabel: "删除", cancelLabel: "取消",
      });
      if (!accepted) return;
      await api.deletePart(part.id);
    } catch (cause) {
      setError(errorMessage(cause));
      return;
    }
    setParts((current) => current.filter((item) => item.id !== part.id));
    if (api.listBoxes) {
      try { setBoxes(await api.listBoxes()); }
      catch { setError("器件已删除，但盒位刷新失败，请刷新页面后重试"); }
    }
  }
  async function lookupLcsc() {
    if (!api.lookupLcsc) { setError("LCSC 查询不可用"); return; }
    setError("");
    try {
      const info: LcscPartInfo = await api.lookupLcsc(form.lcsc_code);
      setForm((current) => ({ ...current, name: current.name.trim() ? current.name : info.name, category: current.category.trim() ? current.category : info.category, package: current.package.trim() ? current.package : info.package, manufacturer: current.manufacturer.trim() ? current.manufacturer : info.manufacturer, mpn: current.mpn.trim() ? current.mpn : info.mpn, lcsc_code: info.lcsc_code }));
    } catch (cause) { setError(errorMessage(cause)); }
  }
  const columns = useMemo<DataColumn<Part>[]>(() => [
    { id: "name", header: "名称", width: 190, cell: (part) => part.name },
    { id: "spec", header: "规格", width: 220, cell: (part) => [part.category, part.package, part.mpn].filter(Boolean).join(" · ") || "—" },
    { id: "lcsc", header: "LCSC ID", width: 110, cell: (part) => part.lcsc_code || "—" },
    { id: "box", header: "盒位", width: 150, cell: (part) => `${part.box_id === null ? "未分配" : (boxNames.get(part.box_id) || `盒子 #${part.box_id}`)}${part.slot ? ` / ${part.slot}` : ""}` },
    { id: "quantity", header: "库存", width: 70, cell: (part) => part.quantity },
    { id: "actions", header: "操作", width: 250, cell: (part) => <div className="inventory-actions">
      <button className="pn-button pn-button--ghost" type="button" onClick={() => openEdit(part)}>编辑资料</button>
      <button className="pn-button pn-button--secondary" type="button" onClick={() => { setAdjusting(part); setAdjustmentQuantity("1"); setAdjustmentMode("increase"); setAdjustmentReason("采购入库"); setMorePartId(null); }}>调整库存</button>
      {api.deletePart && <div className="inventory-more"><button className="pn-button pn-button--ghost" type="button" aria-label={`${part.name}更多操作`} aria-expanded={morePartId === part.id} onClick={() => setMorePartId((current) => current === part.id ? null : part.id)}>更多 ⋮</button>{morePartId === part.id && <div className="inventory-more__menu" role="menu"><button type="button" role="menuitem" onClick={() => { setMorePartId(null); openEdit(part); }}>编辑资料</button><button type="button" role="menuitem" onClick={() => { setMorePartId(null); setAdjusting(part); }}>调整库存</button><button type="button" role="menuitem" className="inventory-more__danger" onClick={() => { setMorePartId(null); void remove(part); }}>删除器件</button></div>}</div>}
    </div> },
  ], [api.deletePart, boxNames, morePartId]);
  const toolbarActions = useMemo(() => <><input className="pn-control" aria-label="搜索库存" placeholder="名称、MPN 或 LCSC" value={search} onChange={(event) => setSearch(event.target.value)} /><button className="pn-button pn-button--primary" type="button" onClick={openCreate}>新增器件</button></>, [search, drawerOpen]);
  const inShell = usePageActions(toolbarActions);
  return <section className="inventory-page inventory-page--workspace" aria-label="库存管理">
    {!inShell && <div className="inventory-local-toolbar">{toolbarActions}</div>}
    {loadFailed && <div className="pn-inline-error inventory-load-error" role="alert">读取库存失败 <button className="pn-button pn-button--ghost" type="button" onClick={() => void load()}>重试</button></div>}
    {error && !drawerOpen && <p className="pn-inline-error" role="alert">{error}</p>}
    <DataTable label="器件列表" rows={parts} columns={columns} rowKey={(part) => part.id} emptyText="暂无器件" />
    <PartDrawer open={drawerOpen} editing={editing} form={form} boxes={boxes} quantityText={quantityText} error={error} onChange={setField} onQuantityChange={setQuantityText} onLookupLcsc={() => void lookupLcsc()} onSave={() => void save()} onRequestClose={closeDrawer} onRequestCloseReady={(request) => { drawerCloseRef.current = request; }} />
    <Dialog open={Boolean(adjusting)} title="调整库存" onRequestClose={() => setAdjusting(null)}>
      {adjusting && <form className="pn-dialog-form" onSubmit={(event) => { event.preventDefault(); void adjust(); }}>
        <p className="inventory-adjust-summary"><strong>器件：</strong>{adjusting.name}<br /><strong>当前库存：</strong>{adjusting.quantity}</p>
        <FormField label="操作方式" htmlFor="adjustment-mode"><div className="inventory-adjust-modes"><button type="button" className={`pn-button ${adjustmentMode === "increase" ? "pn-button--primary" : "pn-button--secondary"}`} onClick={() => setAdjustmentMode("increase")}>增加库存</button><button type="button" className={`pn-button ${adjustmentMode === "decrease" ? "pn-button--primary" : "pn-button--secondary"}`} onClick={() => setAdjustmentMode("decrease")}>减少库存</button></div></FormField>
        <FormField label="数量" htmlFor="adjustment-quantity"><input id="adjustment-quantity" className="pn-control" type="number" min="1" step="1" value={adjustmentQuantity} onChange={(event) => setAdjustmentQuantity(event.target.value)} /></FormField>
        <FormField label="原因" htmlFor="adjustment-reason"><select id="adjustment-reason" className="pn-control" value={adjustmentReason} onChange={(event) => setAdjustmentReason(event.target.value)}><option>采购入库</option><option>盘点修正</option><option>损坏报废</option><option>手工调整</option></select></FormField>
        <p className="inventory-adjust-result">调整后库存：<strong>{adjusting.quantity + (adjustmentMode === "increase" ? 1 : -1) * (Number(adjustmentQuantity) || 0)}</strong></p>
        {error && <p className="pn-inline-error" role="alert">{error}</p>}
        <div className="pn-form-actions"><button className="pn-button pn-button--ghost" type="button" onClick={() => setAdjusting(null)}>取消</button><button className="pn-button pn-button--primary" type="submit">确认调整</button></div>
      </form>}
    </Dialog>
  </section>;
}
