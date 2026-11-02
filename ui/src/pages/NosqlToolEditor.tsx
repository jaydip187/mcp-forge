import { useEffect, useMemo, useState, type FormEvent } from "react";
import { PlusIcon, Trash2Icon } from "lucide-react";
import { toast } from "sonner";

import { api, ApiError } from "@/api";
import { PageHeader } from "@/components/PageHeader";
import { Field, Section } from "@/components/FormSection";
import { StepDialog, type FormStep } from "@/components/StepDialog";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { ApprovalSwitch } from "@/components/ApprovalSwitch";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import { href, navigate } from "@/lib/router";
import {
  bodyExample,
  buildNosqlInput,
  newNosqlForm,
  nosqlFormFromTool,
  nosqlParamIndex,
  syncNosqlParams,
  targetLabel,
  type NosqlForm,
} from "@/lib/nosqlForm";
import { enumText, paramType, withEnum } from "@/lib/sqlForm";
import { TestConsole } from "@/pages/TestConsole";
import {
  DRIVER_LABEL,
  NOSQL_DRIVERS,
  NOSQL_OPERATIONS,
  SQL_PARAM_TYPES,
  nosqlWrites,
  type DbConnection,
  type SqlParam,
  type SqlParamType,
  type Tool,
} from "@/types";

const EXPLORATION_TEXT: Record<string, { title: string; blurb: string }> = {
  list_targets: { title: "List", blurb: "Lists the collections, tables, indexes or keys agents may use." },
  describe_target: { title: "Describe", blurb: "Shows one collection's fields, a table's keys, an index mapping or a key's type." },
  run_query: {
    title: "Run query",
    blurb: "Lets the agent write its own operation against allowed names. Most flexible, so most to review.",
  },
};

