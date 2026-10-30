import { useEffect, useState } from "react";
import { CheckIcon, ShieldCheckIcon, XIcon } from "lucide-react";
import { toast } from "sonner";

import { api, ApiError } from "@/api";
import { EmptyState } from "@/components/EmptyState";
import { PageHeader } from "@/components/PageHeader";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { useApprovals } from "@/lib/approvals";
import { href } from "@/lib/router";
import type { PendingApproval } from "@/types";

/** Seconds left before `iso`, ticking. */
function useSecondsLeft(iso: string): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const t = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(t);
  }, []);
  return Math.max(0, Math.round((new Date(iso).getTime() - now) / 1000));
}

function countdown(s: number): string {
  const m = Math.floor(s / 60);
  return m ? `${m}:${String(s % 60).padStart(2, "0")}` : `${s}s`;
}

/** Calls from agents waiting for a person to approve them, across all projects. */
export function ApprovalsPage() {
  const { pending, reload } = useApprovals();
  return (
    <div className="space-y-6">
      <PageHeader
        title="Approvals"
        description="Agent calls to tools that require approval wait here. Approve to run the call, deny to refuse it. Calls nobody answers in time are refused."
      />
      {pending.length === 0 ? (
        <EmptyState
          icon={ShieldCheckIcon}
          title="Nothing waiting"
          description="When an agent calls a tool that requires approval, the call appears here. Turn approval on in a tool's editor."
        />
      ) : (
        <ul className="space-y-3">
          {pending.map((p) => (
            <li key={p.id}>
              <PendingCard p={p} onDone={reload} />
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

function PendingCard({ p, onDone }: { p: PendingApproval; onDone: () => Promise<void> }) {
  const left = useSecondsLeft(p.expires_at);
  const [busy, setBusy] = useState(false);

  async function answer(approve: boolean) {
    setBusy(true);
    try {
      await (approve ? api.approve(p.id) : api.deny(p.id));
      toast.success(approve ? `Approved: ${p.tool_name} is running` : `Denied: ${p.tool_name} won't run`);
    } catch (e) {
      // 404: answered elsewhere (in the agent's app) or timed out meanwhile.
      toast.error(e instanceof ApiError && e.status === 404 ? "This call was already answered or timed out." : String(e));
    } finally {
      setBusy(false);
      await onDone();
    }
  }

  return (
    <article className="rounded-xl border bg-card p-4" aria-label={`${p.tool_name} waiting for approval`}>
      <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
        <div className="min-w-0 space-y-1">
          <div className="flex flex-wrap items-center gap-2">
            <a href={href.tool(p.project_id, p.tool_id)} className="font-mono font-medium hover:underline">
              {p.tool_name}
            </a>
            <a href={href.project(p.project_id, "calls")} className="text-sm text-muted-foreground hover:underline">
              {p.project_name}
            </a>
          </div>
          <p className="text-xs text-muted-foreground">
            {p.client ?? "An agent"} · asked {new Date(p.created_at).toLocaleTimeString()}
            {p.asked_client && " · also asked in the agent's app"}
          </p>
        </div>
        <div className="flex shrink-0 items-center gap-2">
          <Badge variant="outline" className="font-mono tabular-nums" title="Refused automatically when this reaches zero">
            {countdown(left)}
          </Badge>
          <Button variant="outline" size="sm" disabled={busy} onClick={() => void answer(false)}>
            <XIcon /> Deny
          </Button>
          <Button size="sm" disabled={busy} onClick={() => void answer(true)}>
            <CheckIcon /> Approve
          </Button>
        </div>
      </div>
      <pre className="mt-3 max-h-60 overflow-auto rounded-lg border bg-muted/40 p-3 font-mono text-xs whitespace-pre-wrap break-all">
        {typeof p.arguments === "string" ? p.arguments : JSON.stringify(p.arguments, null, 2)}
      </pre>
    </article>
  );
}
