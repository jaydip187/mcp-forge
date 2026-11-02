import { useCallback, useEffect, useState } from "react";
import { BotIcon, CircleCheckIcon, DatabaseIcon, PlugZapIcon, PlusIcon, TriangleAlertIcon } from "lucide-react";
import { toast } from "sonner";

import { api, ApiError } from "@/api";
import { EmptyState } from "@/components/EmptyState";
import { StepDialog, type FormStep } from "@/components/StepDialog";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
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
  READ_ONLY_DRIVERS,
  isSqlDriver,
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
  duckdb: "/path/to/analytics.duckdb (or :memory: to query files only)",
  mssql: "mssql://reader:PASSWORD@db.example.com:1433/shop",
  clickhouse: "https://reader:PASSWORD@db.example.com:8443/shop",
  mongodb: "mongodb://reader:PASSWORD@db.example.com:27017/shop (or mongodb+srv://…/shop)",
  redis: "redis://:PASSWORD@cache.example.com:6379/0 (rediss:// for TLS)",
  dynamodb: "dynamodb://ACCESS_KEY_ID:SECRET@us-east-1 (percent-encode / and + in the secret; ?endpoint=… for DynamoDB Local)",
  elasticsearch: "https://reader:PASSWORD@search.example.com:9200 (or ?api_key=KEY)",
};

/** NoSQL exploration tools, shown under the SQL mode names the API accepts. */
const NOSQL_MODE: Record<string, ExplorationMode> = {
  list_targets: "list_tables",
  describe_target: "describe_table",
  run_query: "run_sql",
};

const NOSQL_EXPLORATION_LABEL: Record<ExplorationMode, { title: string; blurb: string }> = {
  list_tables: { title: "List", blurb: "Lists the collections, tables, indexes or keys the agent may use." },
  describe_table: { title: "Describe", blurb: "Shows one collection's fields, a table's keys, an index mapping or a key's type." },
  run_sql: {
    title: "Run query",
    blurb: "Lets the agent write its own read-only operation against allowed names. Most flexible, so most to review.",
  },
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
  file_allowlist: [],
};

