import { useCallback, useEffect, useState, type FormEvent } from "react";
import { BotIcon, CircleCheckIcon, DatabaseIcon, PlugZapIcon, TriangleAlertIcon } from "lucide-react";
import { toast } from "sonner";

import { api, ApiError } from "@/api";
import { EmptyState } from "@/components/EmptyState";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { Textarea } from "@/components/ui/textarea";
import { fieldError } from "@/lib/projects";
import { href } from "@/lib/router";
import { EXPLORATION_LABEL, splitList } from "@/lib/sqlForm";
import {
  DRIVER_LABEL,
  DRIVERS,
  EXPLORATION_MODES,
  type ConnectionReport,
  type DbConnection,
  type AccessMode,
  type DbConnectionInput,
  type Driver,
  type ExplorationMode,
  type Tool,
} from "@/types";

const DSN_EXAMPLE: Record<Driver, string> = {
  postgres: "postgres://readonly:PASSWORD@db.example.com:5432/shop",
  mysql: "mysql://readonly:PASSWORD@db.example.com:3306/shop",
  sqlite: "/path/to/shop.db",
};

export const MODE_LABEL: Record<AccessMode, { short: string; title: string; blurb: string }> = {
  read_only: {
    short: "read-only",
    title: "Read only",
    blurb: "Every tool only reads. Recommended.",
  },
  curated_write: {
    short: "saved writes",
    title: "Saved writes",
    blurb: "SQL tools you write may also INSERT, UPDATE or DELETE. Agent-written run_sql stays read-only.",
  },
  full_access: {
    short: "full access",
    title: "Full access",
    blurb: "run_sql may also change data with SQL the agent writes itself.",
  },
};

const EMPTY: DbConnectionInput = {
  name: "",
  driver: "postgres",
  dsn_ref: "env:",
  access_mode: "read_only",
  row_limit: 100,
  timeout_ms: 10000,
  max_affected_rows: 100,
  allow_ddl: false,
  table_allowlist: [],
  column_masks: [],
};

