import { cn } from "./utils";

export interface CardProps extends React.HTMLAttributes<HTMLDivElement> {
  variant?: "default" | "ghost" | "elevated";
  padding?: "none" | "sm" | "md" | "lg";
}

export function Card({
  className,
  variant = "default",
  padding = "md",
  children,
  ...props
}: CardProps) {
  const variants = {
    default:
      "bg-secondary border border-[var(--border-subtle)] rounded-xl shadow-sm transition-all duration-200 hover:border-[var(--border-prominent)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--accent)]",
    ghost: "bg-transparent border border-transparent rounded-xl",
    elevated:
      "bg-elevated border border-[var(--border-subtle)] rounded-xl shadow-md transition-all duration-200 hover:shadow-lg hover:border-[var(--border-prominent)]",
  };

  const paddings = {
    none: "",
    sm: "p-4",
    md: "p-6",
    lg: "p-8",
  };

  return (
    <div
      className={cn(variants[variant], paddings[padding], className)}
      {...props}
    >
      {children}
    </div>
  );
}

export function CardHeader({
  className,
  children,
  ...props
}: React.HTMLAttributes<HTMLDivElement>) {
  return (
    <div className={cn("mb-4 flex items-center justify-between", className)} {...props}>
      {children}
    </div>
  );
}

export function CardTitle({
  className,
  children,
  ...props
}: React.HTMLAttributes<HTMLHeadingElement>) {
  return (
    <h3 className={cn("text-base font-semibold text-primary", className)} {...props}>
      {children}
    </h3>
  );
}

export function CardDescription({
  className,
  children,
  ...props
}: React.HTMLAttributes<HTMLParagraphElement>) {
  return (
    <p className={cn("text-sm text-secondary", className)} {...props}>
      {children}
    </p>
  );
}
