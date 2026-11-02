import { useEffect, useMemo, useState, type FormEvent } from "react";
import { toast } from "sonner";

import { api, ApiError } from "@/api";
import { PageHeader } from "@/components/PageHeader";
import { Field, Section } from "@/components/FormSection";
import { asProblems, checkError, StepDialog, type FormStep } from "@/components/StepDialog";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { ApprovalSwitch } from "@/components/ApprovalSwitch";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import { href, navigate } from "@/lib/router";
import {
  buildGraphqlInput,
  graphqlFormFromTool,
  graphqlVarIndex,
  newGraphqlForm,
  operationOf,
  syncGraphqlVars,
  type GraphqlForm,
} from "@/lib/graphqlForm";
import { describeAuth } from "@/pages/AuthTab";
import { TestConsole } from "@/pages/TestConsole";
import type { AuthConfig, Tool, FieldError } from "@/types";

/** Editor for GraphQL tools: one query or mutation against an endpoint. */
export function GraphqlToolEditor({ projectId, tool }: { projectId: string; tool: Tool | null }) {
  const [saved, setSaved] = useState<Tool | null>(tool);
  const [form, setForm] = useState<GraphqlForm>(tool ? graphqlFormFromTool(tool) : newGraphqlForm());
  const [auths, setAuths] = useState<AuthConfig[]>([]);
  const [serverErr, setServerErr] = useState<ApiError | null>(null);
  const [localErr, setLocalErr] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);

  useEffect(() => {
    api.listAuth(projectId).then(setAuths, () => {});
  }, [projectId]);

  const built = useMemo(() => buildGraphqlInput(form), [form]);
  const dirty = useMemo(() => {
    if (!saved || !built.ok) return !saved;
    const { name, description, enabled, tags, response_shape, require_approval, kind } = saved;
    return JSON.stringify(built.input) !== JSON.stringify({ name, description, enabled, tags, response_shape, require_approval, kind });
  }, [saved, built]);

  const set = (patch: Partial<GraphqlForm>) => setForm({ ...form, ...patch });
  const err = (field: string) => localErr[field] ?? serverErr?.fields.find((f) => f.field === field)?.message;
  const operation = operationOf(form.document);
  // Shown live; saved with the document on blur and on save.
  const vars = syncGraphqlVars(form.document, form.variables);
  const varErrors = (i: number) =>
    (serverErr?.fields ?? []).filter((f) => graphqlVarIndex(f.field) === i).map((f) => f.message);
  const setDescription = (name: string, description: string) =>
    set({ variables: vars.map((v) => (v.name === name ? { ...v, description } : v)) });

  async function save(e?: FormEvent) {
    e?.preventDefault();
    const result = buildGraphqlInput(form);
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
      setForm(graphqlFormFromTool(t));
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

  /** For Next: what the form or the server would refuse now. */
  async function check(): Promise<FieldError[]> {
    const result = buildGraphqlInput(form);
    return result.ok ? api.checkTool(projectId, result.input) : asProblems(result.fieldErrors);
  }

  const steps: FormStep[] = [
    {
      title: "What the agent sees",
      description: "The name and description are how an agent decides when to use this tool.",
      fields: ["name", "description", "tags", "enabled", "require_approval"],
      content: (
        <>
          <Field id="g-name" label="Name" error={err("name")} hint="Letters, digits, _ and -. Up to 64 characters.">
            <Input id="g-name" value={form.name} onChange={(e) => set({ name: e.target.value })} className="font-mono" placeholder="get_user" aria-invalid={!!err("name")} />
          </Field>
          <Field id="g-desc" label="Description" error={err("description")}>
            <Textarea id="g-desc" value={form.description} onChange={(e) => set({ description: e.target.value })} placeholder="A user's profile by login: name, bio and public repositories." className="min-h-20" />
          </Field>
          <Field id="g-tags" label="Tags" error={err("tags")} hint="Comma-separated. Project filters and tool search use them.">
            <Input id="g-tags" value={form.tagsText} onChange={(e) => set({ tagsText: e.target.value })} placeholder="users" />
          </Field>
          <div className="flex items-center gap-2">
            <Switch id="g-enabled" checked={form.enabled} onCheckedChange={(v) => set({ enabled: v })} />
            <Label htmlFor="g-enabled">Visible to agents</Label>
          </div>
          <ApprovalSwitch id="g-approval" checked={form.requireApproval ?? operation === "mutation"} onChange={(v) => set({ requireApproval: v })} />
        </>
      ),
    },
    {
      title: "Operation",
      description: "Sent as a POST with {query, operationName, variables}, like any GraphQL client.",
      fields: ["kind.endpoint", "kind.document", "kind.auth_ref", "kind.timeout_ms", "kind.operation", "kind.operation_name"],
      content: (
        <>
          <Field id="g-endpoint" label="Endpoint" error={err("kind.endpoint")}>
            <Input id="g-endpoint" value={form.endpoint} onChange={(e) => set({ endpoint: e.target.value })} className="font-mono" placeholder="https://api.example.com/graphql" aria-invalid={!!err("kind.endpoint")} />
          </Field>
          <Field
            id="g-doc"
            label="Document"
            error={err("kind.document")}
            hint="One query or mutation. Declare each value the agent passes as a $variable."
          >
            <Textarea
              id="g-doc"
              value={form.document}
              onChange={(e) => set({ document: e.target.value })}
              onBlur={() => set({ variables: syncGraphqlVars(form.document, form.variables) })}
              className="min-h-48 font-mono text-xs"
              spellCheck={false}
              placeholder={"query User($login: String!) {\n  user(login: $login) {\n    name\n    bio\n  }\n}"}
              aria-invalid={!!err("kind.document")}
            />
          </Field>
          {operation === "mutation" ? (
            <Alert>
              <AlertDescription>
                This is a mutation, so agents see it as a tool that changes data, and a read-only project hides it.
              </AlertDescription>
            </Alert>
          ) : null}
          <div className="grid gap-3 sm:grid-cols-[1fr_10rem]">
            <Field id="g-auth" label="Auth" error={err("kind.auth_ref")}>
              <Select value={form.auth_ref ?? "none"} onValueChange={(v) => set({ auth_ref: v === "none" ? null : v })}>
                <SelectTrigger id="g-auth" className="w-full">
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
            <Field id="g-timeout" label="Timeout (ms)" error={err("kind.timeout_ms")}>
              <Input id="g-timeout" value={form.timeout} onChange={(e) => set({ timeout: e.target.value })} inputMode="numeric" className="font-mono" aria-invalid={!!err("kind.timeout_ms")} />
            </Field>
          </div>
          {auths.length === 0 && (
            <p className="text-xs text-muted-foreground">
              Need a key or token? <a className="underline" href={href.project(projectId, "auth")}>Add an auth config</a> first.
            </p>
          )}
        </>
      ),
    },
    {
      title: "Variables",
      description: "Each $variable in the document is an argument the agent fills in. Non-null types (ending in !) are required.",
      fields: ["kind.variables"],
      content: (
        <>
          {vars.length === 0 ? (
            <p className="text-sm text-muted-foreground">No variables. Add them to the document, e.g. ($id: ID!).</p>
          ) : (
            <div className="grid gap-3">
              {vars.map((v, i) => (
                <div key={v.name} className="grid gap-2 rounded-md border p-3">
                  <div className="flex flex-wrap items-baseline gap-2 font-mono text-sm">
                    <span className="font-medium">{v.name}</span>
                    <span className="text-muted-foreground">{v.graphql_type}</span>
                    {v.schema && <span className="font-sans text-xs text-muted-foreground">· schema from the import</span>}
                  </div>
                  <Input value={v.description} onChange={(e) => setDescription(v.name, e.target.value)} placeholder="What the agent should pass (optional)" aria-label={`Description of ${v.name}`} />
                  {varErrors(i).map((m) => (
                    <p key={m} className="text-xs text-destructive">
                      {m}
                    </p>
                  ))}
                </div>
              ))}
            </div>
          )}
        </>
      ),
    },
    {
      title: "Response",
      description: "Agents get the data object. GraphQL errors come back as a failed call listing each message.",
      fields: ["response_shape"],
      content: (
        <>
          <Field id="g-shape" label="Response shape (jq)" error={err("response_shape")} hint="Leave empty to return the data as is. The project's size limit still applies.">
            <Textarea id="g-shape" value={form.responseShape} onChange={(e) => set({ responseShape: e.target.value })} placeholder=".user | {name, bio}" className="min-h-16 font-mono text-xs" spellCheck={false} aria-invalid={!!err("response_shape")} />
          </Field>
        </>
      ),
    },
  ];

  const description = (
    <>
      One fixed query or mutation. Agents fill in its <code>$variables</code>, which are sent separately from the document,
      never pasted into it.
    </>
  );

  const banner = (
    <>
      {serverErr && serverErr.code !== "check" && (
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
        title="New GraphQL tool"
        description={description}
        steps={steps}
        errorFields={[...Object.keys(localErr), ...(serverErr?.fields ?? []).map((f) => f.field)]}
        banner={banner}
        submitLabel="Create tool"
        busy={busy}
        onSubmit={() => save()}
        onClose={() => navigate(href.project(projectId))}
        check={check}
        showErrors={(found) => {
          setLocalErr({});
          setServerErr(checkError(found));
        }}
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
