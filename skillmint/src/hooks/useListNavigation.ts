import { useCallback, useEffect, useRef, useState } from "react";

export interface UseListNavigationOptions<T> {
  items: T[];
  onSelect?: (item: T, index: number) => void;
  /** Initial active index; -1 means no selection. */
  initialIndex?: number;
  /** Enable cycling (arrow down from last item goes to first). Default false. */
  cycle?: boolean;
  /** Ref to the list container for focus management. */
  listRef?: React.RefObject<HTMLElement | null>;
}

export interface UseListNavigationResult {
  activeIndex: number;
  setActiveIndex: (index: number) => void;
  handleKeyDown: (e: React.KeyboardEvent) => void;
  getItemProps: (index: number) => {
    id: string;
    role: string;
    "aria-selected": boolean;
    onClick: () => void;
    onMouseEnter: () => void;
  };
  listProps: {
    role: string;
    "aria-activedescendant": string | undefined;
    tabIndex: number;
    onKeyDown: (e: React.KeyboardEvent) => void;
  };
}

let idCounter = 0;

export function useListNavigation<T>(
  options: UseListNavigationOptions<T>
): UseListNavigationResult {
  const { items, onSelect, initialIndex = -1, cycle = false, listRef } = options;
  const [activeIndex, setActiveIndex] = useState(initialIndex);
  const listIdRef = useRef(`list-nav-${++idCounter}`);

  const move = useCallback(
    (delta: number) => {
      if (items.length === 0) return;
      setActiveIndex((prev) => {
        if (prev < 0) {
          // From no selection, always start at the first item.
          return 0;
        }
        const next = prev + delta;
        if (cycle) {
          return (next + items.length) % items.length;
        }
        return Math.max(0, Math.min(items.length - 1, next));
      });
    },
    [items.length, cycle]
  );

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (items.length === 0) return;
      switch (e.key) {
        case "ArrowDown":
          e.preventDefault();
          move(1);
          break;
        case "ArrowUp":
          e.preventDefault();
          move(-1);
          break;
        case "Home":
          e.preventDefault();
          setActiveIndex(0);
          break;
        case "End":
          e.preventDefault();
          setActiveIndex(items.length - 1);
          break;
        case "Enter":
        case " ":
          if (activeIndex >= 0 && activeIndex < items.length) {
            e.preventDefault();
            onSelect?.(items[activeIndex], activeIndex);
          }
          break;
      }
    },
    [items, activeIndex, move, onSelect]
  );

  const getItemProps = useCallback(
    (index: number) => {
      return {
        id: `${listIdRef.current}-item-${index}`,
        role: "option",
        "aria-selected": index === activeIndex,
        onClick: () => {
          setActiveIndex(index);
          onSelect?.(items[index], index);
        },
        onMouseEnter: () => setActiveIndex(index),
      };
    },
    [items, activeIndex, onSelect]
  );

  useEffect(() => {
    if (activeIndex >= items.length) {
      setActiveIndex(items.length > 0 ? items.length - 1 : -1);
    }
  }, [items.length, activeIndex]);

  const listId = listIdRef.current;
  const activeDescendant =
    activeIndex >= 0 ? `${listId}-item-${activeIndex}` : undefined;

  return {
    activeIndex,
    setActiveIndex,
    handleKeyDown,
    getItemProps,
    listProps: {
      role: "listbox",
      "aria-activedescendant": activeDescendant,
      tabIndex: 0,
      onKeyDown: (e) => {
        handleKeyDown(e);
        listRef?.current?.focus();
      },
    },
  };
}
