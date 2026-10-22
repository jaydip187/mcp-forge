import { useEffect, useState, type ChangeEvent, type FormEvent, type ReactNode } from "react";
import { UploadIcon } from "lucide-react";
import { toast } from "sonner";

import { api, ApiError } from "@/api";
import { PageHeader } from "@/components/PageHeader";
import { MethodLabel } from "@/components/RequestLine";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { listLabels } from "@/lib/fields";
import { href, navigate } from "@/lib/router";
import { describeAuth } from "@/pages/AuthTab";
import type { AuthConfig, ReimportPreview } from "@/types";

interface Source {
  content?: string;
  filename?: string | null;
  sourceUrl?: string | null;
  label: string;
}

function errorText(e: unknown): string {
  if (e instanceof ApiError) return e.fields[0]?.message ?? e.message;
  return e instanceof Error ? e.message : String(e);
}

export function ReimportPage({ projectId, specId }: { projectId: string; specId: string }) {
  const [preview, setPreview] = useState<ReimportPreview | null>(null);
  const [source, setSource] = useState<Source>({ label: "the stored copy" });
  const [baseUrl, setBaseUrl] = useState("");
  const [authRef, setAuthRef] = useState<string>("");
  const [auths, setAuths] = useState<AuthConfig[]>([]);
  const [url, setUrl] = useState("");
  const [add, setAdd] = useState<Set<string>>(new Set());
  const [remove, setRemove] = useState<Set<string>>(new Set());
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function load(src: Source, opts: { url?: string; base_url?: string; auth_ref?: string } = {}) {
    setBusy(true);
    setError(null);
    try {
      const pv = await api.previewReimport(projectId, specId, { content: src.content, ...opts });
      const nextSource = opts.url ? { content: pv.content ?? undefined, sourceUrl: pv.source_url, label: opts.url } : src;
      setPreview(pv);
      setSource(nextSource);
      setBaseUrl(pv.base_url ?? "");
      setAuthRef(pv.auth_ref ?? "");
      setAdd(new Set(pv.added.filter((o) => o.supported).map((o) => o.key)));
      setRemove(new Set());
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  }

  useEffect(() => {
    api.listAuth(projectId).then(setAuths, () => {});
    void load({ label: "the stored copy" });
  }, [projectId, specId]);

  async function onFile(e: ChangeEvent<HTMLInputElement>) {
    const file = e.target.files?.[0];
    e.target.value = "";
    if (!file) return;
    await load({ content: await file.text(), filename: file.name, label: file.name });
  }

  async function onFetch(e: FormEvent) {
    e.preventDefault();
    await load({ label: url }, { url: url.trim() });
  }

  async function applyOptions() {
    await load(source, { base_url: baseUrl.trim(), auth_ref: authRef });
  }

  async function apply() {
    if (!preview) return;
    setBusy(true);
    setError(null);
    try {
      const res = await api.reimport(projectId, specId, {
        content: source.content,
        source_url: source.sourceUrl,
        filename: source.filename,
        base_url: baseUrl.trim(),
        auth_ref: authRef,
        add: [...add],
        remove: [...remove],
      });
      const r = res.result;
      toast.success(`Re-imported: ${r.updated.length} updated, ${r.created.length} added, ${r.deleted.length} removed`);
      navigate(href.project(projectId));
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  }

  const optionsChanged =
    preview !== null && (baseUrl.trim() !== (preview.base_url ?? "") || authRef !== (preview.auth_ref ?? ""));
  const nothingToDo = preview !== null && preview.changed.length === 0 && add.size === 0 && remove.size === 0;

  return (
    <div className="space-y-6">
      <PageHeader
        back={{ href: href.project(projectId, "specs"), label: "Specs" }}
        title={`Re-import ${preview?.spec.title ?? "spec"}`}
        description="Tools take the new spec's values, except for fields you edited by hand. Review the changes, then apply them."
      />

      <Card>
        <CardHeader>
          <CardTitle>New version</CardTitle>
          <CardDescription>Comparing against {source.label}.</CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <Tabs defaultValue={preview?.spec.source_url ? "url" : "file"}>
            <TabsList>
              <TabsTrigger value="file">Upload a file</TabsTrigger>
              <TabsTrigger value="url">From a URL</TabsTrigger>
            </TabsList>
            <TabsContent value="file" className="pt-3">
              <Label htmlFor="re-file" className="flex w-fit cursor-pointer items-center gap-2 rounded-lg border border-dashed px-4 py-3 text-sm hover:bg-muted/60 has-focus-visible:ring-2 has-focus-visible:ring-ring">
                <UploadIcon className="size-4" /> Choose the updated spec
                <input id="re-file" type="file" accept=".json,.yaml,.yml" className="sr-only" onChange={onFile} disabled={busy} />
              </Label>
            </TabsContent>
            <TabsContent value="url" className="pt-3">
              <form onSubmit={onFetch} className="flex flex-col gap-2 sm:flex-row">
                <Input value={url} onChange={(e) => setUrl(e.target.value)} placeholder={preview?.spec.source_url ?? "https://api.example.com/openapi.json"} className="font-mono" aria-label="Spec URL" type="url" required />
                <Button type="submit" disabled={busy || !url.trim()}>
                  Fetch spec
                </Button>
              </form>
            </TabsContent>
          </Tabs>
          {preview && (
            <div className="grid gap-4 lg:grid-cols-[1fr_1fr_auto] lg:items-end">
              <div className="grid gap-1.5">
                <Label htmlFor="re-base">Base URL</Label>
                <Input id="re-base" value={baseUrl} onChange={(e) => setBaseUrl(e.target.value)} placeholder="From the spec's servers" className="font-mono" />
              </div>
              <div className="grid gap-1.5">
                <Label htmlFor="re-auth">Auth</Label>
                <Select value={authRef || "none"} onValueChange={(v) => setAuthRef(v === "none" ? "" : v)}>
                  <SelectTrigger id="re-auth" className="w-full">
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
              </div>
              <Button variant="outline" onClick={() => void applyOptions()} disabled={busy || !optionsChanged}>
                Update preview
              </Button>
            </div>
          )}
          {error && (
            <Alert variant="destructive">
              <AlertDescription>{error}</AlertDescription>
            </Alert>
          )}
        </CardContent>
      </Card>

      {!preview && !error && <Skeleton className="h-40 w-full" />}
      {preview && (
        <>
          <p className="text-sm text-muted-foreground">
            {preview.info.title} v{preview.info.api_version}: {preview.changed.length} changed, {preview.added.length} new,{" "}
            {preview.removed.length} gone, {preview.unchanged} unchanged.
          </p>

          <Section title="Changed" empty="No existing tools change." count={preview.changed.length}>
            {preview.changed.map((c) => (
              <li key={c.tool_id} className="px-4 py-2.5 text-sm">
                <span className="font-mono">{c.name}</span>
                {c.new_name !== c.name && <span className="font-mono text-muted-foreground"> → {c.new_name}</span>}
                <span className="text-muted-foreground"> updates its {listLabels(c.fields)}.</span>
                {c.kept.length > 0 && <span className="block text-xs text-muted-foreground">Keeps your {listLabels(c.kept)}.</span>}
              </li>
            ))}
          </Section>

          <Section title="New in the spec" empty="No new operations." count={preview.added.length}>
            {preview.added.map((o) => (
              <li key={o.key} className="flex items-start gap-3 px-4 py-2.5">
                <Checkbox
                  id={`add-${o.key}`}
                  checked={add.has(o.key)}
                  disabled={!o.supported}
                  onCheckedChange={(v) =>
                    setAdd((s) => {
                      const n = new Set(s);
                      if (v === true) n.add(o.key);
                      else n.delete(o.key);
                      return n;
                    })
                  }
                  className="mt-1"
                />
                <label htmlFor={`add-${o.key}`} className="min-w-0 flex-1 text-sm">
                  <span className="inline-flex items-baseline gap-2 font-mono">
                    <MethodLabel method={o.method} />
                    <span className="break-all">{o.path}</span>
                  </span>
                  <span className="ml-2 font-mono text-xs text-muted-foreground">{o.name}</span>
                  {!o.supported && <span className="block text-destructive">{o.warnings[0]}</span>}
                </label>
              </li>
            ))}
          </Section>

          <Section title="Gone from the spec" empty="Nothing was removed." count={preview.removed.length}>
            {preview.removed.map((t) => (
              <li key={t.tool_id} className="flex items-start gap-3 px-4 py-2.5">
                <Checkbox
                  id={`rm-${t.tool_id}`}
                  checked={remove.has(t.tool_id)}
                  onCheckedChange={(v) =>
                    setRemove((s) => {
                      const n = new Set(s);
                      if (v === true) n.add(t.tool_id);
                      else n.delete(t.tool_id);
                      return n;
                    })
                  }
                  className="mt-1"
                />
                <label htmlFor={`rm-${t.tool_id}`} className="text-sm">
                  Delete <span className="font-mono">{t.name}</span>
                  <span className="block text-xs text-muted-foreground">Unticked tools are kept as they are.</span>
                </label>
              </li>
            ))}
          </Section>

          {preview.blocked.length > 0 && (
            <Section title="Can't update" empty="" count={preview.blocked.length}>
              {preview.blocked.map((b) => (
                <li key={b.tool_id} className="px-4 py-2.5 text-sm">
                  <span className="font-mono">{b.name}</span> stays as it is. <span className="text-muted-foreground">{b.warnings[0]}</span>
                </li>
              ))}
            </Section>
          )}

          <div className="sticky bottom-0 -mx-1 flex flex-wrap items-center gap-3 border-t bg-background/95 px-1 py-3 backdrop-blur">
            <Button onClick={() => void apply()} disabled={busy || optionsChanged || nothingToDo}>
              Apply re-import
            </Button>
            {optionsChanged && <span className="text-sm text-muted-foreground">Update the preview first.</span>}
            {!optionsChanged && nothingToDo && <span className="text-sm text-muted-foreground">Everything is up to date.</span>}
          </div>
        </>
      )}
    </div>
  );
}

function Section({ title, empty, count, children }: { title: string; empty: string; count: number; children: ReactNode }) {
  return (
    <section className="space-y-2">
      <h2 className="flex items-center gap-2 text-sm font-medium">
        {title} <Badge variant="secondary">{count}</Badge>
      </h2>
      {count === 0 ? (
        <p className="text-sm text-muted-foreground">{empty}</p>
      ) : (
        <ul className="divide-y rounded-lg border bg-card">{children}</ul>
      )}
    </section>
  );
}
