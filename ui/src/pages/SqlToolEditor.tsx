import { useEffect, useMemo, useState, type FormEvent } from "react";
import { ArrowLeftIcon, PlusIcon, Trash2Icon } from "lucide-react";
import { toast } from "sonner";

import { api, ApiError } from "@/api";
import { Field, Section } from "@/components/FormSection";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import { href, navigate } from "@/lib/router";
import { buildSqlInput, enumText, EXPLORATION_LABEL, looksLikeWrite, newSqlForm, withEnum, paramType, sqlFormFromTool, sqlParamIndex, syncSqlParams, type SqlForm } from "@/lib/sqlForm";
import { TestConsole } from "@/pages/TestConsole";
import { DRIVER_LABEL, SQL_PARAM_TYPES, type DbConnection, type SqlParam, type SqlParamType, type Tool } from "@/types";

/** Editor for curated read-only SQL query tools. */
export function SqlToolEditor({ projectId, tool }: { projectId: string; tool: Tool | null }) {
  const [saved, setSaved] = useState<Tool | null>(tool);
  const [form, setForm] = useState<SqlForm>(tool ? sqlFormFromTool(tool) : newSqlForm());
  const [conns, setConns] = useState<DbConnection[] | null>(null);
  const [serverErr, setServerErr] = useState<ApiError | null>(null);
  const [localErr, setLocalErr] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);

  useEffect(() => {
    api.listConnections(projectId).then(
      (cs) => {
        setConns(cs);
        // New tools start on the only connection there is.
        setForm((f) => (!f.connection_id && cs.length === 1 ? { ...f, connection_id: cs[0]!.id } : f));
      },
      () => setConns([]),
    );
  }, [projectId]);

  const built = useMemo(() => buildSqlInput(form), [form]);
  const dirty = useMemo(() => {
    if (!saved || !built.ok) return !saved;
    const { name, description, enabled, tags, response_shape, kind } = saved;
    return JSON.stringify(built.input) !== JSON.stringify({ name, description, enabled, tags, response_shape, kind });
  }, [saved, built]);

  const set = (patch: Partial<SqlForm>) => setForm({ ...form, ...patch });
  const setParam = (i: number, patch: Partial<SqlParam>) =>
    set({ params: form.params.map((p, j) => (j === i ? { ...p, ...patch } : p)) });
  const err = (field: string) => localErr[field] ?? serverErr?.fields.find((f) => f.field === field)?.message;
  const paramErrors = (i: number) =>
    (serverErr?.fields ?? []).filter((f) => sqlParamIndex(f.field) === i).map((f) => f.message);
  const conn = conns?.find((c) => c.id === form.connection_id);

  async function save(e: FormEvent) {
    e.preventDefault();
    const result = buildSqlInput(form);
    if (!result.ok) {
      setLocalErr(result.fieldErrors);
      return;
    }
    setLocalErr({});
    setBusy(true);
    try {
      const t = saved ? await api.updateTool(projectId, saved.id, result.input) : await api.createTool(projectId, result.input);
      setServerErr(null);
      setSaved(t);
      toast.success(saved ? "Tool saved" : "Tool created");
      if (!saved) navigate(href.tool(projectId, t.id));
    } catch (e) {
      if (e instanceof ApiError) setServerErr(e);
      else toast.error(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function remove() {
    if (!saved) return;
    try {
      await api.deleteTool(projectId, saved.id);
      toast.success(`Deleted ${saved.name}`);
      navigate(href.project(projectId));
    } catch (e) {
      toast.error(e instanceof Error ? e.message : String(e));
    }
  }

  const general = (serverErr?.fields ?? []).filter((f) => f.field === "kind.mode.params" || f.field === "response_shape");

  return (
    <div className="space-y-6">
      <a href={href.project(projectId)} className="inline-flex items-center gap-1 text-sm text-muted-foreground hover:text-foreground">
        <ArrowLeftIcon className="size-4" /> Tools
      </a>
      <header className="space-y-1">
        <h1 className="text-2xl font-semibold tracking-tight">{saved ? saved.name : "New SQL tool"}</h1>
        <p className="text-sm text-muted-foreground">
          {form.mode !== "query" ? (
            "A database exploration tool. Everything it runs is read-only and limited to the allowed tables."
          ) : (
            <>
          A fixed, read-only query. Agents fill in the <code>:name</code> parameters; values are always sent separately from
          the SQL, never pasted into it.
            </>
          )}
        </p>
      </header>

      {conns && conns.length === 0 && (
        <Alert>
          <AlertTitle>Add a database first</AlertTitle>
          <AlertDescription>
            <span>
              SQL tools run on a database connection.{" "}
              <a className="underline" href={href.project(projectId, "databases")}>
                Add one on the Databases tab
              </a>
              .
            </span>
          </AlertDescription>
        </Alert>
      )}

      <div className="grid items-start gap-6 lg:grid-cols-[minmax(0,1fr)_24rem]">
        <form onSubmit={save} className="min-w-0 space-y-6" noValidate>
          {serverErr && (
            <Alert variant="destructive">
              <AlertTitle>{serverErr.code === "validation" ? "Fix these before saving" : "Couldn't save"}</AlertTitle>
              <AlertDescription>
                {serverErr.fields.length === 0 ? (
                  serverErr.message
                ) : (
                  <ul className="list-disc pl-4">
                    {(general.length ? general : serverErr.fields).map((f, i) => (
                      <li key={i}>{f.message}</li>
                    ))}
                  </ul>
                )}
              </AlertDescription>
            </Alert>
          )}

          <Section title="What the agent sees" description="The name and description are how an agent decides when to use this tool.">
            <Field id="s-name" label="Name" error={err("name")} hint="Letters, digits, _ and -. Up to 64 characters.">
              <Input id="s-name" value={form.name} onChange={(e) => set({ name: e.target.value })} className="font-mono" placeholder="find_orders" aria-invalid={!!err("name")} />
            </Field>
            <Field id="s-desc" label="Description" error={err("description")}>
              <Textarea id="s-desc" value={form.description} onChange={(e) => set({ description: e.target.value })} placeholder={form.mode === "query" ? "Recent orders for one customer: id, status, total and date, newest first." : "Optional. Extra guidance for agents, shown before the generated description."} className="min-h-20" />
            </Field>
            <Field id="s-tags" label="Tags" error={err("tags")} hint="Comma-separated. Project filters and tool search use them.">
              <Input id="s-tags" value={form.tagsText} onChange={(e) => set({ tagsText: e.target.value })} placeholder="orders, reporting" />
            </Field>
            <div className="flex items-center gap-2">
              <Switch id="s-enabled" checked={form.enabled} onCheckedChange={(v) => set({ enabled: v })} />
              <Label htmlFor="s-enabled">Visible to agents</Label>
            </div>
          </Section>

          {form.mode !== "query" && (
            <Section title={EXPLORATION_LABEL[form.mode].title} description={EXPLORATION_LABEL[form.mode].blurb}>
              <Field id="s-conn" label="Database" error={err("kind.connection_id")}>
                <Select value={form.connection_id || "none"} onValueChange={(v) => set({ connection_id: v === "none" ? "" : v })}>
                  <SelectTrigger id="s-conn" className="w-full">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {(conns ?? []).map((c) => (
                      <SelectItem key={c.id} value={c.id}>
                        {c.name} ({DRIVER_LABEL[c.driver]})
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </Field>
              <p className="text-sm text-muted-foreground">
                Agents get a description written from the database's settings (limits, allowed tables, masked columns),
                so it stays current when you change them. Anything you put in Description above is shown first.
              </p>
              {conn && (
                <p className="text-sm text-muted-foreground">
                  Allowed tables: {conn.table_allowlist.length ? conn.table_allowlist.join(", ") : "every non-system table"}.{" "}
                  <a className="underline" href={href.project(projectId, "databases")}>
                    Change on the Databases tab
                  </a>
                  .
                </p>
              )}
            </Section>
          )}

          {form.mode === "query" && (
            <>
          <Section title="Query" description="One SELECT statement. Writes, multiple statements and risky functions are refused when you save.">
            <Field id="s-conn" label="Database" error={err("kind.connection_id")}>
              <Select value={form.connection_id || "none"} onValueChange={(v) => set({ connection_id: v === "none" ? "" : v })}>
                <SelectTrigger id="s-conn" className="w-full" aria-invalid={!!err("kind.connection_id")}>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="none" disabled>
                    Pick a database
                  </SelectItem>
                  {(conns ?? []).map((c) => (
                    <SelectItem key={c.id} value={c.id}>
                      {c.name} ({DRIVER_LABEL[c.driver]})
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </Field>
            <Field
              id="s-sql"
              label="SQL"
              error={err("kind.mode.sql")}
              hint={
                conn?.driver === "postgres"
                  ? "Write :name for each value. Values are typed by their parameter; cast where Postgres needs it, e.g. :since::timestamptz."
                  : "Write :name for each value. Leaving this field adds a parameter for any new :name."
              }
            >
              <Textarea
                id="s-sql"
                value={form.sql}
                onChange={(e) => set({ sql: e.target.value })}
                onBlur={() => set({ params: syncSqlParams(form.sql, form.params) })}
                className="min-h-40 font-mono text-xs"
                spellCheck={false}
                placeholder={"SELECT id, status, total, created_at\nFROM orders\nWHERE customer_id = :customer_id\nORDER BY created_at DESC"}
                aria-invalid={!!err("kind.mode.sql")}
              />
            </Field>
            {conn && looksLikeWrite(form.sql) ? (
              conn.access_mode === "read_only" ? (
                <Alert variant="destructive">
                  <AlertDescription>
                    This looks like a write, but {conn.name} is read-only. Allow saved writes on the Databases tab first.
                  </AlertDescription>
                </Alert>
              ) : (
                <Alert>
                  <AlertDescription>
                    This tool changes data. Agents can try it with <code>dry_run</code>; a change of more than{" "}
                    {conn.max_affected_rows} rows is undone; UPDATE and DELETE need a WHERE clause; every run is listed on
                    the Audit tab.
                  </AlertDescription>
                </Alert>
              )
            ) : (
              conn && (
                <p className="text-xs text-muted-foreground">
                  Returns at most {conn.row_limit} rows and stops after {conn.timeout_ms / 1000}s (set on the database).
                </p>
              )
            )}
          </Section>

          <Section title="Parameters" description="Each :name in the SQL is an argument the agent fills in.">
            {form.params.length === 0 ? (
              <p className="text-sm text-muted-foreground">No parameters. Add :name to the SQL, or add one here.</p>
            ) : (
              <div className="grid gap-3">
                {form.params.map((p, i) => (
                  <div key={i} className="grid gap-2 rounded-md border p-3">
                    <div className="grid gap-2 sm:grid-cols-[1fr_8rem_auto_auto] sm:items-end">
                      <Field id={`sp-name-${i}`} label="Name">
                        <Input id={`sp-name-${i}`} value={p.name} onChange={(e) => setParam(i, { name: e.target.value })} className="font-mono" />
                      </Field>
                      <Field id={`sp-type-${i}`} label="Type">
                        <Select value={paramType(p)} onValueChange={(v) => setParam(i, { schema: { ...p.schema, type: v as SqlParamType } })}>
                          <SelectTrigger id={`sp-type-${i}`} className="w-full">
                            <SelectValue />
                          </SelectTrigger>
                          <SelectContent>
                            {SQL_PARAM_TYPES.map((t) => (
                              <SelectItem key={t} value={t}>
                                {t}
                              </SelectItem>
                            ))}
                          </SelectContent>
                        </Select>
                      </Field>
                      <div className="flex h-9 items-center gap-2">
                        <Checkbox id={`sp-req-${i}`} checked={p.required} onCheckedChange={(v) => setParam(i, { required: v === true })} />
                        <Label htmlFor={`sp-req-${i}`}>Required</Label>
                      </div>
                      <Button type="button" variant="ghost" size="icon" aria-label={`Remove ${p.name || "parameter"}`} onClick={() => set({ params: form.params.filter((_, j) => j !== i) })}>
                        <Trash2Icon />
                      </Button>
                    </div>
                    <Input value={p.description ?? ""} onChange={(e) => setParam(i, { description: e.target.value })} placeholder="What the agent should pass (optional)" aria-label={`Description of ${p.name}`} />
                    <Input
                      value={enumText(p)}
                      onChange={(e) => setParam(i, { schema: withEnum(p.schema, e.target.value, paramType(p)) })}
                      placeholder="Allowed values, comma-separated (optional), e.g. pending, shipped"
                      aria-label={`Allowed values of ${p.name}`}
                      className="font-mono text-xs"
                    />
                    {!p.required && <p className="text-xs text-muted-foreground">Left out by the agent, it's sent as NULL.</p>}
                    {paramErrors(i).map((m) => (
                      <p key={m} className="text-xs text-destructive">
                        {m}
                      </p>
                    ))}
                  </div>
                ))}
              </div>
            )}
            <div>
              <Button type="button" variant="outline" size="sm" onClick={() => set({ params: [...form.params, { name: "", schema: { type: "string" }, required: true, description: null }] })}>
                <PlusIcon /> Add parameter
              </Button>
            </div>
          </Section>

            </>
          )}

          <Section title="Response" description="Agents get {columns, rows, truncated}. Reshape it here if they need less.">
            <Field id="s-shape" label="Response shape (jq)" error={err("response_shape")} hint="Leave empty to return the result as is. The project's size limit still applies.">
              <Textarea id="s-shape" value={form.responseShape} onChange={(e) => set({ responseShape: e.target.value })} placeholder=".rows | map({id: .[0], status: .[1]})" className="min-h-16 font-mono text-xs" spellCheck={false} aria-invalid={!!err("response_shape")} />
            </Field>
          </Section>

          <div className="sticky bottom-0 -mx-1 flex flex-wrap items-center gap-3 border-t bg-background/95 px-1 py-3 backdrop-blur">
            <Button type="submit" disabled={busy || (!!saved && !dirty)}>
              {saved ? "Save tool" : "Create tool"}
            </Button>
            {saved && dirty && <span className="text-sm text-muted-foreground">Unsaved changes</span>}
            <span className="flex-1" />
            {saved &&
              (confirmDelete ? (
                <>
                  <Button type="button" variant="destructive" onClick={() => void remove()}>
                    Delete {saved.name}
                  </Button>
                  <Button type="button" variant="outline" onClick={() => setConfirmDelete(false)}>
                    Keep it
                  </Button>
                </>
              ) : (
                <Button type="button" variant="ghost" className="text-destructive" onClick={() => setConfirmDelete(true)}>
                  Delete tool
                </Button>
              ))}
          </div>
        </form>

        <aside className="lg:sticky lg:top-6">
          <TestConsole projectId={projectId} toolId={saved?.id ?? null} version={saved?.updated_at ?? ""} dirty={dirty} />
        </aside>
      </div>
    </div>
  );
}