export function DatabasesTab({ projectId }: { projectId: string }) {
  const [conns, setConns] = useState<DbConnection[] | null>(null);
  const [editing, setEditing] = useState<DbConnection | null>(null);
  const [confirmDelete, setConfirmDelete] = useState<string | null>(null);
  const [reports, setReports] = useState<Record<string, ConnectionReport | "running">>({});
  const [tools, setTools] = useState<Tool[]>([]);
  const [agentPanel, setAgentPanel] = useState<string | null>(null);

  const load = useCallback(async () => {
    const [cs, ts] = await Promise.all([api.listConnections(projectId), api.listTools(projectId)]);
    setConns(cs);
    setTools(ts);
  }, [projectId]);

  /** Exploration tools a connection already has, by mode. */
  const explorationTools = (c: DbConnection) =>
    new Map(
      tools.flatMap((t) =>
        t.kind.type === "sql" && t.kind.connection_id === c.id && t.kind.mode.type !== "query"
          ? [[t.kind.mode.type, t] as const]
          : [],
      ),
    );
  useEffect(() => {
    void load();
  }, [load]);

  async function remove(c: DbConnection) {
    try {
      await api.deleteConnection(projectId, c.id);
      toast.success(`Deleted ${c.name}`);
      setConfirmDelete(null);
      if (editing?.id === c.id) setEditing(null);
      await load();
    } catch (e) {
      toast.error(e instanceof ApiError ? e.message : String(e));
    }
  }

  async function test(c: DbConnection) {
    setReports((r) => ({ ...r, [c.id]: "running" }));
    try {
      const report = await api.testConnection(projectId, c.id);
      setReports((r) => ({ ...r, [c.id]: report }));
    } catch (e) {
      setReports((r) => {
        const { [c.id]: _, ...rest } = r;
        return rest;
      });
      toast.error(e instanceof ApiError ? e.message : String(e));
    }
  }

  return (
    <div className="grid items-start gap-6 lg:grid-cols-[minmax(0,1fr)_24rem]">
      <section className="min-w-0 space-y-3">
        <p className="max-w-prose text-sm text-muted-foreground">
          A connection lets SQL tools query a database. Queries are read-only: each one runs in a read-only
          transaction with a time limit, and anything but a single SELECT is refused before it reaches the database.
        </p>
        {!conns ? (
          <Skeleton className="h-24 w-full" />
        ) : conns.length === 0 ? (
          <EmptyState
            icon={DatabaseIcon}
            title="No databases yet"
            description="Add a connection with the form, then create SQL tools for it on the Tools tab."
          />
        ) : (
          <ul className="divide-y overflow-hidden rounded-xl border bg-card">
            {conns.map((c) => {
              const report = reports[c.id];
              return (
                <li key={c.id} className="space-y-3 px-4 py-3">
                  <div className="flex flex-wrap items-center gap-3">
                    <div className="min-w-0 flex-1">
                      <p className="font-medium">{c.name}</p>
                      <p className="text-sm text-muted-foreground">
                        {DRIVER_LABEL[c.driver]} · {MODE_LABEL[c.access_mode].short}
                        {c.access_mode !== "read_only" && ` (≤${c.max_affected_rows} rows per write${c.allow_ddl ? ", DDL on" : ""})`} · up to{" "}
                        {c.row_limit} rows · {c.timeout_ms / 1000}s limit
                        {c.column_masks.length > 0 && ` · ${c.column_masks.length} masked column${c.column_masks.length > 1 ? "s" : ""}`}
                        {` · ${c.table_allowlist.length ? `${c.table_allowlist.length} allowed table${c.table_allowlist.length > 1 ? "s" : ""}` : "all tables allowed"}`}
                      </p>
                      <p className="font-mono text-xs text-muted-foreground">{c.dsn_ref}</p>
                    </div>
                    {confirmDelete === c.id ? (
                      <div className="flex gap-2">
                        <Button size="sm" variant="destructive" onClick={() => void remove(c)}>
                          Delete
                        </Button>
                        <Button size="sm" variant="outline" onClick={() => setConfirmDelete(null)}>
                          Keep
                        </Button>
                      </div>
                    ) : (
                      <div className="flex flex-wrap gap-2">
                        <Button size="sm" variant="outline" onClick={() => void test(c)} disabled={report === "running"}>
                          <PlugZapIcon /> {report === "running" ? "Testing…" : "Test"}
                        </Button>
                        <Button size="sm" variant="outline" onClick={() => setAgentPanel(agentPanel === c.id ? null : c.id)} aria-expanded={agentPanel === c.id}>
                          <BotIcon /> Agent tools
                        </Button>
                        <Button size="sm" variant="outline" onClick={() => setEditing(c)}>
                          Edit
                        </Button>
                        <Button size="sm" variant="ghost" className="text-destructive" onClick={() => setConfirmDelete(c.id)}>
                          Delete
                        </Button>
                      </div>
                    )}
                  </div>
                  {report && report !== "running" && <Report report={report} />}
                  {agentPanel === c.id && (
                    <AgentTools
                      projectId={projectId}
                      conn={c}
                      existing={explorationTools(c)}
                      onAdded={async () => {
                        await load();
                      }}
                    />
                  )}
                </li>
              );
            })}
          </ul>
        )}
      </section>

      <ConnectionForm
        key={editing?.id ?? "new"}
        projectId={projectId}
        existing={editing}
        onDone={async () => {
          setEditing(null);
          await load();
        }}
      />
    </div>
  );
}

