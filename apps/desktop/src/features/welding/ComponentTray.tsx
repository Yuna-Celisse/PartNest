import { useEffect, useMemo, useRef, useState, type MouseEvent } from "react";
import type { BomGroup, BomSide } from "../../app/tauri";

export type ComponentTrayProps = {
  groups: BomGroup[];
  side: BomSide | "all";
  activeComponentKey: string | null;
  widths: Record<string, number>;
  onResizeStart: (column: string, event: MouseEvent, renderedWidths?: Record<string, number>) => void;
  onResizeKey: (column: string, delta: number, renderedWidths?: Record<string, number>) => void;
  onSelectDesignators: (designators: string[]) => void;
  collapsed: boolean;
  onToggle: () => void;
};

const collator = new Intl.Collator("zh-CN", { numeric: true, sensitivity: "base" });

const columns = [
  ["component", "器件"],
  ["package", "封装"],
  ["quantity", "数量"],
  ["side", "板面"],
] as const;

/** 点一行就选到它自己所在的面：当前页签有该器件时用它，否则用它自己的面。 */
function rowDesignators(group: BomGroup, side: BomSide | "all") {
  const sided = group.placements.filter((placement) => placement.side);
  // 无板面列的表格 BOM 由操作者的页签决定；"全部"只是查看，不产生选择。
  if (sided.length === 0) return side === "all" ? [] : group.designators;
  const onCurrent = sided.filter((placement) => placement.side === side).map((placement) => placement.designator);
  if (onCurrent.length) return onCurrent;
  const target = sided.some((placement) => placement.side === "top") ? "top" : sided[0].side as BomSide;
  return sided.filter((placement) => placement.side === target).map((placement) => placement.designator);
}

function sideLabel(group: BomGroup) {
  const sides = new Set(group.placements.map((placement) => placement.side).filter(Boolean));
  if (sides.size === 0) return "未标注";
  if (sides.size > 1) return "双面";
  return sides.has("top") ? "顶层" : "底层";
}

