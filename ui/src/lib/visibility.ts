import { pathMatcher } from "./glob";
import type { ProjectSettings, Tool } from "../types";

export type HiddenReason = "disabled" | "read_only" | "tag" | "path";

const READ_METHODS = new Set(["GET", "HEAD", "OPTIONS"]);

/** Mirrors `ProjectSettings::hidden_reason` in crates/core. */
export function hiddenReason(tool: Tool, s: ProjectSettings): HiddenReason | null {
  if (!tool.enabled) return "disabled";
  // SQL query tools only read; path filters only concern HTTP tools.
  const http = tool.kind.type === "http" ? tool.kind : null;
  if (s.read_only && http && !READ_METHODS.has(http.method)) return "read_only";
  const lower = (xs: string[]) => xs.map((x) => x.toLowerCase());
  const tags = lower(tool.tags);
  const hasTag = (list: string[]) => lower(list).some((t) => tags.includes(t));
  if ((s.include_tags.length > 0 && !hasTag(s.include_tags)) || hasTag(s.exclude_tags)) return "tag";
  if (http) {
    const matches = (list: string[]) => list.some((g) => pathMatcher(g)(http.path_template));
    if ((s.include_paths.length > 0 && !matches(s.include_paths)) || matches(s.exclude_paths)) return "path";
  }
  return null;
}

export const HIDDEN_LABEL: Record<HiddenReason, string> = {
  disabled: "Turned off",
  read_only: "Hidden: project is read-only",
  tag: "Hidden by the tag filter",
  path: "Hidden by the path filter",
};
