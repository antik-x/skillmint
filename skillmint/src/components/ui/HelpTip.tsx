import { useEffect, useId, useRef, useState } from "react";
import { HelpCircle } from "lucide-react";
import { cn } from "./utils";

/**
 * SPEC-F6 T2: 轻量就地术语解释组件。
 * - `?` 图标按钮，hover 或键盘聚焦显示气泡。
 * - 纯 CSS/受控 state，不引第三方库；键盘可聚焦（tabindex=0），Esc 关闭。
 * - 气泡定位在图标上方，自动适配窄屏。
 *
 * 用法：
 *   <HelpTip text="把中心仓库里的这份 Skill 写入各 Agent 的规则目录…" />
 *   或在标题旁：  <h2>数据采集 <HelpTip text="只读读取…" /></h2>
 */
export interface HelpTipProps {
  /** 气泡里要展示的解释文案。 */
  text: string;
  /** 可选的 aria-label，默认「什么是…」。 */
  ariaLabel?: string;
  className?: string;
  /** 图标尺寸，默认 14。 */
  size?: number;
}

export function HelpTip({ text, ariaLabel, className, size = 14 }: HelpTipProps) {
  const [open, setOpen] = useState(false);
  const tipId = useId();
  const ref = useRef<HTMLButtonElement>(null);

  // SPEC-F6 T2: Esc 关闭（键盘可访问性）。
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        setOpen(false);
        ref.current?.blur();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open]);

  return (
    <span className={cn("relative inline-flex items-center align-middle", className)}>
      <button
        ref={ref}
        type="button"
        aria-label={ariaLabel ?? "查看解释"}
        aria-describedby={open ? tipId : undefined}
        aria-expanded={open}
        tabIndex={0}
        onMouseEnter={() => setOpen(true)}
        onMouseLeave={() => setOpen(false)}
        onFocus={() => setOpen(true)}
        onBlur={() => setOpen(false)}
        onClick={(e) => {
          e.preventDefault();
          e.stopPropagation();
          setOpen((v) => !v);
        }}
        className="inline-flex h-[1.1em] w-[1.1em] items-center justify-center rounded-full text-tertiary outline-none transition-colors hover:text-accent focus-visible:ring-2 focus-visible:ring-[var(--accent)]"
        data-testid="help-tip-trigger"
      >
        <HelpCircle style={{ height: size, width: size }} aria-hidden="true" />
      </button>
      {open && (
        <span
          id={tipId}
          role="tooltip"
          data-testid="help-tip-bubble"
          className="surface-glass pointer-events-none absolute bottom-full left-1/2 z-50 mb-2 w-64 -translate-x-1/2 rounded-lg border border-[var(--border-subtle)] bg-secondary px-3 py-2 text-left text-xs font-normal leading-relaxed text-primary shadow-lg"
        >
          {text}
        </span>
      )}
    </span>
  );
}
