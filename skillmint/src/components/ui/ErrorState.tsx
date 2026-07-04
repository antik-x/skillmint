import { AlertCircle, ChevronDown, RefreshCw } from "lucide-react";
import { useState } from "react";
import { cn } from "./utils";

export interface ErrorStateProps {
  title?: string;
  message: string;
  detail?: string;
  actionLabel?: string;
  onRetry?: () => void;
  className?: string;
}

/**
 * SPEC-F2 T2: unified error presentation.
 * Never renders the raw technical string by default; keeps it behind a
 * collapsible "技术详情" section.
 */
export function ErrorState({
  title = "加载失败",
  message,
  detail,
  actionLabel = "重试",
  onRetry,
  className,
}: ErrorStateProps) {
  const [showDetail, setShowDetail] = useState(false);

  return (
    <div
      className={cn(
        "rounded-xl border border-danger/20 bg-danger/5 p-6 text-center",
        className
      )}
      role="alert"
      aria-live="polite"
    >
      <AlertCircle className="mx-auto h-8 w-8 text-danger" />
      <h3 className="mt-3 text-sm font-semibold text-primary">{title}</h3>
      <p className="mt-1 text-sm text-secondary">{message}</p>

      {detail && (
        <div className="mt-3">
          <button
            type="button"
            onClick={() => setShowDetail((v) => !v)}
            className="inline-flex items-center gap-1 text-xs text-tertiary hover:text-primary"
            aria-expanded={showDetail}
          >
            技术详情
            <ChevronDown className={cn("h-3 w-3 transition-transform", showDetail && "rotate-180")} />
          </button>
          {showDetail && (
            <pre className="mt-2 max-h-32 overflow-auto rounded-md border border-danger/10 bg-primary p-2 text-left text-xs text-tertiary">
              {detail}
            </pre>
          )}
        </div>
      )}

      {onRetry && (
        <button
          type="button"
          onClick={onRetry}
          className="mt-4 inline-flex items-center gap-1.5 rounded-md bg-danger/10 px-3 py-1.5 text-sm font-medium text-danger hover:bg-danger/20"
        >
          <RefreshCw className="h-4 w-4" />
          {actionLabel}
        </button>
      )}
    </div>
  );
}
