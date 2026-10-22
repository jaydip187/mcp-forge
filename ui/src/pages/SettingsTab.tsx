import { useState, type ChangeEvent, type FormEvent } from "react";
import { DownloadIcon, UploadIcon } from "lucide-react";
import { toast } from "sonner";

import { api, ApiError } from "@/api";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import { splitTags } from "@/lib/toolForm";
import { fieldError, useProjects } from "@/lib/projects";
import { href, navigate } from "@/lib/router";
import type { Project, ProjectSettings } from "@/types";

export function SettingsTab({ project, onSaved }: { project: Project; onSaved: (p: Project) => void }) {
  const { reload } = useProjects();
  const [name, setName] = useState(project.name);
  const [slug, setSlug] = useState(project.slug);
  const [err, setErr] = useState<unknown>(null);
  const [confirming, setConfirming] = useState(false);

  async function save(e: FormEvent) {
    e.preventDefault();
    try {
      const p = await api.updateProject(project.id, { name, slug });
      onSaved(p);
      setErr(null);
      await reload();
      toast.success("Project saved");
    } catch (e) {
      setErr(e);
      if (!(e instanceof ApiError && e.fields.length)) toast.error(e instanceof Error ? e.message : String(e));
    }
  }

  async function remove() {
    try {
      await api.deleteProject(project.id);
      await reload();
      toast.success(`Deleted ${project.name}`);
      navigate(href.home());
    } catch (e) {
      toast.error(e instanceof Error ? e.message : String(e));
    }
  }

  return (
    <div className="grid max-w-2xl gap-6">
      <Card>
        <CardHeader>
          <CardTitle>Name and URL</CardTitle>
          <CardDescription>Changing the slug changes the MCP URL. Connected clients will need the new one.</CardDescription>
        </CardHeader>
        <CardContent>
          <form onSubmit={save} className="grid gap-4">
            <div className="grid gap-1.5">
              <Label htmlFor="s-name">Name</Label>
              <Input id="s-name" value={name} onChange={(e) => setName(e.target.value)} aria-invalid={!!fieldError(err, "name")} />
              {fieldError(err, "name") && <p className="text-xs text-destructive">{fieldError(err, "name")}</p>}
            </div>
            <div className="grid gap-1.5">
              <Label htmlFor="s-slug">Slug</Label>
              <Input id="s-slug" value={slug} onChange={(e) => setSlug(e.target.value)} className="font-mono" aria-invalid={!!fieldError(err, "slug")} />
              {fieldError(err, "slug") && <p className="text-xs text-destructive">{fieldError(err, "slug")}</p>}
            </div>
            <Button type="submit" className="w-fit" disabled={name === project.name && slug === project.slug}>
              Save project
            </Button>
          </form>
        </CardContent>
      </Card>

      <CurationCard project={project} onSaved={onSaved} />

      <BundleCard project={project} onImported={reload} />

      <Card className="border-destructive/40">
        <CardHeader>
          <CardTitle>Delete project</CardTitle>
          <CardDescription>Deletes the project with all its tools and auth configs. This can't be undone.</CardDescription>
        </CardHeader>
        <CardContent className="flex gap-2">
          {confirming ? (
            <>
              <Button variant="destructive" onClick={() => void remove()}>
                Delete {project.name} for good
              </Button>
              <Button variant="outline" onClick={() => setConfirming(false)}>
                Keep it
              </Button>
            </>
          ) : (
            <Button variant="outline" className="text-destructive" onClick={() => setConfirming(true)}>
              Delete project
            </Button>
          )}
        </CardContent>
      </Card>
    </div>
  );
}

type Policy = "fail" | "skip" | "replace";

