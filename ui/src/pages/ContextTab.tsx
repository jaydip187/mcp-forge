import { useCallback, useEffect, useState, type FormEvent } from "react";
import { FileTextIcon, MessageSquareTextIcon, PlusIcon, Trash2Icon } from "lucide-react";
import { toast } from "sonner";

import { api, ApiError } from "@/api";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import { useCanWrite } from "@/lib/permissions";
import { nosqlPlaceholders } from "@/lib/nosqlForm";
import {
  RESOURCE_MIME_TYPES,
  type McpResourceView,
  type Project,
  type PromptArg,
  type PromptDef,
  type PromptInput,
  type ResourceDef,
  type ResourceInput,
} from "@/types";

const fieldMsg = (err: unknown, prefix: string) =>
  err instanceof ApiError ? err.fields.find((f) => f.field === prefix || f.field.startsWith(`${prefix}[`))?.message : undefined;

/** MCP resources and prompts: what agents can read and reuse besides tools. */
export function ContextTab({ project, onSaved }: { project: Project; onSaved: (p: Project) => void }) {
  const canWrite = useCanWrite(project.id);
  const [resources, setResources] = useState<ResourceDef[] | null>(null);
  const [prompts, setPrompts] = useState<PromptDef[] | null>(null);
  const [visible, setVisible] = useState<McpResourceView[]>([]);
  const [editingRes, setEditingRes] = useState<ResourceDef | "new" | null>(null);
  const [editingPrompt, setEditingPrompt] = useState<PromptDef | "new" | null>(null);
  const [preview, setPreview] = useState<{ uri: string; text: string } | null>(null);
  const [trying, setTrying] = useState<string | null>(null);

  const load = useCallback(async () => {
    const [r, p, v] = await Promise.all([
      api.listResources(project.id),
      api.listPrompts(project.id),
      api.mcpResources(project.id),
    ]);
    setResources(r);
    setPrompts(p);
    setVisible(v);
  }, [project.id]);
  useEffect(() => {
    load().catch((e) => toast.error(String(e)));
  }, [load]);

  async function toggleSpecs(on: boolean) {
    try {
      onSaved(await api.updateSettings(project.id, { ...project.settings, share_specs: on }));
      await load();
    } catch (e) {
      toast.error(e instanceof Error ? e.message : String(e));
    }
  }

  async function show(uri: string) {
    try {
      const r = await api.readMcpResource(project.id, uri);
      setPreview({ uri, text: r.text });
    } catch (e) {
      toast.error(e instanceof Error ? e.message : String(e));
    }
  }

  if (!resources || !prompts) return <Skeleton className="h-40 w-full" />;

  return (
    <div className="grid items-start gap-6 lg:grid-cols-2">
      <Card>
        <CardHeader>
          <CardTitle>Resources</CardTitle>
          <CardDescription>
            Documents agents can read: style guides, glossaries, runbooks. Clients show them as attachable context.
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          {resources.length === 0 && editingRes === null && (
            <p className="text-sm text-muted-foreground">No resources yet.</p>
          )}
          <ul className="divide-y rounded-md border empty:hidden">
            {resources.map((r) => (
              <li key={r.id} className="flex items-center gap-3 px-3 py-2">
                <FileTextIcon className="size-4 shrink-0 text-muted-foreground" aria-hidden />
                <div className="min-w-0 flex-1">
                  <p className="font-mono text-sm">{r.name}</p>
                  <p className="truncate text-xs text-muted-foreground">{r.description || r.mime_type}</p>
                </div>
                {canWrite && (
                  <Button size="sm" variant="outline" onClick={() => setEditingRes(r)}>
                    Edit
                  </Button>
                )}
              </li>
            ))}
          </ul>
          {editingRes !== null ? (
            <ResourceForm
              key={editingRes === "new" ? "new" : editingRes.id}
              projectId={project.id}
              existing={editingRes === "new" ? null : editingRes}
              onDone={async () => {
                setEditingRes(null);
                await load();
              }}
            />
          ) : (
            canWrite && (
              <Button size="sm" variant="outline" onClick={() => setEditingRes("new")}>
                <PlusIcon /> Add resource
              </Button>
            )
          )}
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Prompts</CardTitle>
          <CardDescription>
            Reusable instructions with arguments. In Claude they appear as slash commands; {"{{argument}}"} marks where a value
            goes.
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          {prompts.length === 0 && editingPrompt === null && <p className="text-sm text-muted-foreground">No prompts yet.</p>}
          <ul className="divide-y rounded-md border empty:hidden">
            {prompts.map((p) => (
              <li key={p.id} className="space-y-2 px-3 py-2">
                <div className="flex items-center gap-3">
                  <MessageSquareTextIcon className="size-4 shrink-0 text-muted-foreground" aria-hidden />
                  <div className="min-w-0 flex-1">
                    <p className="font-mono text-sm">{p.name}</p>
                    <p className="truncate text-xs text-muted-foreground">
                      {p.description || p.template}
                      {p.arguments.length > 0 && ` · ${p.arguments.map((a) => a.name).join(", ")}`}
                    </p>
                  </div>
                  <Button size="sm" variant="ghost" onClick={() => setTrying(trying === p.id ? null : p.id)} aria-expanded={trying === p.id}>
                    Try
                  </Button>
                  {canWrite && (
                    <Button size="sm" variant="outline" onClick={() => setEditingPrompt(p)}>
                      Edit
                    </Button>
                  )}
                </div>
                {trying === p.id && <PromptPreview projectId={project.id} prompt={p} />}
              </li>
            ))}
          </ul>
          {editingPrompt !== null ? (
            <PromptForm
              key={editingPrompt === "new" ? "new" : editingPrompt.id}
              projectId={project.id}
              existing={editingPrompt === "new" ? null : editingPrompt}
              onDone={async () => {
                setEditingPrompt(null);
                await load();
              }}
            />
          ) : (
            canWrite && (
              <Button size="sm" variant="outline" onClick={() => setEditingPrompt("new")}>
                <PlusIcon /> Add prompt
              </Button>
            )
          )}
        </CardContent>
      </Card>

      <Card className="lg:col-span-2">
        <CardHeader>
          <CardTitle>What agents can read</CardTitle>
          <CardDescription>
            Your resources, plus the schema of every database whose “Describe” agent tool is on (allowed tables and masks
            apply), plus imported specs when shared.
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="flex items-start gap-2">
            <Switch id="ctx-specs" checked={project.settings.share_specs} onCheckedChange={(v) => void toggleSpecs(v)} disabled={!canWrite} className="mt-0.5" />
            <Label htmlFor="ctx-specs" className="grid gap-0.5 font-normal">
              <span className="font-medium">Share imported specs</span>
              <span className="text-xs text-muted-foreground">Offer the OpenAPI, Swagger or GraphQL documents you imported as resources.</span>
            </Label>
          </div>
          {visible.length === 0 ? (
            <p className="text-sm text-muted-foreground">Nothing yet.</p>
          ) : (
            <ul className="divide-y rounded-md border">
              {visible.map((r) => (
                <li key={r.uri} className="flex items-center gap-3 px-3 py-2">
                  <div className="min-w-0 flex-1">
                    <p className="truncate font-mono text-sm">{r.uri}</p>
                    {r.description && <p className="truncate text-xs text-muted-foreground">{r.description}</p>}
                  </div>
                  <Button size="sm" variant="ghost" onClick={() => void show(r.uri)}>
                    Preview
                  </Button>
                </li>
              ))}
            </ul>
          )}
          {preview && (
            <div className="space-y-2">
              <p className="font-mono text-xs text-muted-foreground">{preview.uri}</p>
              <pre className="max-h-96 overflow-auto rounded-md border bg-muted/60 p-3 font-mono text-xs whitespace-pre-wrap">{preview.text}</pre>
            </div>
          )}
        </CardContent>
      </Card>
    </div>
  );
}

