import { useEffect, useState, type ReactNode } from "react";
import { ChevronRightIcon, LayoutGridIcon, PlusIcon, ShieldCheckIcon, UsersIcon } from "lucide-react";

import { api, ApiError, auth, setUnauthorizedHandler } from "@/api";
import { Logo } from "@/components/Logo";
import { ThemeToggle } from "@/components/ThemeToggle";
import { UserMenu } from "@/components/UserMenu";
import { Separator } from "@/components/ui/separator";
import { Skeleton } from "@/components/ui/skeleton";
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
  SidebarMenuBadge,
  SidebarMenuButton,
  SidebarMenuItem,
  SidebarMenuSkeleton,
  SidebarProvider,
  SidebarTrigger,
  useSidebar,
} from "@/components/ui/sidebar";
import { Toaster } from "@/components/ui/sonner";
import { TooltipProvider } from "@/components/ui/tooltip";
import { ApprovalsProvider, useApprovals } from "@/lib/approvals";
import { PermissionsProvider, usePermissions, WriteGuard } from "@/lib/permissions";
import { ProjectsProvider, useProjects } from "@/lib/projects";
import { href, useRoute, type ProjectTab, type Route } from "@/lib/router";
import { cn } from "@/lib/utils";
import { ApprovalsPage } from "@/pages/ApprovalsPage";
import { ConsentPage } from "@/pages/ConsentPage";
import { UsersPage } from "@/pages/UsersPage";
import { Home } from "@/pages/Home";
import { ImportPage } from "@/pages/ImportPage";
import { ProjectPage } from "@/pages/ProjectPage";
import { ReimportPage } from "@/pages/ReimportPage";
import { SqlToolEditor } from "@/pages/SqlToolEditor";
import { ToolEditor } from "@/pages/ToolEditor";
import { Login } from "@/pages/Login";
import { Setup } from "@/pages/Setup";
import type { Access, ServerInfo, User } from "@/types";

type Gate =
  | { state: "checking" }
  | { state: "setup" }
  | { state: "login" }
  | { state: "ready"; info: ServerInfo; user: User | null; access: Record<string, Access> | null }
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
      setGate({ state: "ready", info: await api.info(), user: status.user, access: status.access });
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
        <PermissionsProvider user={gate.user} access={gate.access}>
          <ProjectsProvider>
            <ApprovalsProvider>
              <Shell info={gate.info} user={gate.user} onSignedOut={() => setGate({ state: "login" })} />
            </ApprovalsProvider>
          </ProjectsProvider>
        </PermissionsProvider>
      )}
      <Toaster position="bottom-right" />
    </TooltipProvider>
  );
}

const SECTION: Record<ProjectTab, string> = {
  tools: "Tools",
  specs: "Specs",
  auth: "Auth",
  databases: "Databases",
  calls: "Calls",
  audit: "Audit log",
  connect: "Connect",
  settings: "Settings",
};

function Shell({ info, user, onSignedOut }: { info: ServerInfo; user: User | null; onSignedOut: () => void }) {
  const route = useRoute();
  const activeProject = "projectId" in route ? route.projectId : null;

  return (
    <SidebarProvider>
      <AppSidebar activeProject={activeProject} version={info.version} user={user} onSignedOut={onSignedOut} />
      <SidebarInset className="min-w-0">
        <header className="sticky top-0 z-20 flex h-14 items-center gap-2 border-b bg-background/85 px-3 backdrop-blur supports-[backdrop-filter]:bg-background/70 sm:px-4">
          <SidebarTrigger className="-ml-1" />
          <Separator orientation="vertical" className="mr-1 self-center data-[orientation=vertical]:h-5" />
          <Breadcrumbs route={route} />
          <div className="ml-auto">
            <ThemeToggle />
          </div>
        </header>
        <main className="mx-auto w-full max-w-6xl px-4 py-6 sm:px-6 sm:py-8 lg:px-8">
          <Page route={route} info={info} user={user} />
        </main>
      </SidebarInset>
    </SidebarProvider>
  );
}

/** Where you are: Projects › project › section. */
function Breadcrumbs({ route }: { route: Route }) {
  const { projects, error } = useProjects();
  const crumbs: { label: ReactNode; href?: string }[] =
    route.name === "approvals"
      ? [{ label: "Approvals" }]
      : route.name === "consent"
        ? [{ label: "Connect an app" }]
        : route.name === "users"
          ? [{ label: "Users" }]
          : [{ label: "Projects", href: href.home() }];
  if (route.name !== "home" && route.name !== "approvals" && route.name !== "consent" && route.name !== "users") {
    const project = projects?.find((p) => p.id === route.projectId);
    // Placeholder while the project list loads, so the name doesn't flash in.
    const label =
      projects || error ? (
        (project?.name ?? "Project")
      ) : (
        <Skeleton className="inline-block h-4 w-24 align-middle" aria-label="Loading project" />
      );
    crumbs.push({ label, href: href.project(route.projectId) });
    switch (route.name) {
      case "project":
        crumbs.push({ label: SECTION[route.tab] });
        break;
      case "tool":
        crumbs.push({ label: "Tools", href: href.project(route.projectId) });
        crumbs.push({ label: route.toolId ? "Edit tool" : route.kind === "sql" ? "New SQL tool" : "New HTTP tool" });
        break;
      case "import":
        crumbs.push({ label: "Import" });
        break;
      case "reimport":
        crumbs.push({ label: "Specs", href: href.project(route.projectId, "specs") });
        crumbs.push({ label: "Re-import" });
        break;
    }
  }
  const last = crumbs.length - 1;
  return (
    <nav aria-label="Breadcrumb" className="min-w-0">
      <ol className="flex min-w-0 items-center gap-1 text-sm">
        {crumbs.map((c, i) => (
          // On phones only the last two crumbs fit.
          <li key={i} className={cn("flex min-w-0 items-center gap-1", i < last - 1 && "hidden sm:flex")}>
            {i > 0 && <ChevronRightIcon className="size-3.5 shrink-0 text-muted-foreground/70" aria-hidden="true" />}
            {i === last || !c.href ? (
              <span aria-current={i === last ? "page" : undefined} className="truncate font-medium">
                {c.label}
              </span>
            ) : (
              <a
                href={c.href}
                className="truncate rounded-sm text-muted-foreground outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
              >
                {c.label}
              </a>
            )}
          </li>
        ))}
      </ol>
    </nav>
  );
}

