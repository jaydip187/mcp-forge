import { createContext, useCallback, useContext, useEffect, useRef, useState, type ReactNode } from "react";
import { toast } from "sonner";

import { api } from "@/api";
import { href, navigate } from "@/lib/router";
import type { PendingApproval } from "@/types";

/** How often the UI checks for calls waiting for approval. */
const POLL_MS = 3000;

interface ApprovalsState {
  pending: PendingApproval[];
  reload: () => Promise<void>;
}

const Ctx = createContext<ApprovalsState | null>(null);

/** Keeps the list of calls waiting for approval fresh, and toasts new ones. */
export function ApprovalsProvider({ children }: { children: ReactNode }) {
  const [pending, setPending] = useState<PendingApproval[]>([]);
  const seen = useRef<Set<string> | null>(null);

  const reload = useCallback(async () => {
    try {
      const list = await api.listApprovals();
      // The first load only records what's there; later loads announce new calls.
      if (seen.current) {
        for (const p of list) {
          if (!seen.current.has(p.id)) {
            toast(`${p.tool_name} is waiting for approval`, {
              description: `${p.project_name}${p.client ? ` · ${p.client}` : ""}`,
              action: { label: "Review", onClick: () => navigate(href.approvals()) },
            });
          }
        }
      }
      seen.current = new Set(list.map((p) => p.id));
      setPending(list);
    } catch {
      // A failed poll keeps the last list; the next one tries again.
    }
  }, []);

  useEffect(() => {
    void reload();
    const t = window.setInterval(() => void reload(), POLL_MS);
    return () => window.clearInterval(t);
  }, [reload]);

  return <Ctx.Provider value={{ pending, reload }}>{children}</Ctx.Provider>;
}

export function useApprovals(): ApprovalsState {
  const v = useContext(Ctx);
  if (!v) throw new Error("useApprovals outside ApprovalsProvider");
  return v;
}
