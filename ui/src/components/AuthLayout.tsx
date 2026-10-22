import type { ReactNode } from "react";
import { DatabaseIcon, ShieldCheckIcon, WorkflowIcon } from "lucide-react";

import { Logo } from "@/components/Logo";

const POINTS = [
  { icon: WorkflowIcon, text: "Turn OpenAPI specs into curated MCP tools." },
  { icon: DatabaseIcon, text: "Expose SQL with read-only guards, masks and an audit log." },
  { icon: ShieldCheckIcon, text: "Secrets stay as references; nothing leaves this machine." },
];

/** Two-pane frame for the sign-in and first-run screens; one column on phones. */
export function AuthLayout({ title, description, children }: { title: string; description: ReactNode; children: ReactNode }) {
  return (
    <main className="grid min-h-svh bg-background lg:grid-cols-[minmax(0,1fr)_minmax(0,1.1fr)]">
      <aside className="relative hidden overflow-hidden bg-sidebar p-10 text-sidebar-foreground lg:flex lg:flex-col">
        <div aria-hidden="true" className="auth-grid pointer-events-none absolute inset-0 opacity-60" />
        <div className="relative flex items-center gap-2.5">
          <Logo className="size-8" />
          <span className="text-lg font-semibold tracking-tight">mcp-forge</span>
        </div>
        <div className="relative mt-auto max-w-md space-y-8">
          <p className="text-3xl font-semibold leading-tight tracking-tight text-balance">
            Your APIs and databases, as tools your agents can use safely.
          </p>
          <ul className="space-y-4">
            {POINTS.map(({ icon: Icon, text }) => (
              <li key={text} className="flex gap-3 text-sm text-muted-foreground">
                <span className="grid size-8 shrink-0 place-items-center rounded-lg border bg-background/60 text-primary">
                  <Icon className="size-4" />
                </span>
                <span className="pt-1.5">{text}</span>
              </li>
            ))}
          </ul>
        </div>
      </aside>
      <section className="flex flex-col items-center justify-center px-4 py-10 sm:px-8">
        <div className="w-full max-w-sm">
          <div className="mb-8 flex items-center gap-2.5 lg:hidden">
            <Logo className="size-8" />
            <span className="text-lg font-semibold tracking-tight">mcp-forge</span>
          </div>
          <h1 className="text-2xl font-semibold tracking-tight">{title}</h1>
          <p className="mt-1.5 text-sm text-muted-foreground">{description}</p>
          <div className="mt-8">{children}</div>
        </div>
      </section>
    </main>
  );
}
