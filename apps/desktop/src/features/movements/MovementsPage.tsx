import { useEffect, useMemo, useState } from "react";
import { desktopApi, errorMessage, type DesktopApi, type Movement } from "../../app/tauri";
import { usePageActions } from "../../app/AppShell";
import { DataTable, type DataColumn } from "../../components/ui/DataTable";
import { StatusBadge, type StatusTone } from "../../components/ui/StatusBadge";

export type MovementsApi = Pick<DesktopApi, "listMovements" | "reverseTake">;

const quantity = (value: number | null) => value === null ? "—" : String(value);
const movementLabels: Record<string, { label: string; tone: StatusTone }> = {
  consume: { label: "取用", tone: "active" },
  adjust: { label: "调整", tone: "neutral" },
  reversal: { label: "撤销", tone: "success" },
};

export function MovementsPage({ api = desktopApi }: { api?: MovementsApi }) {
  const [movements, setMovements] = useState<Movement[]>([]);
  const [error, setError] = useState("");
  const [busyId, setBusyId] = useState<string | null>(null);
  const [search, setSearch] = useState("");
  const [movementType, setMovementType] = useState("all");

  async function load() {
    try {
      setError("");
      setMovements(await api.listMovements());
    } catch (cause) {
      setError(errorMessage(cause));
    }
  }

  useEffect(() => { void load(); }, [api]);

  async function reverse(movement: Movement) {
    setBusyId(movement.id);
    setError("");
    try {
      await api.reverseTake(movement.id);
      await load();
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setBusyId(null);
    }
  }

  const filteredMovements = useMemo(() => movements.filter((movement) => {
    if (movementType !== "all" && movement.movement_type !== movementType) return false;
    const needle = search.trim().toLocaleLowerCase();
    if (!needle) return true;
    return [movement.component, movement.part_name, movement.component_key, movement.reason, movement.project_name]
      .filter(Boolean)
      .some((value) => value!.toLocaleLowerCase().includes(needle));
  }), [movements, movementType, search]);

  const toolbarActions = useMemo(() => <>
    <input className="pn-control" aria-label="筛选流水" placeholder="筛选器件或原因" value={search} onChange={(event) => setSearch(event.target.value)} />
    <select className="pn-control" aria-label="流水类型" value={movementType} onChange={(event) => setMovementType(event.target.value)}>
      <option value="all">全部类型</option>
      <option value="consume">取用</option>
      <option value="adjust">调整</option>
      <option value="reversal">撤销</option>
    </select>
  </>, [movementType, search]);
  const inShell = usePageActions(toolbarActions);

  const columns = useMemo<DataColumn<Movement>[]>(() => [
    { id: "created", header: "时间", width: 160, cell: (movement) => movement.created_at },
    { id: "component", header: "器件", width: 160, cell: (movement) => movement.component ?? movement.part_name ?? movement.component_key ?? "—" },
    { id: "delta", header: "变化量", width: 100, cell: (movement) => {
      const status = movementLabels[movement.movement_type] ?? { label: movement.movement_type, tone: "neutral" as StatusTone };
      return <span className="movement-delta"><StatusBadge tone={status.tone}>{status.label}</StatusBadge><span>{movement.delta}</span></span>;
    } },
    { id: "before", header: "变化前", width: 80, cell: (movement) => quantity(movement.before_quantity) },
    { id: "after", header: "变化后", width: 80, cell: (movement) => quantity(movement.after_quantity) },
    { id: "reason", header: "原因", width: 150, cell: (movement) => movement.reason },
    { id: "project", header: "项目", width: 140, cell: (movement) => movement.project_name ?? "—" },
    { id: "actions", header: "操作", width: 100, cell: (movement) => movement.reversible && <button className="pn-button pn-button--secondary" type="button" disabled={busyId === movement.id} onClick={() => void reverse(movement)}>撤销取用</button> },
  ], [busyId]);

  return <section className="operations-page movements-page" aria-label="库存流水">
    {!inShell && <div className="operations-local-toolbar" role="toolbar" aria-label="库存流水工具栏">{toolbarActions}</div>}
    {error && <p className="pn-inline-error" role="alert">{error}</p>}
    <DataTable label="库存流水列表" rows={filteredMovements} columns={columns} rowKey={(movement) => movement.id} emptyText="暂无流水" />
  </section>;
}
