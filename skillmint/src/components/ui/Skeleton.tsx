import { cn } from "./utils";

export interface SkeletonProps extends React.HTMLAttributes<HTMLDivElement> {
  circle?: boolean;
}

export function Skeleton({ className, circle, ...props }: SkeletonProps) {
  return (
    <div
      className={cn(
        "animate-pulse bg-tertiary/60",
        circle ? "rounded-full" : "rounded-md",
        className
      )}
      {...props}
    />
  );
}

export function SkeletonCard({ className, ...props }: React.HTMLAttributes<HTMLDivElement>) {
  return (
    <div
      className={cn(
        "rounded-xl border border-[var(--border-subtle)] bg-secondary p-6",
        className
      )}
      {...props}
    >
      <Skeleton className="mb-4 h-4 w-1/3" />
      <Skeleton className="h-8 w-1/2" />
    </div>
  );
}

export function SkeletonList({ count = 5 }: { count?: number }) {
  return (
    <div className="space-y-3">
      {Array.from({ length: count }).map((_, i) => (
        <div
          key={i}
          className="flex items-center gap-4 rounded-xl border border-[var(--border-subtle)] bg-secondary p-4"
        >
          <Skeleton circle className="h-10 w-10" />
          <div className="flex-1 space-y-2">
            <Skeleton className="h-4 w-1/3" />
            <Skeleton className="h-3 w-1/2" />
          </div>
        </div>
      ))}
    </div>
  );
}