export function DatabasesTab({ projectId }: { projectId: string }) {
  const [conns, setConns] = useState<DbConnection[] | null>(null);
  const [editing, setEditing] = useState<DbConnection | "new" | null>(null);
  const [confirmDelete, setConfirmDelete] = useState<string | null>(null);
  const [reports, setReports] = useState<Record<string, ConnectionReport | "running">>({});
  const [tools, setTools] = useState<Tool[]>([]);
  const [agentPanel, setAgentPanel] = useState<string | null>(null);

  const load = useCallback(async () => {
    const [cs, ts] = await Promise.all([api.listConnections(projectId), api.listTools(projectId)]);
    setConns(cs);
    setTools(ts);
  }, [projectId]);

  /** Exploration tools a connection already has, by mode (NoSQL ones under the SQL mode names). */
  const explorationTools = (c: DbConnection) =>
    new Map(
      tools.flatMap((t) => {
        if (t.kind.type === "sql" && t.kind.connection_id === c.id && t.kind.mode.type !== "query")
          return [[t.kind.mode.type, t] as const];
        if (t.kind.type === "nosql" && t.kind.connection_id === c.id && t.kind.operation in NOSQL_MODE)
          return [[NOSQL_MODE[t.kind.operation]!, t] as const];
        return [];
      }),
    );
  useEffect(() => {
    void load();
  }, [load]);

  async function remove(c: DbConnection) {
    try {
      await api.deleteConnection(projectId, c.id);
      toast.success(`Deleted ${c.name}`);
      setConfirmDelete(null);
      if (editing !== "new" && editing?.id === c.id) setEditing(null);
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
    <div className="space-y-4">
      <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
        <p className="max-w-prose text-sm text-muted-foreground">
          A connection lets SQL tools query a database. Queries are read-only: each one runs in a read-only
          transaction with a time limit, and anything but a single SELECT is refused before it reaches the database.
        </p>
        <Button className="shrink-0" onClick={() => setEditing("new")}>
          <PlusIcon /> Add database
        </Button>
      </div>
      {!conns ? (
        <Skeleton className="h-24 w-full" />
      ) : conns.length === 0 ? (
        <EmptyState
          icon={DatabaseIcon}
          title="No databases yet"
          description="Add a connection, then create database tools for it on the Tools tab."
        >
          <Button onClick={() => setEditing("new")}>Add a database</Button>
        </EmptyState>
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
                      {(() => {
                        const what = isSqlDriver(c.driver) ? "table" : "name";
                        const n = c.table_allowlist.length;
                        return ` · ${n ? `${n} allowed ${what}${n > 1 ? "s" : ""}` : `all ${what}s allowed`}`;
                      })()}
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

      {editing && (
        <ConnectionForm
          key={editing === "new" ? "new" : editing.id}
          projectId={projectId}
          existing={editing === "new" ? null : editing}
          onDone={async () => {
            setEditing(null);
            await load();
          }}
          onClose={() => setEditing(null)}
        />
      )}
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
        {conn.table_allowlist.length
          ? `the ${conn.table_allowlist.length} allowed ${isSqlDriver(conn.driver) ? "tables" : "names"}`
          : isSqlDriver(conn.driver)
            ? "non-system tables"
            : "non-system names"}
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
                  {(isSqlDriver(conn.driver) ? EXPLORATION_LABEL : NOSQL_EXPLORATION_LABEL)[m].title}
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
                <span className="text-xs text-muted-foreground">{(isSqlDriver(conn.driver) ? EXPLORATION_LABEL : NOSQL_EXPLORATION_LABEL)[m].blurb}</span>
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
  onClose,
}: {
  projectId: string;
  existing: DbConnection | null;
  onDone: () => Promise<void>;
  onClose: () => void;
}) {
  const [draft, setDraft] = useState<DbConnectionInput>(existing ?? EMPTY);
  const [masksText, setMasksText] = useState((existing?.column_masks ?? []).join("\n"));
  const [allowText, setAllowText] = useState((existing?.table_allowlist ?? []).join("\n"));
  const [filesText, setFilesText] = useState((existing?.file_allowlist ?? []).join("\n"));
  const readOnlyDriver = READ_ONLY_DRIVERS.includes(draft.driver);
  const [rowLimit, setRowLimit] = useState(String(draft.row_limit));
  const [timeout, setTimeoutText] = useState(String(draft.timeout_ms));
  const [maxRows, setMaxRows] = useState(String(draft.max_affected_rows));
  const [confirmFull, setConfirmFull] = useState(false);
  const [err, setErr] = useState<unknown>(null);
  // Confirmation is needed when this save turns full access on.
  const needsConfirm = draft.access_mode === "full_access" && existing?.access_mode !== "full_access";

  async function submit() {
    const input: DbConnectionInput = {
      ...draft,
      name: draft.name.trim(),
      dsn_ref: draft.dsn_ref.trim(),
      row_limit: Number(rowLimit),
      timeout_ms: Number(timeout),
      column_masks: splitList(masksText),
      table_allowlist: splitList(allowText),
      file_allowlist: draft.driver === "duckdb" ? splitList(filesText) : [],
      max_affected_rows: Number(maxRows),
      allow_ddl: isSqlDriver(draft.driver) && draft.access_mode !== "read_only" && draft.allow_ddl,
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
      setErr(null);
      await onDone();
    } catch (e) {
      setErr(e);
    }
  }

  const fe = (f: string) => fieldError(err, f);
  const maskErr = err instanceof ApiError ? err.fields.find((f) => f.field.startsWith("column_masks"))?.message : undefined;
  const allowErr = err instanceof ApiError ? err.fields.find((f) => f.field.startsWith("table_allowlist"))?.message : undefined;
  const filesErr = err instanceof ApiError ? err.fields.find((f) => f.field.startsWith("file_allowlist"))?.message : undefined;
  const steps: FormStep[] = [
    {
      title: "Connection",
      description: "Which database, and where its connection string lives. The string itself stays outside mcp-forge, like other secrets.",
      fields: ["name", "driver", "dsn_ref"],
      content: (
        <>
          <div className="grid gap-1.5">
            <Label htmlFor="d-name">Name</Label>
            <Input id="d-name" value={draft.name} onChange={(e) => setDraft({ ...draft, name: e.target.value })} placeholder="shop" required aria-invalid={!!fe("name")} />
            {fe("name") && <p className="text-xs text-destructive">{fe("name")}</p>}
          </div>
          <div className="grid gap-1.5">
            <Label htmlFor="d-driver">Database</Label>
            <Select
              value={draft.driver}
              onValueChange={(v) => {
                const driver = v as Driver;
                // Read-only drivers can't keep a write access mode.
                setDraft({ ...draft, driver, access_mode: READ_ONLY_DRIVERS.includes(driver) ? "read_only" : draft.access_mode });
              }}
            >
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
          {err instanceof ApiError && err.fields.some((f) => f.field === "dsn_ref") && (
            <p className="text-xs text-destructive">{fe("dsn_ref")}</p>
          )}
        </>
      ),
    },
    {
      title: "Limits and privacy",
      description: "How much one query may return, what agents may look at, and which values they never see.",
      fields: ["row_limit", "timeout_ms", "table_allowlist", "file_allowlist", "column_masks"],
      content: (
        <>
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
            <Label htmlFor="d-allow">{isSqlDriver(draft.driver) ? "Tables agents may explore" : "Names agents may use"}</Label>
            <Textarea id="d-allow" className="min-h-16 font-mono text-xs" value={allowText} onChange={(e) => setAllowText(e.target.value)} placeholder={isSqlDriver(draft.driver) ? "orders\nsales.*" : draft.driver === "redis" ? "user:*\ncart:*" : "orders\nlogs-*"} spellCheck={false} aria-invalid={!!allowErr} />
            {allowErr ? (
              <p className="text-xs text-destructive">{allowErr}</p>
            ) : (
              isSqlDriver(draft.driver) ? (
                <p className="text-xs text-muted-foreground">
                  One per line: <code>table</code>, <code>schema.table</code> or <code>schema.*</code>. Empty means every
                  non-system table. Applies to the agent tools (list, describe, run SQL), not to your own saved queries.
                </p>
              ) : (
                <p className="text-xs text-muted-foreground">
                  One per line: collection, table or index names, or patterns with <code>*</code> (Redis keys like{" "}
                  <code>user:*</code>). Empty means every non-system name. Applies to every NoSQL tool on this connection.
                </p>
              )
            )}
          </div>
          {draft.driver === "duckdb" && (
            <div className="grid gap-1.5">
              <Label htmlFor="d-files">Folders queries may read files from</Label>
              <Textarea id="d-files" className="min-h-16 font-mono text-xs" value={filesText} onChange={(e) => setFilesText(e.target.value)} placeholder={"/data/exports\n/srv/reports"} spellCheck={false} aria-invalid={!!filesErr} />
              {filesErr ? (
                <p className="text-xs text-destructive">{filesErr}</p>
              ) : (
                <p className="text-xs text-muted-foreground">
                  One absolute folder per line. Queries may then use <code>read_csv</code>, <code>read_parquet</code> and{" "}
                  <code>read_json</code> on files inside them; anything else on disk, URLs and other databases stay off.
                  Empty means no files at all.
                </p>
              )}
            </div>
          )}
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
        </>
      ),
    },
    {
      title: "What agents may do",
      description: "Read only, or also write. Writes are capped, can be tried first and are all audited.",
      fields: ["access_mode", "max_affected_rows", "allow_ddl"],
      content: (
        <>
          <div className="grid gap-1.5">
            <Label htmlFor="d-mode">What agents may do</Label>
            <Select value={draft.access_mode} onValueChange={(v) => setDraft({ ...draft, access_mode: v as AccessMode })}>
              <SelectTrigger id="d-mode" className="w-full" aria-invalid={!!fe("access_mode")}>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {(Object.keys(MODE_LABEL) as AccessMode[]).map((m) => (
                  <SelectItem key={m} value={m} disabled={readOnlyDriver && m !== "read_only"}>
                    {MODE_LABEL[m].title}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            <p className="text-xs text-muted-foreground">
              {readOnlyDriver
                ? `${DRIVER_LABEL[draft.driver]} connections are read-only: without transactions, a write couldn't be capped or undone.`
                : MODE_LABEL[draft.access_mode].blurb}
            </p>
          </div>
          {draft.access_mode === "read_only" ? (
            <Alert>
              <AlertDescription>
                Agents can only read through this connection. For a second layer of safety, connect with a database
                login that can only read; <strong>Test</strong> tells you if it can write.
              </AlertDescription>
            </Alert>
          ) : (
            <>
              <div className="grid gap-1.5">
                <Label htmlFor="d-maxrows">{isSqlDriver(draft.driver) ? "Most rows one write may change" : "Most items one write may change"}</Label>
                <Input id="d-maxrows" inputMode="numeric" className="font-mono" value={maxRows} onChange={(e) => setMaxRows(e.target.value)} aria-invalid={!!fe("max_affected_rows")} />
                {fe("max_affected_rows") && <p className="text-xs text-destructive">{fe("max_affected_rows")}</p>}
                {isSqlDriver(draft.driver) ? (
                  <p className="text-xs text-muted-foreground">
                    A statement that changes more rows is undone. UPDATE and DELETE always need a WHERE clause, every write
                    can be tried first with <code>dry_run</code>, and all writes are listed on the Audit tab.
                  </p>
                ) : (
                  <p className="text-xs text-muted-foreground">
                    A write that would change more items is refused before it runs (MongoDB counts the matches first).
                    Every write can be tried first with <code>dry_run</code>, and all writes are listed on the Audit tab.
                  </p>
                )}
              </div>
              {isSqlDriver(draft.driver) && (
              <div className="flex items-start gap-2">
                <Checkbox id="d-ddl" className="mt-0.5" checked={draft.allow_ddl} onCheckedChange={(v) => setDraft({ ...draft, allow_ddl: v === true })} />
                <Label htmlFor="d-ddl" className="grid gap-0.5 font-normal">
                  <span className="font-medium">Allow schema changes</span>
                  <span className="text-xs text-muted-foreground">CREATE, ALTER, DROP and TRUNCATE. Off unless you need it.</span>
                </Label>
              </div>
              )}
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
        </>
      ),
    },
  ];

  return (
    <StepDialog
      title={existing ? `Edit ${existing.name}` : "Add database"}
      steps={steps}
      errorFields={err instanceof ApiError ? err.fields.map((f) => f.field) : []}
      banner={
        <>
                    {err instanceof ApiError && err.fields.length === 0 && (
                      <Alert variant="destructive">
                        <AlertDescription>
                          {err.status === 400 && err.message.includes("dsn_ref")
                            ? "That looks like a connection string, not a reference. Put it in an environment variable or the keyring and use env:NAME or keyring:NAME."
                            : err.message}
                        </AlertDescription>
                      </Alert>
                    )}
        </>
      }
      submitLabel={existing ? "Save database" : "Add database"}
      onSubmit={submit}
      onClose={onClose}
    />
  );
}
