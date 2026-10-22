import { useCallback, useEffect, useState, type FormEvent } from "react";
import { CircleCheckIcon, PlugZapIcon, TriangleAlertIcon } from "lucide-react";
import { toast } from "sonner";

import { api, ApiError } from "@/api";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { Textarea } from "@/components/ui/textarea";
import { fieldError } from "@/lib/projects";
import { splitList } from "@/lib/sqlForm";
import { DRIVER_LABEL, DRIVERS, type ConnectionReport, type DbConnection, type DbConnectionInput, type Driver } from "@/types";

const DSN_EXAMPLE: Record<Driver, string> = {
  postgres: "postgres://readonly:PASSWORD@db.example.com:5432/shop",
  mysql: "mysql://readonly:PASSWORD@db.example.com:3306/shop",
  sqlite: "/path/to/shop.db",
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

  const load = useCallback(async () => {
    setConns(await api.listConnections(projectId));
  }, [projectId]);
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
    <div className="grid gap-6 lg:grid-cols-[1fr_24rem]">
      <section className="min-w-0 space-y-3">
        <p className="max-w-prose text-sm text-muted-foreground">
          A connection lets SQL tools query a database. Queries are read-only: each one runs in a read-only
          transaction with a time limit, and anything but a single SELECT is refused before it reaches the database.
        </p>
        {!conns ? (
          <Skeleton className="h-24 w-full" />
        ) : conns.length === 0 ? (
          <div className="rounded-lg border border-dashed p-8 text-center text-sm text-muted-foreground">
            No databases yet. Add a connection, then create SQL tools on the Tools tab.
          </div>
        ) : (
          <ul className="divide-y rounded-lg border bg-card">
            {conns.map((c) => {
              const report = reports[c.id];
              return (
                <li key={c.id} className="space-y-3 px-4 py-3">
                  <div className="flex flex-wrap items-center gap-3">
                    <div className="min-w-0 flex-1">
                      <p className="font-medium">{c.name}</p>
                      <p className="text-sm text-muted-foreground">
                        {DRIVER_LABEL[c.driver]} · read-only · up to {c.row_limit} rows · {c.timeout_ms / 1000}s limit
                        {c.column_masks.length > 0 && ` · ${c.column_masks.length} masked column${c.column_masks.length > 1 ? "s" : ""}`}
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
                      <div className="flex gap-2">
                        <Button size="sm" variant="outline" onClick={() => void test(c)} disabled={report === "running"}>
                          <PlugZapIcon /> {report === "running" ? "Testing…" : "Test"}
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
  const [rowLimit, setRowLimit] = useState(String(draft.row_limit));
  const [timeout, setTimeoutText] = useState(String(draft.timeout_ms));
  const [err, setErr] = useState<unknown>(null);

  async function submit(e: FormEvent) {
    e.preventDefault();
    const input: DbConnectionInput = {
      ...draft,
      name: draft.name.trim(),
      dsn_ref: draft.dsn_ref.trim(),
      row_limit: Number(rowLimit),
      timeout_ms: Number(timeout),
      column_masks: splitList(masksText),
    };
    try {
      if (existing) await api.updateConnection(projectId, existing.id, input);
      else await api.createConnection(projectId, input);
      toast.success(existing ? `Saved ${input.name}` : `Added ${input.name}`);
      if (!existing) {
        setDraft(EMPTY);
        setMasksText("");
        setRowLimit(String(EMPTY.row_limit));
        setTimeoutText(String(EMPTY.timeout_ms));
      }
      setErr(null);
      await onDone();
    } catch (e) {
      setErr(e);
    }
  }

  const fe = (f: string) => fieldError(err, f);
  const maskErr = err instanceof ApiError ? err.fields.find((f) => f.field.startsWith("column_masks"))?.message : undefined;
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
          <Alert>
            <AlertDescription>
              Agents can only read through this connection. For a second layer of safety, connect with a database
              login that has SELECT rights only; <strong>Test</strong> tells you if it can write.
            </AlertDescription>
          </Alert>
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
