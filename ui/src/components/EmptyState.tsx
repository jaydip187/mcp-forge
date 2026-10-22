import type { ComponentType, ReactNode } from "react";

import { cn } from "@/lib/utils";

/** What to show when a list has nothing in it yet: what goes here, and how to add the first one. */
export function EmptyState({
  icon: Icon,
  title,
  description,
  children,
  className,
}: {
  icon?: ComponentType<{ className?: string }>;
  title: string;
  description?: ReactNode;
  children?: ReactNode;
  className?: string;
}) {
  return (
    <div className={cn("flex flex-col items-center rounded-xl border border-dashed bg-card/40 px-6 py-12 text-center", className)}>
      {Icon && (
        <span className="mb-4 grid size-11 place-items-center rounded-lg border bg-card text-primary shadow-xs">
          <Icon className="size-5" />
        </span>
      )}
      <p className="font-medium text-balance">{title}</p>
      {description && <p className="mt-1.5 max-w-md text-sm leading-relaxed text-pretty text-muted-foreground">{description}</p>}
      {children && <div className="mt-5 flex flex-wrap justify-center gap-2">{children}</div>}
    </div>
  );
}
