import { useEffect, useMemo, useState, type FormEvent, type ReactNode } from "react";
import { ArrowLeftIcon } from "lucide-react";
import { toast } from "sonner";

import { api, ApiError } from "@/api";
import { RequestLine } from "@/components/RequestLine";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import { href, navigate } from "@/lib/router";
import { listLabels } from "@/lib/fields";
import { syncPathParams } from "@/lib/template";
import { buildToolInput, formFromTool, newForm, paramIndex, type ToolForm } from "@/lib/toolForm";
import { ParamsEditor, toRow, type ParamRow } from "@/pages/ParamsEditor";
import { TestConsole } from "@/pages/TestConsole";
import { describeAuth } from "@/pages/AuthTab";
import { METHODS, type AuthConfig, type HttpMethod, type Tool } from "@/types";

export function ToolEditor({ projectId, toolId }: { projectId: string; toolId: string | null }) {
  const [saved, setSaved] = useState<Tool | null>(null);
  const [form, setForm] = useState<ToolForm | null>(toolId ? null : newForm());
  const [rows, setRows] = useState<ParamRow[]>([]);
  const [auths, setAuths] = useState<AuthConfig[]>([]);
  const [serverErr, setServerErr] = useState<ApiError | null>(null);
  const [localErr, setLocalErr] = useState<Record<string, string>>({});
  const [loadErr, setLoadErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);

  useEffect(() => {
    api.listAuth(projectId).then(setAuths, () => {});
    if (!toolId) return;
    api.getTool(projectId, toolId).then(
      (t) => {
        setSaved(t);
        setForm(formFromTool(t));
        setRows(t.kind.params.map(toRow));
      },
      (e) => setLoadErr(e instanceof ApiError && e.status === 404 ? "This tool doesn't exist. It may have been deleted." : String(e)),
    );
  }, [projectId, toolId]);

  const built = useMemo(() => (form ? buildToolInput(form, rows) : null), [form, rows]);
  const dirty = useMemo(() => {
    if (!saved || !built?.ok) return !saved;
    const { name, description, enabled, response_shape, kind } = saved;
    return JSON.stringify(built.input) !== JSON.stringify({ name, description, enabled, response_shape, kind });
  }, [saved, built]);

  if (loadErr)
    return (
      <Alert variant="destructive">
        <AlertDescription>
          {loadErr} <a className="underline" href={href.project(projectId)}>Back to tools</a>
        </AlertDescription>
      </Alert>
    );
  if (!form) return <Skeleton className="h-64 w-full" />;

  const set = (patch: Partial<ToolForm>) => setForm({ ...form, ...patch });
  const err = (field: string) => localErr[field] ?? serverErr?.fields.find((f) => f.field === field)?.message;
  const rowServerErrors = (i: number) =>
    (serverErr?.fields ?? []).filter((f) => paramIndex(f.field) === i).map((f) => f.message);
  const generalErrors = (serverErr?.fields ?? []).filter(
    (f) => f.field === "kind.params" || f.field === "kind.auth_ref" || f.field === "response_shape",
  );

  async function save(e: FormEvent) {
    e.preventDefault();
    if (!form) return;
    const result = buildToolInput(form, rows);
    if (!result.ok) {
      setLocalErr(result.fieldErrors);
      setRows(rows.map((r, i) => ({ ...r, error: result.rowErrors[i] })));
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

  return (
    <div className="space-y-6">
      <a href={href.project(projectId)} className="inline-flex items-center gap-1 text-sm text-muted-foreground hover:text-foreground">
        <ArrowLeftIcon className="size-4" /> Tools
      </a>

      <header className="space-y-3">
        <h1 className="text-2xl font-semibold tracking-tight">{saved ? saved.name : "New tool"}</h1>
        <div className="overflow-x-auto rounded-lg border bg-card px-4 py-3">
          <RequestLine method={form.method} baseUrl={form.base_url} path={form.path_template} size="lg" />
        </div>
        {saved && saved.source.type !== "manual" && (
          <ImportedNote
            tool={saved}
            onReset={async () => {
              try {
                const t = await api.resetOverrides(projectId, saved.id);
                setSaved(t);
                toast.success("The next re-import will use the spec's values");
              } catch (e) {
                toast.error(e instanceof Error ? e.message : String(e));
              }
            }}
          />
        )}
      </header>

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
                    {(generalErrors.length ? generalErrors : serverErr.fields).map((f, i) => (
                      <li key={i}>{f.message}</li>
                    ))}
                  </ul>
                )}
              </AlertDescription>
            </Alert>
          )}

          <Section title="What the agent sees" description="The name and description are how an agent decides when to use this tool.">
            <Field id="t-name" label="Name" error={err("name")} hint="Letters, digits, _ and -. Up to 64 characters.">
              <Input id="t-name" value={form.name} onChange={(e) => set({ name: e.target.value })} className="font-mono" placeholder="get_user" aria-invalid={!!err("name")} />
            </Field>
            <Field id="t-desc" label="Description" error={err("description")}>
              <Textarea id="t-desc" value={form.description} onChange={(e) => set({ description: e.target.value })} placeholder="Fetch one user by id. Returns name, email and created date." className="min-h-20" />
            </Field>
            <div className="flex items-center gap-2">
              <Switch id="t-enabled" checked={form.enabled} onCheckedChange={(v) => set({ enabled: v })} />
              <Label htmlFor="t-enabled">Visible to agents</Label>
            </div>
          </Section>

          <Section title="Request" description="Use {name} in the path for values the agent fills in.">
            <div className="grid gap-3 sm:grid-cols-[8rem_1fr]">
              <Field id="t-method" label="Method">
                <Select value={form.method} onValueChange={(v) => set({ method: v as HttpMethod })}>
                  <SelectTrigger id="t-method" className="w-full font-mono">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {METHODS.map((m) => (
                      <SelectItem key={m} value={m} className="font-mono">
                        {m}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </Field>
              <Field id="t-base" label="Base URL" error={err("kind.base_url")}>
                <Input id="t-base" value={form.base_url} onChange={(e) => set({ base_url: e.target.value })} className="font-mono" placeholder="https://api.example.com/v1" aria-invalid={!!err("kind.base_url")} />
              </Field>
            </div>
            <Field id="t-path" label="Path" error={err("kind.path_template")} hint="Leaving this field adds a parameter for any new {placeholder}.">
              <Input
                id="t-path"
                value={form.path_template}
                onChange={(e) => set({ path_template: e.target.value })}
                onBlur={() => {
                  const synced = syncPathParams(form.path_template, rows.map((r) => r.def));
                  if (synced.length !== rows.length) setRows([...rows, ...synced.slice(rows.length).map(toRow)]);
                }}
                className="font-mono"
                placeholder="/users/{id}"
                aria-invalid={!!err("kind.path_template")}
              />
            </Field>
            <div className="grid gap-3 sm:grid-cols-[1fr_10rem]">
              <Field id="t-auth" label="Auth" error={err("kind.auth_ref")}>
                <Select value={form.auth_ref ?? "none"} onValueChange={(v) => set({ auth_ref: v === "none" ? null : v })}>
                  <SelectTrigger id="t-auth" className="w-full">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="none">No auth</SelectItem>
                    {auths.map((a) => (
                      <SelectItem key={a.id} value={a.id}>
                        {a.name}: {describeAuth(a.kind)}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </Field>
              <Field id="t-timeout" label="Timeout (ms)" error={err("kind.timeout_ms")}>
                <Input id="t-timeout" value={form.timeout} onChange={(e) => set({ timeout: e.target.value })} inputMode="numeric" className="font-mono" aria-invalid={!!err("kind.timeout_ms")} />
              </Field>
            </div>
            {auths.length === 0 && (
              <p className="text-xs text-muted-foreground">
                Need a key or token? <a className="underline" href={href.project(projectId, "auth")}>Add an auth config</a> first.
              </p>
            )}
          </Section>

          <Section title="Parameters" description="Each parameter is an argument the agent fills in, unless you give it a fixed value.">
            <ParamsEditor rows={rows} onChange={setRows} serverErrors={rowServerErrors} />
          </Section>

          <Section title="Body" description="Sent as JSON. Use body parameters for a few fields, or one schema for the whole body.">
            <Field id="t-bodymode" label="Body comes from">
              <Select value={form.bodyMode} onValueChange={(v) => set({ bodyMode: v as ToolForm["bodyMode"] })}>
                <SelectTrigger id="t-bodymode" className="w-full sm:w-80">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="fields">Parameters set to go in Body</SelectItem>
                  <SelectItem value="schema">One JSON schema for the whole body</SelectItem>
                </SelectContent>
              </Select>
            </Field>
            {form.bodyMode === "schema" && (
              <Field id="t-bodyschema" label="Body JSON Schema" error={err("kind.body_schema")} hint="The agent passes the whole body as one argument named body.">
                <Textarea id="t-bodyschema" value={form.bodySchemaText} onChange={(e) => set({ bodySchemaText: e.target.value })} className="min-h-40 font-mono text-xs" spellCheck={false} aria-invalid={!!err("kind.body_schema")} />
              </Field>
            )}
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

function Section({ title, description, children }: { title: string; description: string; children: ReactNode }) {
  return (
    <Card>
      <CardHeader>
        <CardTitle>{title}</CardTitle>
        <CardDescription>{description}</CardDescription>
      </CardHeader>
      <CardContent className="grid gap-4">{children}</CardContent>
    </Card>
  );
}

function Field({ id, label, error, hint, children }: { id: string; label: string; error?: string; hint?: string; children: ReactNode }) {
  return (
    <div className="grid gap-1.5">
      <Label htmlFor={id}>{label}</Label>
      {children}
      {error ? <p className="text-xs text-destructive">{error}</p> : hint && <p className="text-xs text-muted-foreground">{hint}</p>}
    </div>
  );
}

function ImportedNote({ tool, onReset }: { tool: Tool; onReset: () => Promise<void> }) {
  const kept = tool.overrides.fields;
  const from = tool.source.type === "swagger" ? "a Swagger spec" : "an OpenAPI spec";
  return (
    <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-sm text-muted-foreground">
      <span>
        Imported from {from}.{" "}
        {kept.length === 0
          ? "Re-importing updates it; anything you change here will be kept."
          : `Re-importing keeps your ${listLabels(kept)}.`}
      </span>
      {kept.length > 0 && (
        <Button type="button" variant="link" size="sm" className="h-auto p-0" onClick={() => void onReset()}>
          Use the spec's values next time
        </Button>
      )}
    </div>
  );
}
