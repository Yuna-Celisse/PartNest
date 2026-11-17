import type { BomAnalysisRow } from "./useProjectImport";
import { DataTable, type DataColumn } from "../../components/ui/DataTable";
import { StatusBadge, type StatusTone } from "../../components/ui/StatusBadge";
import { useState } from "react";
import { Dialog } from "../../components/ui/Overlay";
import type { Part } from "../../app/contracts";

const statusLabel = { exact: "精确匹配", candidate: "候选匹配", none: "未匹配" } as const;
const statusTone: Record<BomAnalysisRow["status"], StatusTone> = { exact: "success", candidate: "warning", none: "danger" };

export function BomAnalysisTable({ rows, inventory = [], onConfirmMatch, onUndoMatch }: { rows: BomAnalysisRow[]; inventory?: Part[]; onConfirmMatch: (componentKey: string, partId: string, manual?: boolean) => void | Promise<boolean>; onUndoMatch?: (componentKey: string) => Promise<boolean> }) {
  const [undoing, setUndoing] = useState<string | null>(null);
  const [matching, setMatching] = useState<BomAnalysisRow | null>(null);
  const [search, setSearch] = useState("");
  const [selected, setSelected] = useState("");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");
  const filtered = inventory.filter((part) => `${part.name} ${part.package} ${part.mpn} ${part.lcsc_code}`.toLowerCase().includes(search.trim().toLowerCase()));
  async function saveManualMatch() {
    if (!matching || !selected) return;
    setSaving(true); setError("");
    try {
      const saved = await onConfirmMatch(matching.componentKey, selected, true);
      if (saved === false) setError("保存手动匹配失败，请重试或重新加载库存");
      else setMatching(null);
    } catch { setError("保存手动匹配失败，请重试"); }
    finally { setSaving(false); }
  }
  const columns: DataColumn<BomAnalysisRow>[] = [
    { id: "component", header: "器件", width: 180, cell: (row) => <span title={row.name || row.value}>{row.name || row.value || "—"}</span> },
    { id: "package", header: "封装", width: 100, cell: (row) => row.package || "—" },
    { id: "required", header: "BOM 数量", width: 90, cell: (row) => row.required },
    { id: "stock", header: "库存", width: 80, cell: (row) => row.stock },
    { id: "shortage", header: "缺料", width: 80, cell: (row) => <StatusBadge tone={row.shortage > 0 ? "danger" : "success"}>{row.shortage}</StatusBadge> },
    { id: "match", header: "匹配状态", width: 360, cell: (row) => <div className="bom-match-cell">
      <StatusBadge tone={statusTone[row.status]}>{row.manual ? "手动匹配" : row.confirmed ? "人工确认" : statusLabel[row.status]}</StatusBadge>
      {!row.confirmed && row.status !== "exact" && <button className="pn-button pn-button--secondary" type="button" onClick={() => { setMatching(row); setSearch(""); setSelected(""); setError(""); }}>手动匹配</button>}
      {row.manual && <span>{inventory.find((part) => part.id === row.partId)?.name}</span>}
      {row.confirmed && <StatusBadge tone="active">已确认</StatusBadge>}
      {row.confirmed && onUndoMatch && <button className="pn-button pn-button--ghost" disabled={undoing !== null} type="button" onClick={() => { setUndoing(row.componentKey); void onUndoMatch(row.componentKey).finally(() => setUndoing(null)); }}>{undoing === row.componentKey ? "撤销中…" : "撤销匹配"}</button>}
      {row.status === "candidate" && row.candidateIds?.map((id) => {
        const candidate = row.candidates?.find((item) => item.id === id);
        return <div className="bom-match-candidate" key={id}>
          {candidate && <><strong>{candidate.name} · {candidate.package} · 库存 {candidate.quantity}</strong>{candidate.reasons.map((reason) => <small key={reason}>{reason}</small>)}</>}
          <button className="pn-button pn-button--ghost" type="button" onClick={() => onConfirmMatch(row.componentKey, id)}>确认匹配</button>
        </div>;
      })}
    </div> },
    { id: "box", header: "盒位", width: 120, cell: (row) => row.boxSlot ?? "—" },
  ];
  return <section className="bom-analysis" aria-labelledby="bom-analysis-title">
    <h2 id="bom-analysis-title" className="bom-analysis__title">缺料分析</h2>
    <DataTable label="BOM 分析表" rows={rows} columns={columns} rowKey={(row) => row.componentKey} emptyText="暂无 BOM 器件" resizable />
    <Dialog open={Boolean(matching)} title="手动匹配库存器件" onRequestClose={() => { if (!saving) setMatching(null); }}>
      <div className="bom-manual-match">
        <p>BOM：{matching?.name || matching?.value} · {matching?.package} · 需要 {matching?.required} 个</p>
        <p>请核对型号、封装与电气参数。手动匹配仅建立关联，不会扣减库存。</p>
        <input className="pn-control" aria-label="搜索匹配库存" placeholder="名称、封装、MPN 或 LCSC" value={search} onChange={(event) => setSearch(event.target.value)} />
        <div className="bom-manual-match__list" role="radiogroup" aria-label="库存器件">
          {filtered.map((part) => <label key={part.id}><input type="radio" name="bom-manual-match" checked={selected === part.id} onChange={() => setSelected(part.id)} /><span><strong>{part.name}</strong><small>{part.package || "未填写封装"} · {part.mpn || "未填写型号"} · {part.lcsc_code || "无 LCSC"} · 库存 {part.quantity} · 盒位 {part.box_id ?? "未分配"}/{part.slot || "—"}</small></span></label>)}
          {!filtered.length && <p>没有符合搜索条件的库存器件</p>}
        </div>
        {error && <p role="alert">{error}</p>}
        <div className="pn-form-actions"><button className="pn-button pn-button--ghost" disabled={saving} onClick={() => setMatching(null)}>取消</button><button className="pn-button pn-button--primary" disabled={!selected || saving} onClick={() => void saveManualMatch()}>{saving ? "保存中…" : "保存手动匹配"}</button></div>
      </div>
    </Dialog>
  </section>;
}
