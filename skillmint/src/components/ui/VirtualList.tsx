import { useEffect, useRef } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { cn } from "./utils";

export interface VirtualListProps<T> {
  items: T[];
  estimateSize?: number;
  renderItem: (item: T, index: number) => React.ReactNode;
  getItemKey?: (item: T, index: number) => string;
  className?: string;
  empty?: React.ReactNode;
  activeIndex?: number;
}

export function VirtualList<T>({
  items,
  estimateSize = 56,
  renderItem,
  getItemKey,
  className,
  empty,
  activeIndex,
}: VirtualListProps<T>) {
  const parentRef = useRef<HTMLDivElement>(null);
  const virtualizer = useVirtualizer({
    count: items.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => estimateSize,
    getItemKey: getItemKey ? (index) => getItemKey(items[index], index) : undefined,
    overscan: 6,
  });

  useEffect(() => {
    if (activeIndex != null && activeIndex >= 0 && activeIndex < items.length) {
      virtualizer.scrollToIndex(activeIndex, { align: "center" });
    }
  }, [activeIndex, items.length, virtualizer]);

  if (items.length === 0 && empty) {
    return <>{empty}</>;
  }

  const virtualItems = virtualizer.getVirtualItems();

  return (
    <div
      ref={parentRef}
      className={cn("overflow-auto", className)}
      style={{ contain: "strict" }}
    >
      <div
        style={{
          height: `${virtualizer.getTotalSize()}px`,
          width: "100%",
          position: "relative",
        }}
      >
        {virtualItems.map((virtualItem) => {
          const item = items[virtualItem.index];
          return (
            <div
              key={virtualItem.key}
              data-index={virtualItem.index}
              ref={virtualizer.measureElement}
              style={{
                position: "absolute",
                top: 0,
                left: 0,
                width: "100%",
                transform: `translateY(${virtualItem.start}px)`,
              }}
            >
              {renderItem(item, virtualItem.index)}
            </div>
          );
        })}
      </div>
    </div>
  );
}
