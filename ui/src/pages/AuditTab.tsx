import { useCallback, useEffect, useState } from "react";
import { RefreshCwIcon } from "lucide-react";

import { api, ApiError } from "@/api";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import type { AuditEntry } from "@/types";

const OUTCOME: Record<AuditEntry["outcome"], { label: string; variant: "default" | "secondary" | "destructive" | "outline" }> = {
  committed: { label: "Committed", variant: "destructive" },
  dry_run: { label: "Dry run", variant: "secondary" },
  rolled_back: { label: "Rolled back", variant: "outline" },
  failed: { label: "Failed", variant: "outline" },
};

/** Every SQL write attempt that reached a database, newest first. */
export function AuditTab({ projectId }: { projectId: string }) {
  const [entries, setEntries] = useState<AuditEntry[] | null>(null);
  const [more, setMore] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [open, setOpen] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      const page = await api.listAudit(projectId);
      setEntries(page);
      setMore(page.length === 100);
      setError(null);
    } catch (e) {
      setError(e instanceof ApiError ? e.message : String(e));
    }
  }, [projectId]);
  useEffect(() => {
    void load();
  }, [load]);

  async function older() {
    const last = entries?.at(-1);
    if (!last) return;
    const page = await api.listAudit(projectId, last.ts);
    setEntries([...(entries ?? []), ...page]);
    setMore(page.length === 100);
  }

  if (error)
    return (
      <Alert variant="destructive">
        <AlertDescription>{error}</AlertDescription>
      </Alert>
    );
  if (!entries) return <Skeleton className="h-40 w-full" />;

  return (
    <div className="space-y-3">
      <div className="flex items-center justify-between gap-4">
        <p className="max-w-prose text-sm text-muted-foreground">
          Every statement that changed, or tried to change, data through a SQL tool: from agents and from the test
          console. Statements refused before reaching the database aren't listed.
        </p>
        <Button variant="outline" size="sm" onClick={() => void load()}>
          <RefreshCwIcon /> Refresh
        </Button>
      </div>
      {entries.length === 0 ? (
        <div className="rounded-lg border border-dashed p-8 text-center text-sm text-muted-foreground">
          No writes yet. They appear here once a database allows writes and a tool changes data.
        </div>
      ) : (
        <div className="overflow-x-auto rounded-lg border bg-card">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>When</TableHead>
                <TableHead>Tool</TableHead>
                <TableHead>Outcome</TableHead>
                <TableHead className="text-right">Rows</TableHead>
                <TableHead>Statement</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {entries.map((e) => (
                <TableRow key={e.id} className="align-top">
                  <TableCell className="whitespace-nowrap text-xs text-muted-foreground" title={e.ts}>
                    {new Date(e.ts).toLocaleString()}
                  </TableCell>
                  <TableCell className="text-sm">
                    <div className="font-mono">{e.tool_name}</div>
                    <div className="text-xs text-muted-foreground">
                      {e.connection_name} · {e.origin === "mcp" ? "agent" : "test console"}
                      {e.statement_class === "ddl" && " · schema change"}
                    </div>
                  </TableCell>
                  <TableCell>
                    <Badge variant={OUTCOME[e.outcome].variant}>{OUTCOME[e.outcome].label}</Badge>
                  </TableCell>
                  <TableCell className="text-right font-mono text-sm">{e.rows_affected ?? "–"}</TableCell>
                  <TableCell className="max-w-md">
                    <button
                      type="button"
                      className="w-full text-left"
                      aria-expanded={open === e.id}
                      onClick={() => setOpen(open === e.id ? null : e.id)}
                    >
                      <code className={`block font-mono text-xs ${open === e.id ? "whitespace-pre-wrap break-all" : "truncate"}`}>
                        {e.sql}
                      </code>
                    </button>
                    {open === e.id && (
                      <div className="mt-2 space-y-1 text-xs text-muted-foreground">
                        {e.params.length > 0 && (
                          <p>
                            Values: <code className="font-mono">{e.params.map((v) => JSON.stringify(v)).join(", ")}</code>
                          </p>
                        )}
                        {e.error && <p className="text-destructive">{e.error}</p>}
                      </div>
                    )}
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </div>
      )}
      {more && (
        <Button variant="outline" size="sm" onClick={() => void older()}>
          Load older entries
        </Button>
      )}
    </div>
  );
}
