import { Loader2 } from "lucide-react";
import { cn } from "./utils";

export interface ButtonProps extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: "primary" | "secondary" | "ghost" | "danger" | "warning";
  size?: "sm" | "md" | "lg";
  loading?: boolean;
}

export function Button({
  className,
  variant = "secondary",
  size = "md",
  loading = false,
  disabled,
  children,
  ...props
}: ButtonProps) {
  const base =
    "inline-flex items-center justify-center gap-2 rounded-lg font-medium transition-all duration-200 focus:outline-none focus-visible:ring-2 focus-visible:ring-[var(--accent)] focus-visible:ring-offset-2 focus-visible:ring-offset-[var(--bg-primary)] disabled:cursor-not-allowed disabled:opacity-50";

  const variants = {
    primary:
      "btn-primary-gradient text-white shadow-sm hover:shadow-md hover:brightness-105 active:scale-[0.98]",
    secondary:
      "bg-tertiary text-primary border border-[var(--border-subtle)] hover:bg-[var(--border-subtle)] active:scale-[0.98]",
    ghost:
      "bg-transparent text-secondary hover:bg-[var(--border-subtle)] hover:text-primary active:scale-[0.98]",
    danger:
      "bg-danger/10 text-danger border border-danger/20 hover:bg-danger/20 active:scale-[0.98]",
    warning:
      "bg-warning/10 text-warning border border-warning/20 hover:bg-warning/20 active:scale-[0.98]",
  };

  const sizes = {
    sm: "px-3 py-1.5 text-xs",
    md: "px-4 py-2 text-sm",
    lg: "px-5 py-2.5 text-base",
  };

  return (
    <button
      className={cn(base, variants[variant], sizes[size], className)}
      disabled={disabled || loading}
      {...props}
    >
      {loading && <Loader2 className="h-4 w-4 animate-spin" />}
      {children}
    </button>
  );
}
