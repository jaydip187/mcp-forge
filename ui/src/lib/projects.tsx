import { createContext, useCallback, useContext, useEffect, useState, type ReactNode } from "react";

import { api, ApiError } from "@/api";
import type { Project } from "@/types";

interface ProjectsState {
  projects: Project[] | null;
  error: string | null;
  reload: () => Promise<void>;
}

const Ctx = createContext<ProjectsState | null>(null);

export function ProjectsProvider({ children }: { children: ReactNode }) {
  const [projects, setProjects] = useState<Project[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const reload = useCallback(async () => {
    try {
      setProjects(await api.listProjects());
      setError(null);
    } catch (e) {
      setError(e instanceof ApiError ? e.message : String(e));
    }
  }, []);
  useEffect(() => {
    void reload();
  }, [reload]);
  return <Ctx.Provider value={{ projects, error, reload }}>{children}</Ctx.Provider>;
}

export function useProjects(): ProjectsState {
  const v = useContext(Ctx);
  if (!v) throw new Error("useProjects outside ProjectsProvider");
  return v;
}

/** Inline error text for one server-validated field. */
export function fieldError(err: unknown, field: string): string | undefined {
  if (!(err instanceof ApiError)) return undefined;
  return err.fields.find((f) => f.field === field)?.message;
}