export function ComponentTray({ groups, side, activeComponentKey, widths, onResizeStart, onResizeKey, onSelectDesignators, collapsed, onToggle }: ComponentTrayProps) {
  const tableRef = useRef<HTMLDivElement>(null);
  const [fixedWidths, setFixedWidths] = useState(false);
  const [sort, setSort] = useState<{ column: typeof columns[number][0]; direction: "ascending" | "descending" } | null>(null);
  const quantity = (group: BomGroup) => side === "all" ? group.quantity : rowDesignators(group, side).length;
  const sortedGroups = useMemo(() => {
    if (!sort) return groups;
    const sign = sort.direction === "ascending" ? 1 : -1;
    return [...groups].sort((a, b) => sign * (sort.column === "quantity" ? quantity(a) - quantity(b) : collator.compare(sort.column === "component" ? a.name || a.value : sort.column === "package" ? a.package : sideLabel(a), sort.column === "component" ? b.name || b.value : sort.column === "package" ? b.package : sideLabel(b))));
  }, [groups, side, sort]);
  const rowStyle = { width: fixedWidths ? `max(100%, ${columns.reduce((sum, [column]) => sum + widths[column], 0)}px)` : "100%", minWidth: columns.length * 72 };
  const cellStyle = (column: string) => ({ width: widths[column], flexGrow: fixedWidths ? 0 : widths[column], flexShrink: fixedWidths ? 0 : 1, minWidth: 72 });
  const renderedWidths = () => Object.fromEntries(columns.map(([column], index) => [column, tableRef.current?.querySelectorAll<HTMLElement>("[role='columnheader']")[index]?.getBoundingClientRect().width || widths[column]]));
  useEffect(() => {
    const table = tableRef.current;
    const row = table?.querySelector<HTMLElement>('[data-active="true"]');
    if (!table || !row || collapsed) return;
    const viewport = table.getBoundingClientRect(), selected = row.getBoundingClientRect();
    const headerHeight = table.firstElementChild?.getBoundingClientRect().height ?? 0;
    if (selected.top < viewport.top + headerHeight) table.scrollTop += selected.top - viewport.top - headerHeight;
    else if (selected.bottom > viewport.bottom) table.scrollTop += selected.bottom - viewport.bottom;
  }, [activeComponentKey, collapsed, sort, side]);
  const resizeButton = (column: string, label: string) => <button
    type="button"
    role="separator"
    aria-label={`调整${label}列宽`}
    aria-orientation="vertical"
    aria-valuemin={72}
    aria-valuemax={2400}
    aria-valuenow={widths[column]}
    tabIndex={0}
    onMouseDown={(event) => { if (event.button !== 0) return; event.stopPropagation(); const measured = renderedWidths(); setFixedWidths(true); onResizeStart(column, event, measured); }}
    onClick={(event) => event.stopPropagation()}
    onKeyDown={(event) => {
      if (event.key === "ArrowLeft" || event.key === "ArrowRight") {
        event.preventDefault();
        const measured = renderedWidths(); setFixedWidths(true);
        onResizeKey(column, event.key === "ArrowLeft" ? -8 : 8, measured);
      }
    }}
  />;

  return <section className="component-tray" aria-label="器件列表" data-collapsed={collapsed ? "true" : "false"}>
    <header className="component-tray__header">
      <h3>器件列表</h3>
      <div className="component-tray__actions"><button type="button" className="pn-button pn-button--ghost" onClick={() => setFixedWidths(false)}>适应宽度</button>
      <button type="button" className="pn-button pn-button--ghost" aria-expanded={!collapsed} aria-label={collapsed ? "展开器件列表" : "收起器件列表"} onClick={onToggle}>{collapsed ? "展开" : "收起"}</button></div>
    </header>
    {!collapsed && <div ref={tableRef} className="component-tray__table" role="table" aria-label="BOM 器件" data-scroll-container="true">
      <div className="component-tray__row" role="row" style={rowStyle}>
        {columns.map(([column, label]) => <div className="component-tray__cell component-tray__header-cell" role="columnheader" key={column} data-testid={column === "component" ? "component-column" : undefined} style={cellStyle(column)} aria-sort={sort?.column === column ? sort.direction : "none"}>
          <button type="button" className="component-tray__sort" onClick={() => setSort((current) => ({ column, direction: current?.column === column && current.direction === "ascending" ? "descending" : "ascending" }))}>{label}<span aria-hidden="true">{sort?.column === column ? sort.direction === "ascending" ? "↑" : "↓" : "↕"}</span></button>{resizeButton(column, label)}
        </div>)}
      </div>
      {sortedGroups.map((group) => {
        const designators = rowDesignators(group, side);
        const selectable = designators.length > 0;
        return <div className="component-tray__row" role="row" style={rowStyle} key={group.component_key} data-testid={`tray-row-${group.component_key}`} data-active={group.component_key === activeComponentKey ? "true" : "false"} data-selectable={selectable ? "true" : "false"} tabIndex={selectable ? 0 : undefined} aria-label={selectable ? `${group.name || group.value}，选择 ${designators.join(", ")}` : undefined} onClick={() => selectable && onSelectDesignators(designators)} onKeyDown={(event) => { if (selectable && (event.key === "Enter" || event.key === " ")) { event.preventDefault(); onSelectDesignators(designators); } }}>
          <div className="component-tray__cell" role="cell" data-testid={group.component_key === activeComponentKey ? "component-cell" : undefined} style={cellStyle("component")}>{group.name || group.value}</div>
          <div className="component-tray__cell" role="cell" style={cellStyle("package")}>{group.package || "—"}</div>
          <div className="component-tray__cell" role="cell" style={cellStyle("quantity")}>{quantity(group)}</div>
          <div className="component-tray__cell" role="cell" style={cellStyle("side")}>{sideLabel(group)}</div>
        </div>;
      })}
      {!groups.length && <p className="component-tray__empty">暂无器件</p>}
    </div>}
  </section>;
}
