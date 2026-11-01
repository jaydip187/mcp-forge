import { useEffect, useState } from "react";

/** Hash routes, so the server never needs an SPA fallback. */
export type Route =
  | { name: "home" }
  | { name: "approvals" }
  | { name: "consent"; requestId: string }
  | { name: "users" }
  | { name: "project"; projectId: string; tab: ProjectTab }
  | { name: "tool"; projectId: string; toolId: string | null; kind: ToolEditorKind }
  | { name: "import"; projectId: string }
  | { name: "reimport"; projectId: string; specId: string };

export type ToolEditorKind = "http" | "sql" | "graphql" | "nosql" | "chain";

export type ProjectTab = "tools" | "specs" | "auth" | "databases" | "context" | "calls" | "audit" | "connect" | "settings";
const TABS: ProjectTab[] = ["tools", "specs", "auth", "databases", "context", "calls", "audit", "connect", "settings"];

export function parseRoute(hash: string): Route {
  const parts = hash.replace(/^#\/?/, "").split("/").filter(Boolean).map(decodeURIComponent);
  const [a, pid, b, tid, c] = parts;
  if (a === "approvals") return { name: "approvals" };
  if (a === "users") return { name: "users" };
  if (a === "oauth" && pid === "consent" && b) return { name: "consent", requestId: b };
  if (a === "p" && pid) {
    if (b === "tools" && tid) {
      if (tid === "new") return { name: "tool", projectId: pid, toolId: null, kind: "http" };
      if (tid === "new-sql") return { name: "tool", projectId: pid, toolId: null, kind: "sql" };
      if (tid === "new-graphql") return { name: "tool", projectId: pid, toolId: null, kind: "graphql" };
      if (tid === "new-nosql") return { name: "tool", projectId: pid, toolId: null, kind: "nosql" };
      if (tid === "new-chain") return { name: "tool", projectId: pid, toolId: null, kind: "chain" };
      return { name: "tool", projectId: pid, toolId: tid, kind: "http" };
    }
    if (b === "import") return { name: "import", projectId: pid };
    if (b === "specs" && tid && c === "reimport") return { name: "reimport", projectId: pid, specId: tid };
    const tab = (TABS as string[]).includes(b ?? "") ? (b as ProjectTab) : "tools";
    return { name: "project", projectId: pid, tab };
  }
  return { name: "home" };
}

export const href = {
  home: () => "#/",
  approvals: () => "#/approvals",
  users: () => "#/users",
  project: (pid: string, tab: ProjectTab = "tools") =>
    tab === "tools" ? `#/p/${encodeURIComponent(pid)}` : `#/p/${encodeURIComponent(pid)}/${tab}`,
  tool: (pid: string, tid: string | null, kind: ToolEditorKind = "http") =>
    `#/p/${encodeURIComponent(pid)}/tools/${tid ?? (kind === "http" ? "new" : `new-${kind}`)}`,
  import: (pid: string) => `#/p/${encodeURIComponent(pid)}/import`,
  reimport: (pid: string, sid: string) => `#/p/${encodeURIComponent(pid)}/specs/${encodeURIComponent(sid)}/reimport`,
};

export function navigate(to: string) {
  window.location.hash = to.replace(/^#/, "");
}

export function useRoute(): Route {
  const [route, setRoute] = useState(() => parseRoute(window.location.hash));
  useEffect(() => {
    const onChange = () => setRoute(parseRoute(window.location.hash));
    window.addEventListener("hashchange", onChange);
    return () => window.removeEventListener("hashchange", onChange);
  }, []);
  return route;
}
