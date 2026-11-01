import { useEffect, useMemo, useState, type FormEvent } from "react";
import { ArrowDownIcon, ArrowUpIcon, PlusIcon, Trash2Icon } from "lucide-react";
import { toast } from "sonner";

import { api, ApiError } from "@/api";
import { PageHeader } from "@/components/PageHeader";
import { Field, Section } from "@/components/FormSection";
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
import { paramType } from "@/lib/sqlForm";
import { splitTags } from "@/lib/toolForm";
import { TestConsole } from "@/pages/TestConsole";
import { SQL_PARAM_TYPES, type ChainStep, type SqlParam, type SqlParamType, type Tool, type ToolInput } from "@/types";

interface ChainForm {
  name: string;
  description: string;
  enabled: boolean;
  tagsText: string;
  params: SqlParam[];
  steps: ChainStep[];
  output: string;
  responseShape: string;
  requireApproval: boolean;
}

function formFromTool(t: Tool | null): ChainForm {
  if (!t || t.kind.type !== "chain")
    return {
      name: "",
      description: "",
      enabled: true,
      tagsText: "",
      params: [],
      steps: [{ tool: "", args: "" }],
      output: "",
      responseShape: "",
      requireApproval: false,
    };
  return {
    name: t.name,
    description: t.description,
    enabled: t.enabled,
    tagsText: t.tags.join(", "),
    params: t.kind.params,
    steps: t.kind.steps,
    output: t.kind.output ?? "",
    responseShape: t.response_shape ?? "",
    requireApproval: t.require_approval,
  };
}

function buildInput(f: ChainForm): ToolInput {
  return {
    name: f.name.trim(),
    description: f.description,
    enabled: f.enabled,
    tags: splitTags(f.tagsText),
    response_shape: f.responseShape.trim() || null,
    require_approval: f.requireApproval,
    kind: {
      type: "chain",
      params: f.params.map((p) => ({ ...p, name: p.name.trim(), description: p.description?.trim() || null })),
      steps: f.steps.map((s) => ({ tool: s.tool, args: s.args })),
      output: f.output.trim() || null,
    },
  };
}

