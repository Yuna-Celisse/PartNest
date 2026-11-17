import { useEffect, useRef, useState } from "react";

const MIN_COLUMN_WIDTH = 72;
const MAX_COLUMN_WIDTH = 480;

export function useResizableColumns(defaults: Record<string, number>, limits: { minWidth?: number; maxWidth?: number } = {}) {
  const minWidth = limits.minWidth ?? MIN_COLUMN_WIDTH;
  const maxWidth = limits.maxWidth ?? MAX_COLUMN_WIDTH;
  const [widths, setWidths] = useState(() => ({ ...defaults }));
  const drag = useRef<{ column: string; startX: number; startWidth: number } | null>(null);

  useEffect(() => {
    const move = (event: MouseEvent) => {
      const current = drag.current;
      if (!current) return;
      const width = Math.max(minWidth, Math.min(maxWidth, current.startWidth + event.clientX - current.startX));
      setWidths((previous) => ({ ...previous, [current.column]: width }));
    };
    const end = () => { drag.current = null; };
    document.addEventListener("mousemove", move);
    document.addEventListener("mouseup", end);
    return () => {
      document.removeEventListener("mousemove", move);
      document.removeEventListener("mouseup", end);
    };
  }, [minWidth, maxWidth]);

  function startResize(column: string, event: React.MouseEvent, renderedWidths?: Record<string, number>) {
    if (event.button !== 0) return;
    event.preventDefault();
    if (renderedWidths) setWidths((previous) => ({ ...previous, ...renderedWidths }));
    drag.current = { column, startX: event.clientX, startWidth: renderedWidths?.[column] ?? widths[column] ?? minWidth };
  }

  function adjustWidth(column: string, delta: number, renderedWidths?: Record<string, number>) {
    setWidths((previous) => {
      const current = { ...previous, ...renderedWidths };
      return { ...current, [column]: Math.max(minWidth, Math.min(maxWidth, (current[column] ?? minWidth) + delta)) };
    });
  }

  return { widths, startResize, adjustWidth, minWidth, maxWidth };
}
