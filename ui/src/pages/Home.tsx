import { useEffect, useRef, useState, type FormEvent } from "react";
import { ChevronRightIcon, ServerIcon } from "lucide-react";

import { api, ApiError } from "@/api";
import { EmptyState } from "@/components/EmptyState";
import { PageHeader } from "@/components/PageHeader";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Skeleton } from "@/components/ui/skeleton";
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
      <PageHeader
        title="Projects"
        description="A project is one MCP server: a set of tools that agents reach at a single URL."
      />

      {error && (
        <Alert variant="destructive">
          <AlertDescription>{error}</AlertDescription>
        </Alert>
      )}

      <div className="grid items-start gap-6 lg:grid-cols-[minmax(0,1fr)_20rem]">
        <section aria-labelledby="list-heading" className="min-w-0">
          <h2 id="list-heading" className="sr-only">
            All projects
          </h2>
          {projects === null ? (
            <Skeleton className="h-40 w-full rounded-xl" />
          ) : projects.length > 0 ? (
            <ul className="divide-y overflow-hidden rounded-xl border bg-card">
              {projects.map((p) => (
                <li key={p.id}>
                  <a
                    href={href.project(p.id)}
                    className="group flex items-center gap-4 px-4 py-3.5 outline-none hover:bg-muted/40 focus-visible:bg-muted/40 focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset"
                  >
                    <span
                      aria-hidden="true"
                      className="grid size-9 shrink-0 place-items-center rounded-lg border bg-background text-sm font-semibold text-primary uppercase"
                    >
                      {p.name.slice(0, 1)}
                    </span>
                    <span className="grid min-w-0 flex-1 gap-0.5">
                      <span className="truncate font-medium">{p.name}</span>
                      <span className="truncate font-mono text-xs text-muted-foreground">/mcp/{p.slug}</span>
                    </span>
                    <span className="hidden shrink-0 text-xs text-muted-foreground sm:block">
                      Created {new Date(p.created_at).toLocaleDateString()}
                    </span>
                    <ChevronRightIcon className="size-4 shrink-0 text-muted-foreground transition-transform group-hover:translate-x-0.5" />
                  </a>
                </li>
              ))}
            </ul>
          ) : (
            <EmptyState
              icon={ServerIcon}
              title="Create your first project"
              description="Each project gets its own MCP URL. Add HTTP or SQL tools to it, test them here, then connect an agent."
            />
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
