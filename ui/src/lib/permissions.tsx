import { createContext, useContext, type ReactNode } from "react";

import type { Access, User } from "@/types";

interface Permissions {
  user: User | null;
  /** Admins, and servers with sign-in off, can do everything. */
  isAdmin: boolean;
  /** `write`, `read`, or null (no access) for a project. */
  access: (projectId: string) => Access | null;
}

const Ctx = createContext<Permissions>({ user: null, isAdmin: true, access: () => "write" });

/** What the signed-in account may do. The server enforces it; this only hides what it would refuse. */
export function PermissionsProvider({
  user,
  access,
  children,
}: {
  user: User | null;
  access: Record<string, Access> | null;
  children: ReactNode;
}) {
  const isAdmin = !user || user.role === "admin";
  const value: Permissions = {
    user,
    isAdmin,
    access: (pid) => (isAdmin ? "write" : (access?.[pid] ?? null)),
  };
  return <Ctx.Provider value={value}>{children}</Ctx.Provider>;
}

export function usePermissions(): Permissions {
  return useContext(Ctx);
}

/** Whether the signed-in account may change the project. */
export function useCanWrite(projectId: string): boolean {
  return usePermissions().access(projectId) === "write";
}

/**
 * Disables every input and button inside for read-only users (a disabled
 * fieldset); links keep working. `display: contents` keeps layout as is.
 */
export function WriteGuard({ projectId, children }: { projectId: string; children: ReactNode }) {
  const canWrite = useCanWrite(projectId);
  return (
    <fieldset disabled={!canWrite} className="contents">
      {children}
    </fieldset>
  );
}
