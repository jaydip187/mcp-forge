import { useState, type ChangeEvent, type FormEvent } from "react";
import { DownloadIcon, UploadIcon } from "lucide-react";
import { toast } from "sonner";

import { api, ApiError } from "@/api";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { fieldError, useProjects } from "@/lib/projects";
import { href, navigate } from "@/lib/router";
import type { Project } from "@/types";

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
