import { useMemo, useState, type ReactNode } from "react";
import { useResizableColumns } from "./useResizableColumns";

export type DataColumn<Row> = {
  id: string;
  header: string;
  width?: number | string;
  cell: (row: Row) => ReactNode;
  compare?: (left: Row, right: Row) => number;
};

export function DataTable<Row>({ label, rows, columns, rowKey, emptyText, resizable = false }: {
  label: string;
  rows: Row[];
  columns: DataColumn<Row>[];
  rowKey: (row: Row) => string;
  emptyText: string;
  resizable?: boolean;
}): JSX.Element {
  const resizing = useResizableColumns(Object.fromEntries(columns.map((column) => [column.id, typeof column.width === "number" ? column.width : 120])), { maxWidth: 1200 });
  const width = (column: DataColumn<Row>) => resizing.widths[column.id] ?? 120;
  const [sort, setSort] = useState<{ columnId: string; direction: "ascending" | "descending" } | null>(null);
  const sortedRows = useMemo(() => {
    const compare = columns.find((column) => column.id === sort?.columnId)?.compare;
    if (!compare || !sort) return rows;
    const direction = sort.direction === "ascending" ? 1 : -1;
    return [...rows].sort((left, right) => direction * compare(left, right));
  }, [rows, columns, sort]);
  return <div className="pn-table-wrap">
    <table className={`pn-data-table${resizable ? " pn-data-table--resizable" : ""}`} aria-label={label} style={resizable ? { width: columns.reduce((sum, column) => sum + width(column), 0), minWidth: "100%" } : undefined}>
      <colgroup>{columns.map((column) => <col key={column.id} style={{ width: resizable ? width(column) : typeof column.width === "number" ? `${column.width}px` : column.width }} />)}</colgroup>
      <thead><tr>{columns.map((column) => <th key={column.id} scope="col" aria-sort={column.compare ? (sort?.columnId === column.id ? sort.direction : "none") : undefined}>
        {column.compare ? <button className="pn-data-table__sort" type="button" onClick={() => setSort((current) => ({ columnId: column.id, direction: current?.columnId === column.id && current.direction === "ascending" ? "descending" : "ascending" }))}>
          {column.header}<span aria-hidden="true">{sort?.columnId === column.id ? (sort.direction === "ascending" ? "↑" : "↓") : "↕"}</span>
        </button> : column.header}
        {resizable && <span className="pn-data-table__resize" role="separator" aria-label={`调整${column.header}列宽`} aria-orientation="vertical" aria-valuenow={width(column)} aria-valuemin={resizing.minWidth} aria-valuemax={resizing.maxWidth} tabIndex={0} onMouseDown={(event) => resizing.startResize(column.id, event)} onClick={(event) => event.stopPropagation()} onKeyDown={(event) => { if (event.key === "ArrowLeft" || event.key === "ArrowRight") { event.preventDefault(); resizing.adjustWidth(column.id, event.key === "ArrowRight" ? 16 : -16); } }} />}
      </th>)}</tr></thead>
      <tbody>
        {sortedRows.length === 0 ? <tr><td className="pn-data-table__empty" colSpan={columns.length}>{emptyText}</td></tr> : sortedRows.map((row) => <tr key={rowKey(row)}>{columns.map((column) => <td key={column.id}>{column.cell(row)}</td>)}</tr>)}
      </tbody>
    </table>
  </div>;
}
