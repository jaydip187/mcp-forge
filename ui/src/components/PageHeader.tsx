import type { ReactNode } from "react";
import { ArrowLeftIcon } from "lucide-react";

/** The top of every page: where you are, what this page is for, and its main actions. */
export function PageHeader({
  title,
  description,
  actions,
  back,
  children,
}: {
  title: ReactNode;
  description?: ReactNode;
  actions?: ReactNode;
  back?: { href: string; label: string };
  children?: ReactNode;
}) {
  return (
    <header className="space-y-4">
      {back && (
        <a
          href={back.href}
          className="-ml-1 inline-flex items-center gap-1.5 rounded-md px-1 text-sm text-muted-foreground outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
        >
          <ArrowLeftIcon className="size-4" /> {back.label}
        </a>
      )}
      <div className="flex flex-col gap-4 sm:flex-row sm:items-start sm:justify-between">
        <div className="min-w-0 space-y-1.5">
          <h1 className="text-2xl leading-tight font-semibold tracking-tight text-balance break-words">{title}</h1>
          {description && <p className="max-w-[65ch] text-sm leading-relaxed text-muted-foreground">{description}</p>}
        </div>
        {actions && <div className="flex shrink-0 flex-wrap gap-2">{actions}</div>}
      </div>
      {children}
    </header>
  );
}