/** Editor for chained tools: steps that call other tools in order. */
export function ChainToolEditor({ projectId, tool }: { projectId: string; tool: Tool | null }) {
  const [saved, setSaved] = useState<Tool | null>(tool);
  const [form, setForm] = useState<ChainForm>(formFromTool(tool));
  const [tools, setTools] = useState<Tool[]>([]);
  const [serverErr, setServerErr] = useState<ApiError | null>(null);
  const [busy, setBusy] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);

  useEffect(() => {
    api.listTools(projectId).then(setTools, () => setTools([]));
  }, [projectId]);

  const input = useMemo(() => buildInput(form), [form]);
  const dirty = useMemo(() => {
    if (!saved) return true;
    const { name, description, enabled, tags, response_shape, require_approval, kind } = saved;
    return JSON.stringify(input) !== JSON.stringify({ name, description, enabled, tags, response_shape, require_approval, kind });
  }, [saved, input]);

  const set = (patch: Partial<ChainForm>) => setForm({ ...form, ...patch });
  const setStep = (i: number, patch: Partial<ChainStep>) => set({ steps: form.steps.map((s, j) => (j === i ? { ...s, ...patch } : s)) });
  const setParam = (i: number, patch: Partial<SqlParam>) => set({ params: form.params.map((p, j) => (j === i ? { ...p, ...patch } : p)) });
  const move = (i: number, d: -1 | 1) => {
    const steps = [...form.steps];
    const [s] = steps.splice(i, 1);
    steps.splice(i + d, 0, s!);
    set({ steps });
  };
  const err = (field: string) => serverErr?.fields.find((f) => f.field === field)?.message;
  const others = tools.filter((t) => t.name !== (saved?.name ?? form.name));

  async function save(e: FormEvent) {
    e.preventDefault();
    setBusy(true);
    try {
      const t = saved ? await api.updateTool(projectId, saved.id, input) : await api.createTool(projectId, input);
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
      <PageHeader
        back={{ href: href.project(projectId), label: "Tools" }}
        title={saved ? <span className="font-mono">{saved.name}</span> : "New chain"}
        description="Calls other tools of this project in order, feeding each step from the inputs and earlier results. Agents see one tool."
      />
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
                    {serverErr.fields.map((f, i) => (
                      <li key={i}>{f.message}</li>
                    ))}
                  </ul>
                )}
              </AlertDescription>
            </Alert>
          )}

          <Section title="What the agent sees" description="The name and description are how an agent decides when to use this tool.">
            <Field id="c-name" label="Name" error={err("name")} hint="Letters, digits, _ and -. Up to 64 characters.">
              <Input id="c-name" value={form.name} onChange={(e) => set({ name: e.target.value })} className="font-mono" placeholder="user_overview" />
            </Field>
            <Field id="c-desc" label="Description" hint="Leave empty to describe the steps.">
              <Textarea id="c-desc" value={form.description} onChange={(e) => set({ description: e.target.value })} className="min-h-20" placeholder="A user's profile and their repositories in one call." />
            </Field>
            <Field id="c-tags" label="Tags" hint="Comma-separated.">
              <Input id="c-tags" value={form.tagsText} onChange={(e) => set({ tagsText: e.target.value })} />
            </Field>
            <div className="flex items-center gap-2">
              <Switch id="c-enabled" checked={form.enabled} onCheckedChange={(v) => set({ enabled: v })} />
              <Label htmlFor="c-enabled">Visible to agents</Label>
            </div>
            <ApprovalSwitch id="c-approval" checked={form.requireApproval} onChange={(v) => set({ requireApproval: v })} />
            <p className="-mt-2 text-xs text-muted-foreground">A step that needs approval makes the whole chain wait for one approval.</p>
          </Section>

          <Section title="Inputs" description="What the agent passes. Steps read them as .input.name.">
            {form.params.map((p, i) => (
              <div key={i} className="grid gap-2 rounded-md border p-3 sm:grid-cols-[1fr_8rem_auto_auto] sm:items-end">
                <Field id={`ci-name-${i}`} label="Name" error={err(`kind.params[${i}].name`)}>
                  <Input id={`ci-name-${i}`} value={p.name} onChange={(e) => setParam(i, { name: e.target.value })} className="font-mono" />
                </Field>
                <Field id={`ci-type-${i}`} label="Type">
                  <Select value={paramType(p)} onValueChange={(v) => setParam(i, { schema: { ...p.schema, type: v as SqlParamType } })}>
                    <SelectTrigger id={`ci-type-${i}`} className="w-full">
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
                  <Checkbox id={`ci-req-${i}`} checked={p.required} onCheckedChange={(v) => setParam(i, { required: v === true })} />
                  <Label htmlFor={`ci-req-${i}`}>Required</Label>
                </div>
                <Button type="button" variant="ghost" size="icon" aria-label={`Remove ${p.name || "input"}`} onClick={() => set({ params: form.params.filter((_, j) => j !== i) })}>
                  <Trash2Icon />
                </Button>
                <Input className="sm:col-span-4" value={p.description ?? ""} onChange={(e) => setParam(i, { description: e.target.value })} placeholder="What the agent should pass (optional)" aria-label={`Description of ${p.name}`} />
              </div>
            ))}
            <div>
              <Button type="button" variant="outline" size="sm" onClick={() => set({ params: [...form.params, { name: "", schema: { type: "string" }, required: true, description: null }] })}>
                <PlusIcon /> Add input
              </Button>
            </div>
          </Section>

          <Section title="Steps" description="Each step's arguments are a jq expression over {input, steps, last}: the inputs, every earlier result, and the previous one.">
            {form.steps.map((s, i) => (
              <div key={i} className="grid gap-2 rounded-md border p-3">
                <div className="flex items-end gap-2">
                  <span className="pb-2 font-mono text-sm text-muted-foreground">{i + 1}.</span>
                  <div className="flex-1">
                    <Field id={`cs-tool-${i}`} label="Tool" error={err(`kind.steps[${i}].tool`)}>
                      <Select value={s.tool || "none"} onValueChange={(v) => setStep(i, { tool: v === "none" ? "" : v })}>
                        <SelectTrigger id={`cs-tool-${i}`} className="w-full font-mono">
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          <SelectItem value="none" disabled>
                            Pick a tool
                          </SelectItem>
                          {others.map((t) => (
                            <SelectItem key={t.id} value={t.name} className="font-mono">
                              {t.name}
                            </SelectItem>
                          ))}
                        </SelectContent>
                      </Select>
                    </Field>
                  </div>
                  <Button type="button" variant="ghost" size="icon" aria-label="Move up" disabled={i === 0} onClick={() => move(i, -1)}>
                    <ArrowUpIcon />
                  </Button>
                  <Button type="button" variant="ghost" size="icon" aria-label="Move down" disabled={i === form.steps.length - 1} onClick={() => move(i, 1)}>
                    <ArrowDownIcon />
                  </Button>
                  <Button type="button" variant="ghost" size="icon" aria-label={`Remove step ${i + 1}`} disabled={form.steps.length === 1} onClick={() => set({ steps: form.steps.filter((_, j) => j !== i) })}>
                    <Trash2Icon />
                  </Button>
                </div>
                <Field id={`cs-args-${i}`} label="Arguments (jq)" error={err(`kind.steps[${i}].args`)}>
                  <Textarea
                    id={`cs-args-${i}`}
                    value={s.args}
                    onChange={(e) => setStep(i, { args: e.target.value })}
                    className="min-h-16 font-mono text-xs"
                    spellCheck={false}
                    placeholder={i === 0 ? "{login: .input.login}" : "{owner: .last.login}"}
                  />
                </Field>
              </div>
            ))}
            {err("kind.steps") && <p className="text-sm text-destructive">{err("kind.steps")}</p>}
            <div>
              <Button type="button" variant="outline" size="sm" disabled={form.steps.length >= 10} onClick={() => set({ steps: [...form.steps, { tool: "", args: "" }] })}>
                <PlusIcon /> Add step
              </Button>
            </div>
          </Section>

          <Section title="Result" description="What the agent gets. Without an output expression, it's the last step's result.">
            <Field id="c-output" label="Output (jq over {input, steps})" error={err("kind.output")}>
              <Textarea id="c-output" value={form.output} onChange={(e) => set({ output: e.target.value })} className="min-h-16 font-mono text-xs" spellCheck={false} placeholder="{user: .steps[0].login, repos: [.steps[1][].name]}" />
            </Field>
            <Field id="c-shape" label="Response shape (jq)" error={err("response_shape")} hint="Applied after the output, like for any tool.">
              <Textarea id="c-shape" value={form.responseShape} onChange={(e) => set({ responseShape: e.target.value })} className="min-h-12 font-mono text-xs" spellCheck={false} />
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
        <aside className="lg:sticky lg:top-20">
          <TestConsole projectId={projectId} toolId={saved?.id ?? null} version={saved?.updated_at ?? ""} dirty={dirty} />
        </aside>
      </div>
    </div>
  );
}
