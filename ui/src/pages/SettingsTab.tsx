import { useState, type FormEvent } from "react";
import { toast } from "sonner";

import { api, ApiError } from "@/api";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
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
