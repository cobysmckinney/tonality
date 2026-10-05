import { ReactNode, RefObject, useEffect, useImperativeHandle, useLayoutEffect, useMemo, useRef, useState } from "react";

export interface GridSection<T> {
  key: string;
  header?: ReactNode;
  items: T[];
}

export type Direction = "left" | "right" | "up" | "down";

export interface GridHandle<K> {
  /** The item next to `key` in a direction, or null at an edge. */
  neighbor: (key: K, direction: Direction) => K | null;
  first: () => K | null;
  reveal: (key: K) => void;
}

interface Props<T, K> {
  sections: GridSection<T>[];
  /** Preferred cell width; cells stretch a little to fill each row exactly. */
  cellSize: number;
  itemKey: (item: T) => K;
  renderCell: (item: T, size: number) => ReactNode;
  handle?: RefObject<GridHandle<K> | null>;
  className?: string;
  onBackgroundClick?: () => void;
}

type Row<T> =
  | { kind: "header"; key: string; top: number; height: number; content: ReactNode }
  | { kind: "cells"; key: string; top: number; height: number; items: T[] };

const GAP = 4;
const HEADER_HEIGHT = 64;
const SECTION_SPACING = 12;
const OVERSCAN = 600;

/**
 * A scrolling grid of sections that only mounts the rows near the viewport,
 * so it stays quick with tens of thousands of photos.
 */
export function VirtualGrid<T, K extends string | number>(props: Props<T, K>) {
  const { sections, cellSize, itemKey, renderCell, handle, className, onBackgroundClick } = props;
  const scroller = useRef<HTMLDivElement>(null);
  const canvas = useRef<HTMLDivElement>(null);
  const [viewport, setViewport] = useState({ width: 0, height: 0 });
  const [scrollTop, setScrollTop] = useState(0);

  useLayoutEffect(() => {
    // The canvas gives the usable width, inside the scroller's padding and scrollbar.
    const measure = () => setViewport({ width: canvas.current!.clientWidth, height: scroller.current!.clientHeight });
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(scroller.current!);
    observer.observe(canvas.current!);
    return () => observer.disconnect();
  }, []);

  const layout = useMemo(() => {
    const width = Math.max(0, viewport.width);
    const columns = Math.max(1, Math.floor((width + GAP) / (cellSize + GAP)));
    const size = Math.max(1, (width - GAP * (columns - 1)) / columns);
    const rows: Row<T>[] = [];
    const position = new Map<K, { row: number; column: number }>();
    let top = 0;
    for (const section of sections) {
      if (section.header !== undefined) {
        rows.push({ kind: "header", key: `h:${section.key}`, top, height: HEADER_HEIGHT, content: section.header });
        top += HEADER_HEIGHT;
      }
      for (let start = 0; start < section.items.length; start += columns) {
        const items = section.items.slice(start, start + columns);
        items.forEach((item, column) => position.set(itemKey(item), { row: rows.length, column }));
        rows.push({ kind: "cells", key: `c:${section.key}:${start}`, top, height: size, items });
        top += size + GAP;
      }
      top += SECTION_SPACING;
    }
    return { rows, position, size, height: top };
  }, [sections, cellSize, viewport.width, itemKey]);

  // Changing the zoom or filter can leave the scroll position past the end.
  useEffect(() => {
    const element = scroller.current!;
    if (element.scrollTop !== scrollTop) setScrollTop(element.scrollTop);
  }, [layout, scrollTop]);

  useImperativeHandle(
    handle,
    () => ({
      first() {
        const row = layout.rows.find((r) => r.kind === "cells");
        return row?.kind === "cells" ? itemKey(row.items[0]) : null;
      },
      neighbor(key, direction) {
        const at = layout.position.get(key);
        if (!at) return null;
        const step = direction === "left" || direction === "up" ? -1 : 1;
        const sideways = direction === "left" || direction === "right";
        const here = layout.rows[at.row] as Extract<Row<T>, { kind: "cells" }>;
        if (sideways && here.items[at.column + step]) return itemKey(here.items[at.column + step]);
        for (let r = at.row + step; r >= 0 && r < layout.rows.length; r += step) {
          const row = layout.rows[r];
          if (row.kind !== "cells") continue;
          const column = sideways ? (step < 0 ? row.items.length - 1 : 0) : Math.min(at.column, row.items.length - 1);
          return itemKey(row.items[column]);
        }
        return null;
      },
      reveal(key) {
        const at = layout.position.get(key);
        const element = scroller.current;
        if (!at || !element) return;
        const row = layout.rows[at.row];
        const margin = 12;
        // Bring the day heading along when moving up into the first row of a section.
        const above = layout.rows[at.row - 1];
        const top = above?.kind === "header" ? above.top : row.top - margin;
        if (top < element.scrollTop) element.scrollTop = top;
        else if (row.top + row.height + margin > element.scrollTop + element.clientHeight) {
          element.scrollTop = row.top + row.height + margin - element.clientHeight;
        }
      },
    }),
    [layout, itemKey],
  );

  const visible = useMemo(() => {
    const from = scrollTop - OVERSCAN;
    const to = scrollTop + viewport.height + OVERSCAN;
    // Binary search for the first row that reaches into the window.
    let low = 0;
    let high = layout.rows.length;
    while (low < high) {
      const middle = (low + high) >> 1;
      if (layout.rows[middle].top + layout.rows[middle].height < from) low = middle + 1;
      else high = middle;
    }
    let end = low;
    while (end < layout.rows.length && layout.rows[end].top <= to) end++;
    return layout.rows.slice(low, end);
  }, [layout, scrollTop, viewport.height]);

  return (
    <div
      ref={scroller}
      className={`virtual-grid ${className ?? ""}`}
      onScroll={(event) => setScrollTop(event.currentTarget.scrollTop)}
      onClick={(event) => {
        if (!(event.target as HTMLElement).closest("[data-cell]")) onBackgroundClick?.();
      }}
    >
      <div ref={canvas} className="virtual-grid-canvas" style={{ height: layout.height }}>
        {visible.map((row) =>
          row.kind === "header" ? (
            <div key={row.key} className="virtual-grid-row" style={{ top: row.top, height: row.height }}>
              {row.content}
            </div>
          ) : (
            <div key={row.key} className="virtual-grid-row cells" style={{ top: row.top, height: row.height, gap: GAP }}>
              {row.items.map((item) => (
                <div key={itemKey(item)} data-cell style={{ width: layout.size, height: layout.size }}>
                  {renderCell(item, layout.size)}
                </div>
              ))}
            </div>
          ),
        )}
      </div>
    </div>
  );
}
