import { cn } from "./utils";

export interface BadgeProps extends React.HTMLAttributes<HTMLSpanElement> {
  variant?: "default" | "success" | "warning" | "danger" | "info" | "accent";
  size?: "sm" | "md";
}

export function Badge({
  className,
  variant = "default",
  size = "sm",
  children,
  ...props
}: BadgeProps) {
  const variants = {
    default:
      "bg-tertiary text-secondary border-[var(--border-subtle)]",
    accent:
      "bg-accent/10 text-accent border-accent/20",
    success:
      "bg-success/10 text-success border-success/20",
    warning:
      "bg-warning/10 text-warning border-warning/20",
    danger:
      "bg-danger/10 text-danger border-danger/20",
    info:
      "bg-info/10 text-info border-info/20",
  };

  const sizes = {
    sm: "px-2 py-0.5 text-xs",
    md: "px-2.5 py-1 text-sm",
  };

  return (
    <span
      className={cn(
        "inline-flex items-center rounded-md border font-medium",
        variants[variant],
        sizes[size],
        className
      )}
      {...props}
    >
      {children}
    </span>
  );
}
