import { useEffect, useState } from "react";
import { FolderIcon, PlusIcon } from "lucide-react";

import { api, ApiError, auth, setUnauthorizedHandler } from "@/api";
import { Logo } from "@/components/Logo";
import { UserMenu } from "@/components/UserMenu";
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
import { SqlToolEditor } from "@/pages/SqlToolEditor";
import { ToolEditor } from "@/pages/ToolEditor";
import { Login } from "@/pages/Login";
import { Setup } from "@/pages/Setup";
import type { ServerInfo, User } from "@/types";

type Gate =
  | { state: "checking" }
  | { state: "setup" }
  | { state: "login" }
  | { state: "ready"; info: ServerInfo; user: User | null }
  | { state: "down"; message: string };

export default function App() {
  const [gate, setGate] = useState<Gate>({ state: "checking" });

  async function check() {
    try {
      const status = await auth.status();
      if (status.login_required && !status.user) {
        setGate({ state: status.setup_required ? "setup" : "login" });
        return;
      }
      setGate({ state: "ready", info: await api.info(), user: status.user });
    } catch (e) {
      if (e instanceof ApiError && e.status === 401) setGate({ state: "login" });
      else setGate({ state: "down", message: e instanceof Error ? e.message : String(e) });
    }
  }

  useEffect(() => {
    setUnauthorizedHandler(() => setGate({ state: "login" }));
    void check();
  }, []);

  return (
    <TooltipProvider>
      {gate.state === "setup" && <Setup onSignedIn={() => void check()} />}
      {gate.state === "login" && <Login onSignedIn={() => void check()} />}
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
          <Shell info={gate.info} user={gate.user} onSignedOut={() => setGate({ state: "login" })} />
        </ProjectsProvider>
      )}
      <Toaster position="bottom-right" />
    </TooltipProvider>
  );
}

function Shell({ info, user, onSignedOut }: { info: ServerInfo; user: User | null; onSignedOut: () => void }) {
  const route = useRoute();
  const activeProject = route.name === "home" ? null : route.projectId;

  return (
    <SidebarProvider>
      <AppSidebar activeProject={activeProject} version={info.version} user={user} onSignedOut={onSignedOut} />
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
      if (!route.toolId && route.kind === "sql")
        return <SqlToolEditor key={`${route.projectId}/new-sql`} projectId={route.projectId} tool={null} />;
      return <ToolEditor key={`${route.projectId}/${route.toolId ?? "new"}`} projectId={route.projectId} toolId={route.toolId} />;
  }
}

function AppSidebar({
  activeProject,
  version,
  user,
  onSignedOut,
}: {
  activeProject: string | null;
  version: string;
  user: User | null;
  onSignedOut: () => void;
}) {
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
        {user ? (
          <UserMenu user={user} version={version} onSignedOut={onSignedOut} />
        ) : (
          <p className="px-2 text-xs text-muted-foreground">Version {version}</p>
        )}
      </SidebarFooter>
    </Sidebar>
  );
}