function BundleCard({ project, onImported }: { project: Project; onImported: () => Promise<void> }) {
  const [policy, setPolicy] = useState<Policy>("fail");
  const [busy, setBusy] = useState(false);

  async function download() {
    try {
      const yaml = await api.exportBundle(project.id);
      const url = URL.createObjectURL(new Blob([yaml], { type: "application/yaml" }));
      const a = document.createElement("a");
      a.href = url;
      a.download = `${project.slug}-tools.yaml`;
      a.click();
      URL.revokeObjectURL(url);
    } catch (e) {
      toast.error(e instanceof Error ? e.message : String(e));
    }
  }

  async function upload(e: ChangeEvent<HTMLInputElement>) {
    const file = e.target.files?.[0];
    e.target.value = "";
    if (!file) return;
    setBusy(true);
    try {
      const r = await api.importBundle(project.id, await file.text(), policy);
      const parts = [
        `${r.tools_created.length} tools added`,
        r.tools_updated.length ? `${r.tools_updated.length} replaced` : "",
        r.tools_skipped.length ? `${r.tools_skipped.length} skipped` : "",
        r.auth_created.length ? `${r.auth_created.length} auth configs added` : "",
        r.connections_created.length ? `${r.connections_created.length} database connections added` : "",
      ].filter(Boolean);
      toast.success(`Imported ${file.name}: ${parts.join(", ")}`);
      await onImported();
    } catch (err) {
      const msg = err instanceof ApiError ? (err.fields[0] ? `${err.fields[0].field}: ${err.fields[0].message}` : err.message) : String(err);
      toast.error(msg);
    } finally {
      setBusy(false);
    }
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle>Export and import</CardTitle>
        <CardDescription>
          A YAML file with this project's tools and auth configs. Secrets stay as references, so the file is safe to share.
        </CardDescription>
      </CardHeader>
      <CardContent className="flex flex-wrap items-end gap-3">
        <Button variant="outline" onClick={() => void download()}>
          <DownloadIcon /> Download tools.yaml
        </Button>
        <div className="grid gap-1">
          <Label htmlFor="b-policy" className="text-xs text-muted-foreground">
            If a name already exists
          </Label>
          <Select value={policy} onValueChange={(v) => setPolicy(v as Policy)}>
            <SelectTrigger id="b-policy" className="w-48">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="fail">Stop, change nothing</SelectItem>
              <SelectItem value="skip">Keep what's here</SelectItem>
              <SelectItem value="replace">Use the file's version</SelectItem>
            </SelectContent>
          </Select>
        </div>
        <Label htmlFor="b-file" className="flex h-9 cursor-pointer items-center gap-2 rounded-lg border px-3 text-sm font-medium hover:bg-muted/60 has-focus-visible:ring-2 has-focus-visible:ring-ring">
          <UploadIcon className="size-4" /> {busy ? "Importing…" : "Import tools.yaml"}
          <input id="b-file" type="file" accept=".yaml,.yml,.json" className="sr-only" onChange={upload} disabled={busy} />
        </Label>
      </CardContent>
    </Card>
  );
}

function lines(text: string): string[] {
  return text
    .split("\n")
    .map((l) => l.trim())
    .filter(Boolean);
}

