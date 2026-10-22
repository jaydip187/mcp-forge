import { useCallback, useEffect, useState } from "react";

import { api, ApiError } from "@/api";
import { McpAddress } from "@/components/McpAddress";
import { PageHeader } from "@/components/PageHeader";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Skeleton } from "@/components/ui/skeleton";
import { href, type ProjectTab } from "@/lib/router";
import { cn } from "@/lib/utils";
import { AuditTab } from "@/pages/AuditTab";
import { AuthTab } from "@/pages/AuthTab";
import { ConnectTab, mcpUrl } from "@/pages/ConnectTab";
import { DatabasesTab } from "@/pages/DatabasesTab";
import { SettingsTab } from "@/pages/SettingsTab";
import { SpecsTab } from "@/pages/SpecsTab";
import { ToolsTab } from "@/pages/ToolsTab";
import type { Project, ServerInfo } from "@/types";

const SECTIONS: { tab: ProjectTab; label: string }[] = [
  { tab: "tools", label: "Tools" },
  { tab: "specs", label: "Specs" },
  { tab: "auth", label: "Auth" },
  { tab: "databases", label: "Databases" },
  { tab: "audit", label: "Audit log" },
  { tab: "connect", label: "Connect" },
  { tab: "settings", label: "Settings" },
];

export function ProjectPage({ projectId, tab, info }: { projectId: string; tab: ProjectTab; info: ServerInfo }) {
  const [project, setProject] = useState<Project | null>(null);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      setProject(await api.getProject(projectId));
    } catch (e) {
      setError(e instanceof ApiError && e.status === 404 ? "This project doesn't exist. It may have been deleted." : String(e));
    }
  }, [projectId]);
  useEffect(() => {
    void load();
  }, [load]);

  if (error)
    return (
      <Alert variant="destructive">
        <AlertDescription>
          {error}{" "}
          <a className="underline" href={href.home()}>
            Back to projects
          </a>
        </AlertDescription>
      </Alert>
    );
  if (!project)
    return (
      <div className="space-y-6">
        <Skeleton className="h-8 w-56" />
        <Skeleton className="h-12 w-full max-w-2xl rounded-xl" />
        <Skeleton className="h-40 w-full rounded-xl" />
      </div>
    );

  return (
    <div className="space-y-6">
      <PageHeader title={project.name} description="Agents connect to every visible tool in this project at one URL.">
        <div className="max-w-2xl">
          <McpAddress url={mcpUrl(project.slug)} />
        </div>
      </PageHeader>

      <nav aria-label="Project sections" className="-mx-4 overflow-x-auto overflow-y-hidden border-b px-4 sm:mx-0 sm:px-0">
        <ul className="flex gap-1">
          {SECTIONS.map((s) => {
            const active = s.tab === tab;
            return (
              <li key={s.tab}>
                <a
                  href={href.project(project.id, s.tab)}
                  aria-current={active ? "page" : undefined}
                  className={cn(
                    "relative inline-flex h-10 items-center rounded-t-md px-3 text-sm font-medium whitespace-nowrap outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset",
                    active
                      ? "text-foreground after:absolute after:inset-x-2 after:bottom-0 after:h-0.5 after:rounded-full after:bg-primary"
                      : "text-muted-foreground hover:text-foreground",
                  )}
                >
                  {s.label}
                </a>
              </li>
            );
          })}
        </ul>
      </nav>

      <div>
        {tab === "tools" && <ToolsTab projectId={project.id} settings={project.settings} />}
        {tab === "specs" && <SpecsTab projectId={project.id} />}
        {tab === "auth" && <AuthTab projectId={project.id} />}
        {tab === "databases" && <DatabasesTab projectId={project.id} />}
        {tab === "audit" && <AuditTab projectId={project.id} />}
        {tab === "connect" && <ConnectTab project={project} info={info} />}
        {tab === "settings" && <SettingsTab project={project} onSaved={setProject} />}
      </div>
    </div>
  );
}