/** Editor for NoSQL tools: one operation on a MongoDB, Redis, DynamoDB or Elasticsearch connection. */
export function NosqlToolEditor({ projectId, tool }: { projectId: string; tool: Tool | null }) {
  const [saved, setSaved] = useState<Tool | null>(tool);
  const [form, setForm] = useState<NosqlForm>(tool ? nosqlFormFromTool(tool) : newNosqlForm());
  const [conns, setConns] = useState<DbConnection[] | null>(null);
  const [serverErr, setServerErr] = useState<ApiError | null>(null);
  const [localErr, setLocalErr] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);

  useEffect(() => {
    api.listConnections(projectId).then(
      (cs) => {
        const nosql = cs.filter((c) => NOSQL_DRIVERS.includes(c.driver));
        setConns(nosql);
        setForm((f) => (!f.connection_id && nosql.length === 1 ? { ...f, connection_id: nosql[0]!.id } : f));
      },
      () => setConns([]),
    );
  }, [projectId]);

  const built = useMemo(() => buildNosqlInput(form), [form]);
  const dirty = useMemo(() => {
    if (!saved || !built.ok) return !saved;
    const { name, description, enabled, tags, response_shape, require_approval, kind } = saved;
    return JSON.stringify(built.input) !== JSON.stringify({ name, description, enabled, tags, response_shape, require_approval, kind });
  }, [saved, built]);

  const set = (patch: Partial<NosqlForm>) => setForm({ ...form, ...patch });
  const setParam = (i: number, patch: Partial<SqlParam>) =>
    set({ params: form.params.map((p, j) => (j === i ? { ...p, ...patch } : p)) });
  const err = (field: string) => localErr[field] ?? serverErr?.fields.find((f) => f.field === field)?.message;
  const paramErrors = (i: number) =>
    (serverErr?.fields ?? []).filter((f) => nosqlParamIndex(f.field) === i).map((f) => f.message);
  const conn = conns?.find((c) => c.id === form.connection_id);
  const ops = conn ? NOSQL_OPERATIONS[conn.driver] ?? [] : [];
  const exploration = form.operation in EXPLORATION_TEXT;
  const writes = nosqlWrites(form.operation);

  async function save(e?: FormEvent) {
    e?.preventDefault();
    const result = buildNosqlInput(form);
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
      // The server may reorder the body's keys; show what it stored.
      setForm(nosqlFormFromTool(t));
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

  const steps: FormStep[] = [
    {
      title: "What the agent sees",
      description: "The name and description are how an agent decides when to use this tool.",
      fields: ["name", "description", "tags", "enabled", "require_approval"],
      content: (
        <>
          <Field id="n-name" label="Name" error={err("name")} hint="Letters, digits, _ and -. Up to 64 characters.">
            <Input id="n-name" value={form.name} onChange={(e) => set({ name: e.target.value })} className="font-mono" placeholder="find_customer" aria-invalid={!!err("name")} />
          </Field>
          <Field id="n-desc" label="Description" error={err("description")}>
            <Textarea id="n-desc" value={form.description} onChange={(e) => set({ description: e.target.value })} placeholder={exploration ? "Optional. Extra guidance for agents, shown before the generated description." : "One customer by email: name, plan and signup date."} className="min-h-20" />
          </Field>
          <Field id="n-tags" label="Tags" error={err("tags")} hint="Comma-separated. Project filters and tool search use them.">
            <Input id="n-tags" value={form.tagsText} onChange={(e) => set({ tagsText: e.target.value })} placeholder="customers" />
          </Field>
          <div className="flex items-center gap-2">
            <Switch id="n-enabled" checked={form.enabled} onCheckedChange={(v) => set({ enabled: v })} />
            <Label htmlFor="n-enabled">Visible to agents</Label>
          </div>
          <ApprovalSwitch id="n-approval" checked={form.requireApproval ?? writes} onChange={(v) => set({ requireApproval: v })} />
        </>
      ),
    },
    {
      title: "Operation",
      description: exploration ? "Uses the connection's allowed names, masks and limits." : "What runs on the database.",
      fields: ["kind.connection_id", "kind.operation", "kind.target", "kind.body"],
      content: (
        <>
          <div className="grid gap-3 sm:grid-cols-2">
            <Field id="n-conn" label="Database" error={err("kind.connection_id")}>
              <Select value={form.connection_id || "none"} onValueChange={(v) => set({ connection_id: v === "none" ? "" : v, operation: exploration ? form.operation : "" })}>
                <SelectTrigger id="n-conn" className="w-full" aria-invalid={!!err("kind.connection_id")}>
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
            {exploration ? (
              <Field id="n-op" label="Tool">
                <Input id="n-op" value={EXPLORATION_TEXT[form.operation]!.title} readOnly className="font-mono" />
              </Field>
            ) : (
              <Field id="n-op" label="Operation" error={err("kind.operation")}>
                <Select value={form.operation || "none"} onValueChange={(v) => set({ operation: v === "none" ? "" : v })}>
                  <SelectTrigger id="n-op" className="w-full font-mono" aria-invalid={!!err("kind.operation")}>
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="none" disabled>
                      {conn ? "Pick an operation" : "Pick a database first"}
                    </SelectItem>
                    {ops.map((o) => (
                      <SelectItem key={o.name} value={o.name} className="font-mono">
                        {o.name}
                        {o.write ? " (writes)" : ""}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </Field>
            )}
          </div>
          {!exploration && (
            <>
              <Field
                id="n-target"
                label={targetLabel(conn?.driver)}
                error={err("kind.target")}
                hint={conn?.driver === "redis" ? "The key. Templates work here, e.g. user:{{id}}." : "Templates work here too, e.g. logs-{{year}}."}
              >
                <Input
                  id="n-target"
                  value={form.target}
                  onChange={(e) => set({ target: e.target.value })}
                  onBlur={() => set({ params: syncNosqlParams(form) })}
                  className="font-mono"
                  placeholder={conn?.driver === "redis" ? "user:{{id}}" : conn?.driver === "elasticsearch" ? "products" : "customers"}
                  aria-invalid={!!err("kind.target")}
                />
              </Field>
              <Field
                id="n-body"
                label={conn?.driver === "redis" ? "Arguments (JSON array)" : "Body (JSON)"}
                error={err("kind.body")}
                hint="Write {{name}} where the agent's value goes. Leaving this field adds a parameter for any new {{name}}."
              >
                <Textarea
                  id="n-body"
                  value={form.bodyText}
                  onChange={(e) => set({ bodyText: e.target.value })}
                  onBlur={() => set({ params: syncNosqlParams(form) })}
                  className="min-h-40 font-mono text-xs"
                  spellCheck={false}
                  placeholder={bodyExample(conn?.driver, form.operation)}
                  aria-invalid={!!err("kind.body")}
                />
              </Field>
              {conn && (writes ? (
                conn.access_mode === "read_only" ? (
                  <Alert variant="destructive">
                    <AlertDescription>
                      This operation writes, but {conn.name} is read-only. Allow saved writes on the Databases tab first.
                    </AlertDescription>
                  </Alert>
                ) : (
                  <Alert>
                    <AlertDescription>
                      This tool changes data. Agents can try it with <code>dry_run</code>; at most {conn.max_affected_rows} items
                      may change per call, and every run is listed on the Audit tab.
                    </AlertDescription>
                  </Alert>
                )
              ) : (
                <p className="text-xs text-muted-foreground">
                  Returns at most {conn.row_limit} documents and stops after {conn.timeout_ms / 1000}s (set on the database).
                </p>
              ))}
            </>
          )}
        </>
      ),
    },
    ...(exploration
      ? []
      : [
          {
            title: "Parameters",
            description: "Each {{name}} in the target or body is an argument the agent fills in.",
            fields: ["kind.params"],
            content: (
              <>
                {form.params.length === 0 ? (
                  <p className="text-sm text-muted-foreground">No parameters. Add {"{{name}}"} to the target or body, or add one here.</p>
                ) : (
                  <div className="grid gap-3">
                    {form.params.map((p, i) => (
                      <div key={i} className="grid gap-2 rounded-md border p-3">
                        <div className="grid gap-2 sm:grid-cols-[1fr_8rem_auto_auto] sm:items-end">
                          <Field id={`np-name-${i}`} label="Name">
                            <Input id={`np-name-${i}`} value={p.name} onChange={(e) => setParam(i, { name: e.target.value })} className="font-mono" />
                          </Field>
                          <Field id={`np-type-${i}`} label="Type">
                            <Select value={paramType(p)} onValueChange={(v) => setParam(i, { schema: { ...p.schema, type: v as SqlParamType } })}>
                              <SelectTrigger id={`np-type-${i}`} className="w-full">
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
                            <Checkbox id={`np-req-${i}`} checked={p.required} onCheckedChange={(v) => setParam(i, { required: v === true })} />
                            <Label htmlFor={`np-req-${i}`}>Required</Label>
                          </div>
                          <Button type="button" variant="ghost" size="icon" aria-label={`Remove ${p.name || "parameter"}`} onClick={() => set({ params: form.params.filter((_, j) => j !== i) })}>
                            <Trash2Icon />
                          </Button>
                        </div>
                        <Input value={p.description ?? ""} onChange={(e) => setParam(i, { description: e.target.value })} placeholder="What the agent should pass (optional)" aria-label={`Description of ${p.name}`} />
                        <Input
                          value={enumText(p)}
                          onChange={(e) => setParam(i, { schema: withEnum(p.schema, e.target.value, paramType(p)) })}
                          placeholder="Allowed values, comma-separated (optional), e.g. active, paused"
                          aria-label={`Allowed values of ${p.name}`}
                          className="font-mono text-xs"
                        />
                        {!p.required && <p className="text-xs text-muted-foreground">Left out by the agent, it's sent as null.</p>}
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
              </>
            ),
          },
        ]),
    {
      title: "Response",
      description: "Agents get {documents, truncated} (plus counts or totals). Reshape it here if they need less.",
      fields: ["response_shape"],
      content: (
        <>
          <Field id="n-shape" label="Response shape (jq)" error={err("response_shape")} hint="Leave empty to return the result as is. The project's size limit still applies.">
            <Textarea id="n-shape" value={form.responseShape} onChange={(e) => set({ responseShape: e.target.value })} placeholder=".documents | map({name, email})" className="min-h-16 font-mono text-xs" spellCheck={false} aria-invalid={!!err("response_shape")} />
          </Field>
        </>
      ),
    },
  ];

  const description = exploration ? (
    EXPLORATION_TEXT[form.operation]!.blurb
  ) : (
    <>
      One fixed operation. Agents fill in its <code>{"{{params}}"}</code>, which land as whole values in the body, never as
      operators or keys.
    </>
  );

  const banner = (
    <>
      {conns && conns.length === 0 && (
        <Alert>
          <AlertTitle>Add a NoSQL database first</AlertTitle>
          <AlertDescription>
            <span>
              NoSQL tools run on a MongoDB, Redis, DynamoDB or Elasticsearch connection.{" "}
              <a className="underline" href={href.project(projectId, "databases")}>
                Add one on the Databases tab
              </a>
              .
            </span>
          </AlertDescription>
        </Alert>
      )}
      {serverErr && (
        <Alert variant="destructive">
          <AlertTitle>{serverErr.code === "validation" ? "Fix these before saving" : "Couldn't save"}</AlertTitle>
          <AlertDescription>
            {serverErr.fields.length === 0 ? (
              serverErr.message
            ) : (
              <ul className="list-disc pl-4">
                {serverErr.fields.map((f, i) => (
                  <li key={i}>{f.message}</li>
                ))}
              </ul>
            )}
          </AlertDescription>
        </Alert>
      )}
    </>
  );

  if (!saved)
    return (
      <StepDialog
        title="New NoSQL tool"
        description={description}
        steps={steps}
        errorFields={[...Object.keys(localErr), ...(serverErr?.fields ?? []).map((f) => f.field)]}
        banner={banner}
        submitLabel="Create tool"
        busy={busy}
        onSubmit={() => save()}
        onClose={() => navigate(href.project(projectId))}
      />
    );

  return (
    <div className="space-y-6">
      <PageHeader back={{ href: href.project(projectId), label: "Tools" }} title={<span className="font-mono">{saved.name}</span>} description={description} />

      <div className="grid items-start gap-6 lg:grid-cols-[minmax(0,1fr)_24rem]">
        <form onSubmit={save} className="min-w-0 space-y-6" noValidate>
          {banner}

          {steps.map((s) => (
            <Section key={s.title} title={s.title} description={s.description ?? ""}>
              {s.content}
            </Section>
          ))}

          <div className="sticky bottom-0 -mx-1 flex flex-wrap items-center gap-3 border-t bg-background/95 px-1 py-3 backdrop-blur">
            <Button type="submit" disabled={busy || !dirty}>
              Save tool
            </Button>
            {dirty && <span className="text-sm text-muted-foreground">Unsaved changes</span>}
            <span className="flex-1" />
            {confirmDelete ? (
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
            )}
          </div>
        </form>

        <aside className="lg:sticky lg:top-20">
          <TestConsole projectId={projectId} toolId={saved.id} version={saved.updated_at} dirty={dirty} />
        </aside>
      </div>
    </div>
  );
}
