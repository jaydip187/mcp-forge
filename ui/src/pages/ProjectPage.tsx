import { useCallback, useEffect, useState } from "react";

import { api, ApiError } from "@/api";
import { CopyBlock } from "@/components/CopyBlock";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Skeleton } from "@/components/ui/skeleton";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { href, navigate, type ProjectTab } from "@/lib/router";
import { AuthTab } from "@/pages/AuthTab";
import { ConnectTab, mcpUrl } from "@/pages/ConnectTab";
import { SettingsTab } from "@/pages/SettingsTab";
import { SpecsTab } from "@/pages/SpecsTab";
import { ToolsTab } from "@/pages/ToolsTab";
import type { Project, ServerInfo } from "@/types";

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
          {error} <a className="underline" href={href.home()}>Back to projects</a>
        </AlertDescription>
      </Alert>
    );
  if (!project) return <Skeleton className="h-24 w-full" />;

  return (
    <div className="space-y-6">
      <div className="flex flex-col gap-3 lg:flex-row lg:items-end lg:justify-between">
        <div className="min-w-0 space-y-1">
          <h1 className="truncate text-2xl font-semibold tracking-tight">{project.name}</h1>
          <p className="text-sm text-muted-foreground">Agents connect to this project at one URL.</p>
        </div>
        <div className="w-full lg:max-w-md">
          <CopyBlock code={mcpUrl(project.slug)} label="MCP URL" inline />
        </div>
      </div>

      <Tabs value={tab} onValueChange={(v) => navigate(href.project(project.id, v as ProjectTab))}>
        <TabsList>
          <TabsTrigger value="tools">Tools</TabsTrigger>
          <TabsTrigger value="specs">Specs</TabsTrigger>
          <TabsTrigger value="auth">Auth</TabsTrigger>
          <TabsTrigger value="connect">Connect</TabsTrigger>
          <TabsTrigger value="settings">Settings</TabsTrigger>
        </TabsList>
        <TabsContent value="tools" className="pt-4">
          <ToolsTab projectId={project.id} settings={project.settings} />
        </TabsContent>
        <TabsContent value="specs" className="pt-4">
          <SpecsTab projectId={project.id} />
        </TabsContent>
        <TabsContent value="auth" className="pt-4">
          <AuthTab projectId={project.id} />
        </TabsContent>
        <TabsContent value="connect" className="pt-4">
          <ConnectTab project={project} info={info} />
        </TabsContent>
        <TabsContent value="settings" className="pt-4">
          <SettingsTab project={project} onSaved={setProject} />
        </TabsContent>
      </Tabs>
    </div>
  );
}
