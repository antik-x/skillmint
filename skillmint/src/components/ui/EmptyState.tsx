import { cn } from "./utils";
import type { LucideIcon } from "lucide-react";
import { EmptyIllustration, type EmptyIllustrationKind } from "./EmptyIllustration";

export interface EmptyStateProps extends React.HTMLAttributes<HTMLDivElement> {
  icon?: LucideIcon;
  /** 优先于 icon 显示的品牌插画 */
  illustration?: EmptyIllustrationKind;
  title: string;
  description?: string;
  action?: React.ReactNode;
}

export function EmptyState({
  className,
  icon: Icon,
  illustration,
  title,
  description,
  action,
  ...props
}: EmptyStateProps) {
  return (
    <div
      className={cn(
        "flex flex-col items-center justify-center rounded-xl border border-dashed border-[var(--border-prominent)] bg-secondary/50 px-8 py-12 text-center",
        className
      )}
      {...props}
    >
      {illustration ? (
        <EmptyIllustration kind={illustration} className="mb-3" />
      ) : (
        Icon && (
          <div className="mb-4 flex h-12 w-12 items-center justify-center rounded-full bg-tertiary">
            <Icon className="h-6 w-6 text-secondary" />
          </div>
        )
      )}
      <h3 className="text-base font-semibold text-primary">{title}</h3>
      {description && (
        <p className="mt-1 max-w-xs text-sm text-secondary">{description}</p>
      )}
      {action && <div className="mt-5">{action}</div>}
    </div>
  );
}
