import type { BomAnalysisRow } from "./useProjectImport";
import { DataTable, type DataColumn } from "../../components/ui/DataTable";
import { StatusBadge, type StatusTone } from "../../components/ui/StatusBadge";

const statusLabel = { exact: "精确匹配", candidate: "候选匹配", none: "未匹配" } as const;
const statusTone: Record<BomAnalysisRow["status"], StatusTone> = { exact: "success", candidate: "warning", none: "danger" };

export function BomAnalysisTable({ rows, onConfirmMatch }: { rows: BomAnalysisRow[]; onConfirmMatch: (componentKey: string, partId: string) => void }) {
  const columns: DataColumn<BomAnalysisRow>[] = [
    { id: "component", header: "器件", width: 180, cell: (row) => <span title={row.name || row.value}>{row.name || row.value || "—"}</span> },
    { id: "package", header: "封装", width: 100, cell: (row) => row.package || "—" },
    { id: "required", header: "BOM 数量", width: 90, cell: (row) => row.required },
    { id: "stock", header: "库存", width: 80, cell: (row) => row.stock },
    { id: "shortage", header: "缺料", width: 80, cell: (row) => <StatusBadge tone={row.shortage > 0 ? "danger" : "success"}>{row.shortage}</StatusBadge> },
    { id: "match", header: "匹配状态", width: 220, cell: (row) => <div className="bom-match-cell">
      <StatusBadge tone={statusTone[row.status]}>{statusLabel[row.status]}</StatusBadge>
      {row.confirmed && <StatusBadge tone="active">已确认</StatusBadge>}
      {row.status === "candidate" && row.candidateIds?.map((id) => <button className="pn-button pn-button--ghost" key={id} type="button" onClick={() => onConfirmMatch(row.componentKey, id)}>确认匹配</button>)}
    </div> },
    { id: "box", header: "盒位", width: 120, cell: (row) => row.boxSlot ?? "—" },
  ];
  return <section className="bom-analysis" aria-labelledby="bom-analysis-title">
    <h2 id="bom-analysis-title" className="bom-analysis__title">缺料分析</h2>
    <DataTable label="BOM 分析表" rows={rows} columns={columns} rowKey={(row) => row.componentKey} emptyText="暂无 BOM 器件" />
  </section>;
}
