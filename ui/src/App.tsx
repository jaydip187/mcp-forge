import { useEffect, useState } from "react";
import { FolderIcon, PlusIcon } from "lucide-react";

import { api, ApiError, setUnauthorizedHandler, token } from "@/api";
import {
  Sidebar,
  SidebarContent,
  SidebarFooter,
  SidebarGroup,
  SidebarGroupAction,
  SidebarGroupContent,
  SidebarGroupLabel,
  SidebarHeader,
  SidebarInset,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
  SidebarMenuSkeleton,
  SidebarProvider,
  SidebarTrigger,
} from "@/components/ui/sidebar";
import { Toaster } from "@/components/ui/sonner";
import { TooltipProvider } from "@/components/ui/tooltip";
import { ProjectsProvider, useProjects } from "@/lib/projects";
import { href, useRoute, type Route } from "@/lib/router";
import { Home } from "@/pages/Home";
import { ImportPage } from "@/pages/ImportPage";
import { ProjectPage } from "@/pages/ProjectPage";
import { ReimportPage } from "@/pages/ReimportPage";
import { ToolEditor } from "@/pages/ToolEditor";
import { Unlock } from "@/pages/Unlock";
import type { ServerInfo } from "@/types";

type Gate = { state: "checking" } | { state: "locked" } | { state: "ready"; info: ServerInfo } | { state: "down"; message: string };

export default function App() {
  const [gate, setGate] = useState<Gate>({ state: "checking" });

  async function check() {
    try {
      setGate({ state: "ready", info: await api.info() });
    } catch (e) {
      if (e instanceof ApiError && e.status === 401) setGate({ state: "locked" });
      else setGate({ state: "down", message: e instanceof Error ? e.message : String(e) });
    }
  }

  useEffect(() => {
    setUnauthorizedHandler(() => {
      token.set(null);
      setGate({ state: "locked" });
    });
    void check();
  }, []);

  return (
    <TooltipProvider>
      {gate.state === "locked" && <Unlock onUnlocked={() => void check()} />}
      {gate.state === "down" && (
        <main className="grid min-h-svh place-items-center p-6 text-center">
          <div className="max-w-md space-y-2">
            <h1 className="text-lg font-semibold">The mcp-forge server isn't answering</h1>
            <p className="text-sm text-muted-foreground">{gate.message}</p>
          </div>
        </main>
      )}
      {gate.state === "ready" && (
        <ProjectsProvider>
          <Shell info={gate.info} />
        </ProjectsProvider>
      )}
      <Toaster position="bottom-right" />
    </TooltipProvider>
  );
}

function Shell({ info }: { info: ServerInfo }) {
  const route = useRoute();
  const activeProject = route.name === "home" ? null : route.projectId;

  return (
    <SidebarProvider>
      <AppSidebar activeProject={activeProject} version={info.version} />
      <SidebarInset>
        <header className="flex h-12 items-center gap-2 border-b px-3 md:hidden">
          <SidebarTrigger />
          <span className="font-semibold">mcp-forge</span>
        </header>
        <div className="mx-auto w-full max-w-6xl p-4 sm:p-6 lg:p-8">
          <Page route={route} info={info} />
        </div>
      </SidebarInset>
    </SidebarProvider>
  );
}

function Page({ route, info }: { route: Route; info: ServerInfo }) {
  switch (route.name) {
    case "home":
      return <Home />;
    case "project":
      return <ProjectPage key={route.projectId} projectId={route.projectId} tab={route.tab} info={info} />;
    case "import":
      return <ImportPage key={route.projectId} projectId={route.projectId} />;
    case "reimport":
      return <ReimportPage key={route.specId} projectId={route.projectId} specId={route.specId} />;
    case "tool":
      return <ToolEditor key={`${route.projectId}/${route.toolId ?? "new"}`} projectId={route.projectId} toolId={route.toolId} />;
  }
}

function AppSidebar({ activeProject, version }: { activeProject: string | null; version: string }) {
  const { projects } = useProjects();
  return (
    <Sidebar>
      <SidebarHeader>
        <a href={href.home()} className="flex items-center gap-2 rounded-md px-2 py-1.5 outline-none focus-visible:ring-2 focus-visible:ring-ring">
          <Logo />
          <span className="font-semibold tracking-tight">mcp-forge</span>
        </a>
      </SidebarHeader>
      <SidebarContent>
        <SidebarGroup>
          <SidebarGroupLabel>Projects</SidebarGroupLabel>
          <SidebarGroupAction asChild title="New project">
            <a href={`${href.home()}?new`} aria-label="New project">
              <PlusIcon />
            </a>
          </SidebarGroupAction>
          <SidebarGroupContent>
            <SidebarMenu>
              {projects === null &&
                Array.from({ length: 3 }, (_, i) => (
                  <SidebarMenuItem key={i}>
                    <SidebarMenuSkeleton />
                  </SidebarMenuItem>
                ))}
              {projects?.map((p) => (
                <SidebarMenuItem key={p.id}>
                  <SidebarMenuButton asChild isActive={p.id === activeProject}>
                    <a href={href.project(p.id)}>
                      <FolderIcon />
                      <span>{p.name}</span>
                    </a>
                  </SidebarMenuButton>
                </SidebarMenuItem>
              ))}
              {projects?.length === 0 && <p className="px-2 py-1 text-xs text-muted-foreground">No projects yet.</p>}
            </SidebarMenu>
          </SidebarGroupContent>
        </SidebarGroup>
      </SidebarContent>
      <SidebarFooter>
        <p className="px-2 text-xs text-muted-foreground">Version {version}</p>
      </SidebarFooter>
    </Sidebar>
  );
}

function Logo() {
  return (
    <svg viewBox="0 0 32 32" className="size-6 shrink-0" aria-hidden="true">
      <rect width="32" height="32" rx="7" className="fill-primary" />
      <path d="M9 10h14M9 16h9M9 22h14" className="stroke-primary-foreground" strokeWidth="3" strokeLinecap="round" />
    </svg>
  );
}
