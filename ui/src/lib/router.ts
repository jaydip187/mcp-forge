import { useEffect, useState } from "react";

/** Hash routes, so the server never needs an SPA fallback. */
export type Route =
  | { name: "home" }
  | { name: "project"; projectId: string; tab: ProjectTab }
  | { name: "tool"; projectId: string; toolId: string | null }
  | { name: "import"; projectId: string };

export type ProjectTab = "tools" | "auth" | "connect" | "settings";
const TABS: ProjectTab[] = ["tools", "auth", "connect", "settings"];

export function parseRoute(hash: string): Route {
  const parts = hash.replace(/^#\/?/, "").split("/").filter(Boolean).map(decodeURIComponent);
  const [a, pid, b, tid] = parts;
  if (a === "p" && pid) {
    if (b === "tools" && tid) return { name: "tool", projectId: pid, toolId: tid === "new" ? null : tid };
    if (b === "import") return { name: "import", projectId: pid };
    const tab = (TABS as string[]).includes(b ?? "") ? (b as ProjectTab) : "tools";
    return { name: "project", projectId: pid, tab };
  }
  return { name: "home" };
}

export const href = {
  home: () => "#/",
  project: (pid: string, tab: ProjectTab = "tools") =>
    tab === "tools" ? `#/p/${encodeURIComponent(pid)}` : `#/p/${encodeURIComponent(pid)}/${tab}`,
  tool: (pid: string, tid: string | null) => `#/p/${encodeURIComponent(pid)}/tools/${tid ?? "new"}`,
  import: (pid: string) => `#/p/${encodeURIComponent(pid)}/import`,
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