function ResourceForm({ projectId, existing, onDone }: { projectId: string; existing: ResourceDef | null; onDone: () => Promise<void> }) {
  const [draft, setDraft] = useState<ResourceInput>(
    existing
      ? { name: existing.name, description: existing.description, mime_type: existing.mime_type, content: existing.content }
      : { name: "", description: "", mime_type: "text/markdown", content: "" },
  );
  const [err, setErr] = useState<unknown>(null);
  const [confirm, setConfirm] = useState(false);

  async function submit(e: FormEvent) {
    e.preventDefault();
    try {
      if (existing) await api.updateResource(projectId, existing.id, draft);
      else await api.createResource(projectId, draft);
      toast.success(existing ? `Saved ${draft.name}` : `Added ${draft.name}`);
      await onDone();
    } catch (e) {
      setErr(e);
      if (!(e instanceof ApiError && e.fields.length)) toast.error(e instanceof Error ? e.message : String(e));
    }
  }

  async function remove() {
    if (!existing) return;
    await api.deleteResource(projectId, existing.id);
    toast.success(`Deleted ${existing.name}`);
    await onDone();
  }

  return (
    <form onSubmit={submit} className="grid gap-3 rounded-md border bg-muted/30 p-3">
      <div className="grid gap-3 sm:grid-cols-[1fr_12rem]">
        <div className="grid gap-1.5">
          <Label htmlFor="r-name">Name</Label>
          <Input id="r-name" value={draft.name} onChange={(e) => setDraft({ ...draft, name: e.target.value })} className="font-mono" placeholder="style_guide" aria-invalid={!!fieldMsg(err, "name")} />
          {fieldMsg(err, "name") && <p className="text-xs text-destructive">{fieldMsg(err, "name")}</p>}
        </div>
        <div className="grid gap-1.5">
          <Label htmlFor="r-mime">Format</Label>
          <Select value={draft.mime_type} onValueChange={(v) => setDraft({ ...draft, mime_type: v })}>
            <SelectTrigger id="r-mime" className="w-full">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {RESOURCE_MIME_TYPES.map((m) => (
                <SelectItem key={m} value={m}>
                  {m}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
      </div>
      <div className="grid gap-1.5">
        <Label htmlFor="r-desc">Description</Label>
        <Input id="r-desc" value={draft.description} onChange={(e) => setDraft({ ...draft, description: e.target.value })} placeholder="How we name things in support replies" />
      </div>
      <div className="grid gap-1.5">
        <Label htmlFor="r-content">Content</Label>
        <Textarea id="r-content" value={draft.content} onChange={(e) => setDraft({ ...draft, content: e.target.value })} className="min-h-40 font-mono text-xs" spellCheck={false} aria-invalid={!!fieldMsg(err, "content")} />
        {fieldMsg(err, "content") && <p className="text-xs text-destructive">{fieldMsg(err, "content")}</p>}
      </div>
      <div className="flex flex-wrap gap-2">
        <Button type="submit" size="sm">
          {existing ? "Save resource" : "Add resource"}
        </Button>
        <Button type="button" size="sm" variant="outline" onClick={() => void onDone()}>
          Cancel
        </Button>
        <span className="flex-1" />
        {existing &&
          (confirm ? (
            <Button type="button" size="sm" variant="destructive" onClick={() => void remove()}>
              Delete {existing.name}
            </Button>
          ) : (
            <Button type="button" size="sm" variant="ghost" className="text-destructive" onClick={() => setConfirm(true)}>
              <Trash2Icon /> Delete
            </Button>
          ))}
      </div>
    </form>
  );
}

/** Fills a prompt in with sample arguments: the text an agent's app would send. */
function PromptPreview({ projectId, prompt }: { projectId: string; prompt: PromptDef }) {
  const [args, setArgs] = useState<Record<string, string>>({});
  const [out, setOut] = useState<{ text?: string; error?: string } | null>(null);

  async function fill(e: FormEvent) {
    e.preventDefault();
    try {
      const filled = Object.fromEntries(Object.entries(args).filter(([, v]) => v !== ""));
      setOut(await api.renderPrompt(projectId, prompt.id, filled));
    } catch (e) {
      setOut({ error: e instanceof Error ? e.message : String(e) });
    }
  }

  return (
    <form onSubmit={fill} className="grid gap-2 rounded-md border bg-muted/30 p-3">
      {prompt.arguments.map((a) => (
        <div key={a.name} className="grid gap-1">
          <Label htmlFor={`pp-${prompt.id}-${a.name}`} className="font-mono text-xs">
            {a.name}
            {a.required && <span className="text-destructive"> *</span>}
          </Label>
          <Input
            id={`pp-${prompt.id}-${a.name}`}
            value={args[a.name] ?? ""}
            onChange={(e) => setArgs({ ...args, [a.name]: e.target.value })}
            placeholder={a.description || undefined}
          />
        </div>
      ))}
      <div>
        <Button type="submit" size="sm" variant="outline">
          Fill in
        </Button>
      </div>
      {out?.error && <p className="text-xs text-destructive">{out.error}</p>}
      {out?.text !== undefined && (
        <pre className="max-h-64 overflow-auto rounded border bg-background p-2 text-xs whitespace-pre-wrap">{out.text}</pre>
      )}
    </form>
  );
}

function PromptForm({ projectId, existing, onDone }: { projectId: string; existing: PromptDef | null; onDone: () => Promise<void> }) {
  const [draft, setDraft] = useState<PromptInput>(
    existing
      ? { name: existing.name, description: existing.description, arguments: existing.arguments, template: existing.template }
      : { name: "", description: "", arguments: [], template: "" },
  );
  const [err, setErr] = useState<unknown>(null);
  const [confirm, setConfirm] = useState(false);

  /** The draft with an argument for every new {{name}} in the template. */
  function withTemplateArgs(): PromptInput {
    const have = new Set(draft.arguments.map((a) => a.name));
    const added: PromptArg[] = nosqlPlaceholders(draft.template)
      .filter((n) => !have.has(n))
      .map((name) => ({ name, description: "", required: true }));
    return added.length ? { ...draft, arguments: [...draft.arguments, ...added] } : draft;
  }
  function syncArgs() {
    const next = withTemplateArgs();
    if (next !== draft) {
      setDraft(next);
      setErr(null);
    }
  }
  const setArg = (i: number, patch: Partial<PromptArg>) =>
    setDraft({ ...draft, arguments: draft.arguments.map((a, j) => (j === i ? { ...a, ...patch } : a)) });

  async function submit(e: FormEvent) {
    e.preventDefault();
    const body = withTemplateArgs();
    setDraft(body);
    try {
      if (existing) await api.updatePrompt(projectId, existing.id, body);
      else await api.createPrompt(projectId, body);
      toast.success(existing ? `Saved ${body.name}` : `Added ${body.name}`);
      await onDone();
    } catch (e) {
      setErr(e);
      if (!(e instanceof ApiError && e.fields.length)) toast.error(e instanceof Error ? e.message : String(e));
    }
  }

  async function remove() {
    if (!existing) return;
    await api.deletePrompt(projectId, existing.id);
    toast.success(`Deleted ${existing.name}`);
    await onDone();
  }

  const argErrors = err instanceof ApiError ? err.fields.filter((f) => f.field.startsWith("arguments")).map((f) => f.message) : [];

  return (
    <form onSubmit={submit} className="grid gap-3 rounded-md border bg-muted/30 p-3">
      <div className="grid gap-1.5">
        <Label htmlFor="p-name">Name</Label>
        <Input id="p-name" value={draft.name} onChange={(e) => setDraft({ ...draft, name: e.target.value })} className="font-mono" placeholder="triage_ticket" aria-invalid={!!fieldMsg(err, "name")} />
        {fieldMsg(err, "name") && <p className="text-xs text-destructive">{fieldMsg(err, "name")}</p>}
      </div>
      <div className="grid gap-1.5">
        <Label htmlFor="p-desc">Description</Label>
        <Input id="p-desc" value={draft.description} onChange={(e) => setDraft({ ...draft, description: e.target.value })} placeholder="Look up a ticket and draft a reply" />
      </div>
      <div className="grid gap-1.5">
        <Label htmlFor="p-template">Template</Label>
        <Textarea
          id="p-template"
          value={draft.template}
          onChange={(e) => setDraft({ ...draft, template: e.target.value })}
          onBlur={syncArgs}
          className="min-h-32 font-mono text-xs"
          placeholder={"Look up ticket {{ticket}} with the support tools, then draft a reply in a {{tone}} tone."}
          aria-invalid={!!fieldMsg(err, "template")}
        />
        {fieldMsg(err, "template") && <p className="text-xs text-destructive">{fieldMsg(err, "template")}</p>}
      </div>
      {draft.arguments.length > 0 && (
        <div className="grid gap-2">
          <p className="text-sm font-medium">Arguments</p>
          {draft.arguments.map((a, i) => (
            <div key={i} className="grid gap-2 sm:grid-cols-[10rem_1fr_auto_auto] sm:items-center">
              <Input value={a.name} onChange={(e) => setArg(i, { name: e.target.value })} className="font-mono" aria-label="Argument name" />
              <Input value={a.description} onChange={(e) => setArg(i, { description: e.target.value })} placeholder="What to pass (optional)" aria-label={`Description of ${a.name}`} />
              <div className="flex items-center gap-1.5">
                <Checkbox id={`pa-req-${i}`} checked={a.required} onCheckedChange={(v) => setArg(i, { required: v === true })} />
                <Label htmlFor={`pa-req-${i}`} className="font-normal">
                  Required
                </Label>
              </div>
              <Button type="button" variant="ghost" size="icon" aria-label={`Remove ${a.name}`} onClick={() => setDraft({ ...draft, arguments: draft.arguments.filter((_, j) => j !== i) })}>
                <Trash2Icon />
              </Button>
            </div>
          ))}
        </div>
      )}
      {argErrors.length > 0 && (
        <Alert variant="destructive">
          <AlertDescription>
            {argErrors.map((m, i) => (
              <p key={i}>{m}</p>
            ))}
          </AlertDescription>
        </Alert>
      )}
      <div className="flex flex-wrap gap-2">
        <Button type="submit" size="sm">
          {existing ? "Save prompt" : "Add prompt"}
        </Button>
        <Button type="button" size="sm" variant="outline" onClick={() => void onDone()}>
          Cancel
        </Button>
        <span className="flex-1" />
        {existing &&
          (confirm ? (
            <Button type="button" size="sm" variant="destructive" onClick={() => void remove()}>
              Delete {existing.name}
            </Button>
          ) : (
            <Button type="button" size="sm" variant="ghost" className="text-destructive" onClick={() => setConfirm(true)}>
              <Trash2Icon /> Delete
            </Button>
          ))}
      </div>
    </form>
  );
}