function Page({ route, info, user }: { route: Route; info: ServerInfo; user: User | null }) {
  switch (route.name) {
    case "home":
      return <Home />;
    case "approvals":
      return <ApprovalsPage />;
    case "consent":
      return <ConsentPage requestId={route.requestId} user={user} />;
    case "users":
      return <UsersPage />;
    case "project":
      return <ProjectPage key={route.projectId} projectId={route.projectId} tab={route.tab} info={info} />;
    case "import":
      return (
        <WriteGuard projectId={route.projectId}>
          <ImportPage key={route.projectId} projectId={route.projectId} />
        </WriteGuard>
      );
    case "reimport":
      return (
        <WriteGuard projectId={route.projectId}>
          <ReimportPage key={route.specId} projectId={route.projectId} specId={route.specId} />
        </WriteGuard>
      );
    case "tool":
      // Read-only users see the tool; inputs, Save and the test console are disabled.
      return (
        <WriteGuard projectId={route.projectId}>
          {!route.toolId && route.kind === "sql" ? (
            <SqlToolEditor key={`${route.projectId}/new-sql`} projectId={route.projectId} tool={null} />
          ) : (
            <ToolEditor key={`${route.projectId}/${route.toolId ?? "new"}`} projectId={route.projectId} toolId={route.toolId} />
          )}
        </WriteGuard>
      );
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
  const { pending } = useApprovals();
  const { isAdmin } = usePermissions();
  const route = useRoute();
  const { setOpenMobile } = useSidebar();
  // On phones the sidebar is a sheet; close it once a link is followed.
  const close = () => setOpenMobile(false);
  return (
    <Sidebar>
      <SidebarHeader className="h-14 justify-center border-b border-sidebar-border px-3">
        <a
          href={href.home()}
          onClick={close}
          className="flex items-center gap-2.5 rounded-md px-1 py-1 outline-none focus-visible:ring-2 focus-visible:ring-ring"
        >
          <Logo className="size-7" />
          <span className="font-semibold tracking-tight">mcp-forge</span>
        </a>
      </SidebarHeader>
      <SidebarContent>
        <SidebarGroup>
          <SidebarMenu>
            <SidebarMenuItem>
              <SidebarMenuButton asChild isActive={route.name === "home"}>
                <a href={href.home()} onClick={close}>
                  <LayoutGridIcon />
                  <span>All projects</span>
                </a>
              </SidebarMenuButton>
            </SidebarMenuItem>
            <SidebarMenuItem>
              <SidebarMenuButton asChild isActive={route.name === "approvals"}>
                <a href={href.approvals()} onClick={close}>
                  <ShieldCheckIcon />
                  <span>Approvals</span>
                </a>
              </SidebarMenuButton>
              {pending.length > 0 && (
                <SidebarMenuBadge className="bg-warning/15 font-semibold text-warning" aria-label={`${pending.length} waiting`}>
                  {pending.length}
                </SidebarMenuBadge>
              )}
            </SidebarMenuItem>
            {isAdmin && (
              <SidebarMenuItem>
                <SidebarMenuButton asChild isActive={route.name === "users"}>
                  <a href={href.users()} onClick={close}>
                    <UsersIcon />
                    <span>Users</span>
                  </a>
                </SidebarMenuButton>
              </SidebarMenuItem>
            )}
          </SidebarMenu>
        </SidebarGroup>
        <SidebarGroup>
          <SidebarGroupLabel>Projects</SidebarGroupLabel>
          {isAdmin && (
            <SidebarGroupAction asChild title="New project">
              <a href={`${href.home()}?new`} onClick={close} aria-label="New project">
                <PlusIcon />
              </a>
            </SidebarGroupAction>
          )}
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
                  <SidebarMenuButton asChild isActive={p.id === activeProject} size="lg" className="h-auto py-1.5">
                    <a href={href.project(p.id)} onClick={close}>
                      <span
                        aria-hidden="true"
                        className="grid size-7 shrink-0 place-items-center rounded-md border bg-background text-xs font-semibold text-primary uppercase"
                      >
                        {p.name.slice(0, 1)}
                      </span>
                      <span className="grid min-w-0 leading-tight">
                        <span className="truncate">{p.name}</span>
                        <span className="truncate font-mono text-[11px] text-muted-foreground">/mcp/{p.slug}</span>
                      </span>
                    </a>
                  </SidebarMenuButton>
                </SidebarMenuItem>
              ))}
              {projects?.length === 0 && (
                <p className="px-2 py-1 text-xs leading-relaxed text-muted-foreground">No projects yet. Use + to create one.</p>
              )}
            </SidebarMenu>
          </SidebarGroupContent>
        </SidebarGroup>
      </SidebarContent>
      <SidebarFooter className="border-t border-sidebar-border">
        {user ? (
          <UserMenu user={user} version={version} onSignedOut={onSignedOut} />
        ) : (
          <p className="px-2 text-xs text-muted-foreground">Version {version}</p>
        )}
      </SidebarFooter>
    </Sidebar>
  );
}