/** Opt-in exploration tools for one connection. */
function AgentTools({
  projectId,
  conn,
  existing,
  onAdded,
}: {
  projectId: string;
  conn: DbConnection;
  existing: Map<ExplorationMode, Tool>;
  onAdded: () => Promise<void>;
}) {
  const [picked, setPicked] = useState<Set<ExplorationMode>>(
    () => new Set(EXPLORATION_MODES.filter((m) => m !== "run_sql" && !existing.has(m))),
  );
  const [busy, setBusy] = useState(false);
  const missing = EXPLORATION_MODES.filter((m) => !existing.has(m));

  async function add() {
    setBusy(true);
    try {
      const r = await api.addExplorationTools(projectId, conn.id, [...picked]);
      toast.success(`Added ${r.created.map((t) => t.name).join(", ")}`);
      setPicked(new Set());
      await onAdded();
    } catch (e) {
      toast.error(e instanceof ApiError ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="space-y-3 rounded-md border bg-muted/40 p-3">
      <p className="text-sm text-muted-foreground">
        Let agents explore this database themselves. Everything stays read-only and limited to{" "}
        {conn.table_allowlist.length ? `the ${conn.table_allowlist.length} allowed tables` : "non-system tables"}
        {conn.column_masks.length > 0 && ", with masked columns kept out of reach"}.
      </p>
      <ul className="grid gap-2">
        {EXPLORATION_MODES.map((m) => {
          const tool = existing.get(m);
          return (
            <li key={m} className="flex items-start gap-2">
              {tool ? (
                <CircleCheckIcon className="mt-0.5 size-4 shrink-0 text-muted-foreground" aria-hidden />
              ) : (
                <Checkbox
                  id={`x-${conn.id}-${m}`}
                  className="mt-0.5"
                  checked={picked.has(m)}
                  onCheckedChange={(v) => {
                    const next = new Set(picked);
                    if (v === true) next.add(m);
                    else next.delete(m);
                    setPicked(next);
                  }}
                />
              )}
              <Label htmlFor={`x-${conn.id}-${m}`} className="grid gap-0.5 font-normal">
                <span className="font-medium">
                  {EXPLORATION_LABEL[m].title}
                  {tool && (
                    <>
                      {" "}
                      ·{" "}
                      <a className="font-mono text-xs underline" href={href.tool(projectId, tool.id)}>
                        {tool.name}
                      </a>
                    </>
                  )}
                </span>
                <span className="text-xs text-muted-foreground">{EXPLORATION_LABEL[m].blurb}</span>
              </Label>
            </li>
          );
        })}
      </ul>
      {missing.length > 0 ? (
        <Button size="sm" onClick={() => void add()} disabled={busy || picked.size === 0}>
          {busy ? "Adding…" : "Add selected tools"}
        </Button>
      ) : (
        <p className="text-xs text-muted-foreground">All three are added. Turn them off or delete them on the Tools tab.</p>
      )}
    </div>
  );
}

function Report({ report }: { report: ConnectionReport }) {
  if (!report.ok)
    return (
      <Alert variant="destructive">
        <TriangleAlertIcon />
        <AlertTitle>Couldn't connect</AlertTitle>
        <AlertDescription>{report.error}</AlertDescription>
      </Alert>
    );
  const who = [report.server_version, report.user && `as ${report.user}`].filter(Boolean).join(" ");
  if (report.can_write)
    return (
      <Alert>
        <TriangleAlertIcon className="text-m-put" />
        <AlertTitle>Connected to {who}, but this login can change data</AlertTitle>
        <AlertDescription>
          <ul className="list-disc pl-4">
            {report.write_privileges.map((p) => (
              <li key={p}>{p}</li>
            ))}
          </ul>
          <p className="mt-1">
            Agents still can't write through mcp-forge, but a login that may only SELECT is the safest setup.
          </p>
        </AlertDescription>
      </Alert>
    );
  return (
    <Alert>
      <CircleCheckIcon />
      <AlertTitle>Connected to {who}</AlertTitle>
      <AlertDescription>
        {report.can_write === false ? "This login can only read." : "Couldn't tell whether this login can write."} Took{" "}
        {report.elapsed_ms} ms.
        {report.warnings.length > 0 && (
          <ul className="list-disc pl-4">
            {report.warnings.map((w) => (
              <li key={w}>{w}</li>
            ))}
          </ul>
        )}
      </AlertDescription>
    </Alert>
  );
}

function ConnectionForm({
  projectId,
  existing,
  onDone,
}: {
  projectId: string;
  existing: DbConnection | null;
  onDone: () => Promise<void>;
}) {
  const [draft, setDraft] = useState<DbConnectionInput>(existing ?? EMPTY);
  const [masksText, setMasksText] = useState((existing?.column_masks ?? []).join("\n"));
  const [allowText, setAllowText] = useState((existing?.table_allowlist ?? []).join("\n"));
  const [rowLimit, setRowLimit] = useState(String(draft.row_limit));
  const [timeout, setTimeoutText] = useState(String(draft.timeout_ms));
  const [maxRows, setMaxRows] = useState(String(draft.max_affected_rows));
  const [confirmFull, setConfirmFull] = useState(false);
  const [err, setErr] = useState<unknown>(null);
  // Confirmation is needed when this save turns full access on.
  const needsConfirm = draft.access_mode === "full_access" && existing?.access_mode !== "full_access";

  async function submit(e: FormEvent) {
    e.preventDefault();
    const input: DbConnectionInput = {
      ...draft,
      name: draft.name.trim(),
      dsn_ref: draft.dsn_ref.trim(),
      row_limit: Number(rowLimit),
      timeout_ms: Number(timeout),
      column_masks: splitList(masksText),
      table_allowlist: splitList(allowText),
      max_affected_rows: Number(maxRows),
      allow_ddl: draft.access_mode !== "read_only" && draft.allow_ddl,
    };
    if (needsConfirm && !confirmFull) {
      setErr(new ApiError(422, "validation", "Confirm full access first.", [
        { field: "access_mode", message: "Tick the box to confirm that agents may change and delete data." },
      ]));
      return;
    }
    try {
      if (existing) await api.updateConnection(projectId, existing.id, input, confirmFull);
      else await api.createConnection(projectId, input, confirmFull);
      toast.success(existing ? `Saved ${input.name}` : `Added ${input.name}`);
      if (!existing) {
        setDraft(EMPTY);
        setMasksText("");
        setAllowText("");
        setRowLimit(String(EMPTY.row_limit));
        setTimeoutText(String(EMPTY.timeout_ms));
        setMaxRows(String(EMPTY.max_affected_rows));
        setConfirmFull(false);
      }
      setErr(null);
      await onDone();
    } catch (e) {
      setErr(e);
    }
  }

  const fe = (f: string) => fieldError(err, f);
  const maskErr = err instanceof ApiError ? err.fields.find((f) => f.field.startsWith("column_masks"))?.message : undefined;
  const allowErr = err instanceof ApiError ? err.fields.find((f) => f.field.startsWith("table_allowlist"))?.message : undefined;
  return (
    <Card className="h-fit">
      <CardHeader>
        <CardTitle>{existing ? `Edit ${existing.name}` : "Add database"}</CardTitle>
        <CardDescription>The connection string stays outside mcp-forge, like other secrets.</CardDescription>
      </CardHeader>
      <CardContent>
        <form onSubmit={submit} className="grid gap-4">
          <div className="grid gap-1.5">
            <Label htmlFor="d-name">Name</Label>
            <Input id="d-name" value={draft.name} onChange={(e) => setDraft({ ...draft, name: e.target.value })} placeholder="shop" required aria-invalid={!!fe("name")} />
            {fe("name") && <p className="text-xs text-destructive">{fe("name")}</p>}
          </div>
          <div className="grid gap-1.5">
            <Label htmlFor="d-driver">Database</Label>
            <Select value={draft.driver} onValueChange={(v) => setDraft({ ...draft, driver: v as Driver })}>
              <SelectTrigger id="d-driver" className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {DRIVERS.map((d) => (
                  <SelectItem key={d} value={d}>
                    {DRIVER_LABEL[d]}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <div className="grid gap-1.5">
            <Label htmlFor="d-ref">Connection string reference</Label>
            <Input id="d-ref" className="font-mono" value={draft.dsn_ref} onChange={(e) => setDraft({ ...draft, dsn_ref: e.target.value })} placeholder="env:SHOP_DATABASE_URL" spellCheck={false} autoComplete="off" />
            <p className="text-xs text-muted-foreground">
              <code>env:NAME</code> or <code>keyring:NAME</code> holding something like{" "}
              <code className="break-all">{DSN_EXAMPLE[draft.driver]}</code>.
            </p>
          </div>
          <div className="grid grid-cols-2 gap-2">
            <div className="grid gap-1.5">
              <Label htmlFor="d-rows">Row limit</Label>
              <Input id="d-rows" inputMode="numeric" className="font-mono" value={rowLimit} onChange={(e) => setRowLimit(e.target.value)} aria-invalid={!!fe("row_limit")} />
            </div>
            <div className="grid gap-1.5">
              <Label htmlFor="d-timeout">Time limit (ms)</Label>
              <Input id="d-timeout" inputMode="numeric" className="font-mono" value={timeout} onChange={(e) => setTimeoutText(e.target.value)} aria-invalid={!!fe("timeout_ms")} />
            </div>
            {(fe("row_limit") || fe("timeout_ms")) && (
              <p className="col-span-2 text-xs text-destructive">{fe("row_limit") ?? fe("timeout_ms")}</p>
            )}
          </div>
          <div className="grid gap-1.5">
            <Label htmlFor="d-allow">Tables agents may explore</Label>
            <Textarea id="d-allow" className="min-h-16 font-mono text-xs" value={allowText} onChange={(e) => setAllowText(e.target.value)} placeholder={"orders\nsales.*"} spellCheck={false} aria-invalid={!!allowErr} />
            {allowErr ? (
              <p className="text-xs text-destructive">{allowErr}</p>
            ) : (
              <p className="text-xs text-muted-foreground">
                One per line: <code>table</code>, <code>schema.table</code> or <code>schema.*</code>. Empty means every
                non-system table. Applies to the agent tools (list, describe, run SQL), not to your own saved queries.
              </p>
            )}
          </div>
          <div className="grid gap-1.5">
            <Label htmlFor="d-masks">Masked columns</Label>
            <Textarea id="d-masks" className="min-h-16 font-mono text-xs" value={masksText} onChange={(e) => setMasksText(e.target.value)} placeholder={"users.password_hash\n*.email"} spellCheck={false} aria-invalid={!!maskErr} />
            {maskErr ? (
              <p className="text-xs text-destructive">{maskErr}</p>
            ) : (
              <p className="text-xs text-muted-foreground">
                One per line. Values in these result columns are replaced with [masked]. <code>*.email</code> matches any table.
              </p>
            )}
          </div>
          <div className="grid gap-1.5">
            <Label htmlFor="d-mode">What agents may do</Label>
            <Select value={draft.access_mode} onValueChange={(v) => setDraft({ ...draft, access_mode: v as AccessMode })}>
              <SelectTrigger id="d-mode" className="w-full" aria-invalid={!!fe("access_mode")}>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {(Object.keys(MODE_LABEL) as AccessMode[]).map((m) => (
                  <SelectItem key={m} value={m}>
                    {MODE_LABEL[m].title}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            <p className="text-xs text-muted-foreground">{MODE_LABEL[draft.access_mode].blurb}</p>
          </div>
          {draft.access_mode === "read_only" ? (
            <Alert>
              <AlertDescription>
                Agents can only read through this connection. For a second layer of safety, connect with a database
                login that has SELECT rights only; <strong>Test</strong> tells you if it can write.
              </AlertDescription>
            </Alert>
          ) : (
            <>
              <div className="grid gap-1.5">
                <Label htmlFor="d-maxrows">Most rows one write may change</Label>
                <Input id="d-maxrows" inputMode="numeric" className="font-mono" value={maxRows} onChange={(e) => setMaxRows(e.target.value)} aria-invalid={!!fe("max_affected_rows")} />
                {fe("max_affected_rows") && <p className="text-xs text-destructive">{fe("max_affected_rows")}</p>}
                <p className="text-xs text-muted-foreground">
                  A statement that changes more rows is undone. UPDATE and DELETE always need a WHERE clause, every write
                  can be tried first with <code>dry_run</code>, and all writes are listed on the Audit tab.
                </p>
              </div>
              <div className="flex items-start gap-2">
                <Checkbox id="d-ddl" className="mt-0.5" checked={draft.allow_ddl} onCheckedChange={(v) => setDraft({ ...draft, allow_ddl: v === true })} />
                <Label htmlFor="d-ddl" className="grid gap-0.5 font-normal">
                  <span className="font-medium">Allow schema changes</span>
                  <span className="text-xs text-muted-foreground">CREATE, ALTER, DROP and TRUNCATE. Off unless you need it.</span>
                </Label>
              </div>
            </>
          )}
          {draft.access_mode === "full_access" && (
            <Alert variant="destructive">
              <TriangleAlertIcon />
              <AlertTitle>Agents will be able to change and delete data</AlertTitle>
              <AlertDescription>
                <p>
                  With full access, an agent can write its own INSERT, UPDATE and DELETE statements against{" "}
                  {draft.table_allowlist.length || allowText.trim() ? "the allowed tables" : "every non-system table"}.
                  A wrong or manipulated prompt can destroy data. Prefer saved write tools, keep the row cap low, and back
                  up the database.
                </p>
                {needsConfirm && (
                  <div className="mt-2 flex items-start gap-2">
                    <Checkbox id="d-confirm" className="mt-0.5" checked={confirmFull} onCheckedChange={(v) => setConfirmFull(v === true)} />
                    <Label htmlFor="d-confirm" className="font-normal">
                      I understand agents may change and delete data in {draft.name.trim() || "this database"}.
                    </Label>
                  </div>
                )}
              </AlertDescription>
            </Alert>
          )}
          {fe("access_mode") && <p className="text-xs text-destructive">{fe("access_mode")}</p>}
          {err instanceof ApiError && err.fields.length === 0 && (
            <Alert variant="destructive">
              <AlertDescription>
                {err.status === 400 && err.message.includes("dsn_ref")
                  ? "That looks like a connection string, not a reference. Put it in an environment variable or the keyring and use env:NAME or keyring:NAME."
                  : err.message}
              </AlertDescription>
            </Alert>
          )}
          {err instanceof ApiError && err.fields.some((f) => f.field === "dsn_ref") && (
            <p className="text-xs text-destructive">{fe("dsn_ref")}</p>
          )}
          <div className="flex gap-2">
            <Button type="submit">{existing ? "Save database" : "Add database"}</Button>
            {existing && (
              <Button type="button" variant="ghost" onClick={() => void onDone()}>
                Cancel
              </Button>
            )}
          </div>
        </form>
      </CardContent>
    </Card>
  );
}