function CurationCard({ project, onSaved }: { project: Project; onSaved: (p: Project) => void }) {
  const st = project.settings;
  const [readOnly, setReadOnly] = useState(st.read_only);
  const [includeTags, setIncludeTags] = useState(st.include_tags.join(", "));
  const [excludeTags, setExcludeTags] = useState(st.exclude_tags.join(", "));
  const [includePaths, setIncludePaths] = useState(st.include_paths.join("\n"));
  const [excludePaths, setExcludePaths] = useState(st.exclude_paths.join("\n"));
  const [stripNulls, setStripNulls] = useState(st.strip_nulls);
  const [maxChars, setMaxChars] = useState(String(st.max_response_chars));
  const [toolSearch, setToolSearch] = useState(st.tool_search);
  const [err, setErr] = useState<unknown>(null);

  const next: ProjectSettings = {
    read_only: readOnly,
    include_tags: splitTags(includeTags),
    exclude_tags: splitTags(excludeTags),
    include_paths: lines(includePaths),
    exclude_paths: lines(excludePaths),
    strip_nulls: stripNulls,
    max_response_chars: Number(maxChars),
    tool_search: toolSearch,
  };
  const dirty = JSON.stringify(next) !== JSON.stringify(st);

  async function save(e: FormEvent) {
    e.preventDefault();
    try {
      onSaved(await api.updateSettings(project.id, next));
      setErr(null);
      toast.success("Curation saved. Connected agents get the new tool list.");
    } catch (e) {
      setErr(e);
      if (!(e instanceof ApiError && e.fields.length)) toast.error(e instanceof Error ? e.message : String(e));
    }
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle>What agents see</CardTitle>
        <CardDescription>Filters decide which tools are offered. Hidden tools can't be called either.</CardDescription>
      </CardHeader>
      <CardContent>
        <form onSubmit={save} className="grid gap-5">
          <Toggle id="c-ro" checked={readOnly} onChange={setReadOnly} label="Read-only" hint="Only GET, HEAD and OPTIONS tools." />
          <div className="grid gap-4 sm:grid-cols-2">
            <div className="grid gap-1.5">
              <Label htmlFor="c-it">Only tools tagged</Label>
              <Input id="c-it" value={includeTags} onChange={(e) => setIncludeTags(e.target.value)} placeholder="Any tag" />
            </div>
            <div className="grid gap-1.5">
              <Label htmlFor="c-et">Hide tools tagged</Label>
              <Input id="c-et" value={excludeTags} onChange={(e) => setExcludeTags(e.target.value)} placeholder="admin, internal" />
            </div>
            <div className="grid gap-1.5">
              <Label htmlFor="c-ip">Only paths matching</Label>
              <Textarea id="c-ip" value={includePaths} onChange={(e) => setIncludePaths(e.target.value)} placeholder="/repos/**" className="min-h-16 font-mono text-xs" />
            </div>
            <div className="grid gap-1.5">
              <Label htmlFor="c-ep">Hide paths matching</Label>
              <Textarea id="c-ep" value={excludePaths} onChange={(e) => setExcludePaths(e.target.value)} placeholder="/admin/**" className="min-h-16 font-mono text-xs" />
            </div>
          </div>
          <p className="-mt-2 text-xs text-muted-foreground">
            Tags are comma-separated. Paths go one per line: <code>*</code> matches within a segment, <code>**</code> across
            segments, and text without wildcards matches anywhere in the path.
          </p>
          <Toggle
            id="c-ts"
            checked={toolSearch}
            onChange={setToolSearch}
            label="Tool search"
            hint="Offer only search_tools and call_tool. Agents look tools up by keyword. Useful when a project has hundreds of tools."
          />
          <Toggle id="c-sn" checked={stripNulls} onChange={setStripNulls} label="Strip null fields" hint="Remove null values from JSON responses before the agent sees them." />
          <div className="grid max-w-xs gap-1.5">
            <Label htmlFor="c-max">Longest response (characters)</Label>
            <Input
              id="c-max"
              value={maxChars}
              onChange={(e) => setMaxChars(e.target.value)}
              inputMode="numeric"
              className="font-mono"
              aria-invalid={!!fieldError(err, "max_response_chars")}
            />
            {fieldError(err, "max_response_chars") ? (
              <p className="text-xs text-destructive">{fieldError(err, "max_response_chars")}</p>
            ) : (
              <p className="text-xs text-muted-foreground">Longer responses are cut, with a note telling the agent what was left out.</p>
            )}
          </div>
          <Button type="submit" className="w-fit" disabled={!dirty}>
            Save curation
          </Button>
        </form>
      </CardContent>
    </Card>
  );
}

function Toggle({
  id,
  checked,
  onChange,
  label,
  hint,
}: {
  id: string;
  checked: boolean;
  onChange: (v: boolean) => void;
  label: string;
  hint: string;
}) {
  return (
    <div className="flex items-start gap-3">
      <Switch id={id} checked={checked} onCheckedChange={onChange} className="mt-0.5" />
      <div className="grid gap-0.5">
        <Label htmlFor={id}>{label}</Label>
        <p className="text-xs text-muted-foreground">{hint}</p>
      </div>
    </div>
  );
}
