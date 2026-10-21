import { useEffect, useRef, useState, type FormEvent } from "react";

import { api, ApiError } from "@/api";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { fieldError, useProjects } from "@/lib/projects";
import { href, navigate } from "@/lib/router";
import { slugify } from "@/lib/template";

export function Home() {
  const { projects, error, reload } = useProjects();
  const nameRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (window.location.hash.includes("new")) nameRef.current?.focus();
  }, []);

  return (
    <div className="space-y-8">
      <div className="space-y-1">
        <h1 className="text-2xl font-semibold tracking-tight">Projects</h1>
        <p className="max-w-prose text-sm text-muted-foreground">
          A project is one MCP server: a set of tools that agents connect to at a single URL.
        </p>
      </div>

      {error && (
        <Alert variant="destructive">
          <AlertDescription>{error}</AlertDescription>
        </Alert>
      )}

      <div className="grid gap-6 lg:grid-cols-[1fr_20rem]">
        <section aria-labelledby="list-heading" className="min-w-0">
          <h2 id="list-heading" className="sr-only">
            All projects
          </h2>
          {projects && projects.length > 0 ? (
            <div className="rounded-lg border bg-card">
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>Name</TableHead>
                    <TableHead>MCP path</TableHead>
                    <TableHead className="hidden sm:table-cell">Created</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {projects.map((p) => (
                    <TableRow key={p.id} className="cursor-pointer" onClick={() => navigate(href.project(p.id))}>
                      <TableCell className="font-medium">
                        <a href={href.project(p.id)} className="hover:underline" onClick={(e) => e.stopPropagation()}>
                          {p.name}
                        </a>
                      </TableCell>
                      <TableCell className="font-mono text-xs text-muted-foreground">/mcp/{p.slug}</TableCell>
                      <TableCell className="hidden text-muted-foreground sm:table-cell">
                        {new Date(p.created_at).toLocaleDateString()}
                      </TableCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            </div>
          ) : (
            projects && (
              <div className="rounded-lg border border-dashed p-8 text-center">
                <p className="font-medium">No projects yet</p>
                <p className="mt-1 text-sm text-muted-foreground">
                  Create one to start turning an API into tools an agent can call.
                </p>
              </div>
            )
          )}
        </section>

        <CreateProject nameRef={nameRef} onCreated={reload} />
      </div>
    </div>
  );
}

function CreateProject({ nameRef, onCreated }: { nameRef: React.RefObject<HTMLInputElement | null>; onCreated: () => Promise<void> }) {
  const [name, setName] = useState("");
  const [slug, setSlug] = useState("");
  const [err, setErr] = useState<unknown>(null);
  const [busy, setBusy] = useState(false);

  async function submit(e: FormEvent) {
    e.preventDefault();
    setBusy(true);
    try {
      const p = await api.createProject({ name, slug: slug.trim() || undefined });
      await onCreated();
      navigate(href.project(p.id));
    } catch (e) {
      setErr(e);
    } finally {
      setBusy(false);
    }
  }

  const slugPreview = slug.trim() || slugify(name);
  return (
    <Card className="h-fit">
      <CardHeader>
        <CardTitle>New project</CardTitle>
        <CardDescription>The slug becomes part of the MCP URL.</CardDescription>
      </CardHeader>
      <CardContent>
        <form onSubmit={submit} className="grid gap-4">
          <div className="grid gap-1.5">
            <Label htmlFor="p-name">Name</Label>
            <Input id="p-name" ref={nameRef} value={name} onChange={(e) => setName(e.target.value)} placeholder="GitHub" required aria-invalid={!!fieldError(err, "name")} />
            {fieldError(err, "name") && <p className="text-xs text-destructive">{fieldError(err, "name")}</p>}
          </div>
          <div className="grid gap-1.5">
            <Label htmlFor="p-slug">Slug (optional)</Label>
            <Input id="p-slug" value={slug} onChange={(e) => setSlug(e.target.value)} placeholder={slugify(name) || "github"} className="font-mono" aria-invalid={!!fieldError(err, "slug")} />
            {fieldError(err, "slug") ? (
              <p className="text-xs text-destructive">{fieldError(err, "slug")}</p>
            ) : (
              slugPreview && <p className="font-mono text-xs text-muted-foreground">/mcp/{slugPreview}</p>
            )}
          </div>
          {err instanceof ApiError && err.fields.length === 0 && (
            <Alert variant="destructive">
              <AlertDescription>{err.message}</AlertDescription>
            </Alert>
          )}
          <Button type="submit" disabled={busy || !name.trim()}>
            Create project
          </Button>
        </form>
      </CardContent>
    </Card>
  );
}
