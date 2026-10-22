import { useEffect, useMemo, useState, type ChangeEvent, type FormEvent } from "react";
import { AlertTriangleIcon, KeyRoundIcon, UploadIcon } from "lucide-react";
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
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { pathMatcher } from "@/lib/glob";
import { href, navigate } from "@/lib/router";
import { splitTemplate } from "@/lib/template";
import { describeAuth } from "@/pages/AuthTab";
import { METHODS, type AuthConfig, type HttpMethod, type ImportOperation, type ImportPreview } from "@/types";

/** Specs at or under this many operations start with everything selected. */
const SELECT_ALL_UNDER = 50;

interface Source {
  content: string;
  filename: string | null;
  sourceUrl: string | null;
}

function errorText(e: unknown): string {
  if (e instanceof ApiError) return e.fields[0]?.message ?? e.message;
  return e instanceof Error ? e.message : String(e);
}

export function ImportPage({ projectId }: { projectId: string }) {
  const [source, setSource] = useState<Source | null>(null);
  const [preview, setPreview] = useState<ImportPreview | null>(null);
  const [baseUrl, setBaseUrl] = useState("");
  const [busy, setBusy] = useState<"reading" | "fetching" | "previewing" | "importing" | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [url, setUrl] = useState("");
  const [auths, setAuths] = useState<AuthConfig[]>([]);
  const [authRef, setAuthRef] = useState<string | null>(null);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [tag, setTag] = useState("all");
  const [methods, setMethods] = useState<Set<HttpMethod>>(new Set());
  const [glob, setGlob] = useState("");

  useEffect(() => {
    api.listAuth(projectId).then(setAuths, () => {});
  }, [projectId]);

  function adopt(pv: ImportPreview, src: Source, keepSelection: boolean) {
    setPreview(pv);
    setSource(src);
    setBaseUrl((b) => (keepSelection ? b : (pv.info.base_url ?? "")));
    if (!keepSelection) {
      const supported = pv.operations.filter((o) => o.supported).map((o) => o.key);
      setSelected(new Set(supported.length <= SELECT_ALL_UNDER ? supported : []));
      setTag("all");
      setMethods(new Set());
      setGlob("");
    } else {
      // Drop selections that became unsupported.
      const ok = new Set(pv.operations.filter((o) => o.supported).map((o) => o.key));
      setSelected((s) => new Set([...s].filter((k) => ok.has(k))));
    }
  }

  async function loadFile(e: ChangeEvent<HTMLInputElement>) {
    const file = e.target.files?.[0];
    e.target.value = "";
    if (!file) return;
    setError(null);
    setBusy("reading");
    try {
      const content = await file.text();
      setBusy("previewing");
      const pv = await api.previewOpenApi(projectId, { content });
      adopt(pv, { content, filename: file.name, sourceUrl: null }, false);
    } catch (err) {
      setError(errorText(err));
    } finally {
      setBusy(null);
    }
  }

  async function fetchUrl(e: FormEvent) {
    e.preventDefault();
    setError(null);
    setBusy("fetching");
    try {
      const pv = await api.previewOpenApi(projectId, { url: url.trim() });
      adopt(pv, { content: pv.content ?? "", filename: null, sourceUrl: pv.source_url }, false);
    } catch (err) {
      setError(errorText(err));
    } finally {
      setBusy(null);
    }
  }

  async function applyBaseUrl() {
    if (!source) return;
    setError(null);
    setBusy("previewing");
    try {
      const pv = await api.previewOpenApi(projectId, {
        content: source.content,
        base_url: baseUrl.trim() || undefined,
      });
      adopt({ ...pv, source_url: source.sourceUrl }, source, true);
    } catch (err) {
      setError(errorText(err));
    } finally {
      setBusy(null);
    }
  }

  async function runImport() {
    if (!source || !preview) return;
    setError(null);
    setBusy("importing");
    try {
      const keys = preview.operations.filter((o) => selected.has(o.key)).map((o) => ({ key: o.key }));
      const res = await api.importOpenApi(projectId, {
        content: source.content,
        source_url: source.sourceUrl,
        filename: source.filename,
        base_url: baseUrl.trim() || undefined,
        auth_ref: authRef,
        operations: keys,
      });
      toast.success(`Imported ${res.created.length} ${res.created.length === 1 ? "tool" : "tools"} from ${res.spec.title}`);
      navigate(href.project(projectId));
    } catch (err) {
      setError(errorText(err));
    } finally {
      setBusy(null);
    }
  }

  const tags = useMemo(() => [...new Set(preview?.operations.flatMap((o) => o.tags) ?? [])].sort(), [preview]);
  const visible = useMemo(() => {
    if (!preview) return [];
    const match = pathMatcher(glob);
    return preview.operations.filter(
      (o) => (tag === "all" || o.tags.includes(tag)) && (methods.size === 0 || methods.has(o.method)) && match(o.path),
    );
  }, [preview, tag, methods, glob]);
  const visibleSupported = visible.filter((o) => o.supported);
  const allVisibleSelected = visibleSupported.length > 0 && visibleSupported.every((o) => selected.has(o.key));
  const someVisibleSelected = visibleSupported.some((o) => selected.has(o.key));
  const baseChanged = preview !== null && baseUrl.trim() !== (preview.operations[0]?.base_url ?? preview.info.base_url ?? "");

  function toggleVisible(on: boolean) {
    setSelected((s) => {
      const next = new Set(s);
      for (const o of visibleSupported) {
        if (on) next.add(o.key);
        else next.delete(o.key);
      }
      return next;
    });
  }

  function toggle(key: string, on: boolean) {
    setSelected((s) => {
      const next = new Set(s);
      if (on) next.add(key);
      else next.delete(key);
      return next;
    });
  }

  return (
    <div className="space-y-6">
      <PageHeader
        back={{ href: href.project(projectId), label: "Tools" }}
        title="Import from an API spec"
        description="Turn the operations in an OpenAPI 3.0, 3.1 or Swagger 2.0 spec into tools. You pick which ones; nothing is created until you import."
      />

      <Card>
        <CardHeader>
          <CardTitle>Spec</CardTitle>
          <CardDescription>
            {source
              ? source.filename
                ? `Loaded ${source.filename}.`
                : `Fetched from ${source.sourceUrl}.`
              : "JSON or YAML. A URL is fetched once, when you press Fetch."}
          </CardDescription>
        </CardHeader>
        <CardContent>
          <Tabs defaultValue="file">
            <TabsList>
              <TabsTrigger value="file">Upload a file</TabsTrigger>
              <TabsTrigger value="url">From a URL</TabsTrigger>
            </TabsList>
            <TabsContent value="file" className="pt-3">
              <Label htmlFor="spec-file" className="flex w-fit cursor-pointer items-center gap-2 rounded-lg border border-dashed px-4 py-3 text-sm hover:bg-muted/60 has-focus-visible:ring-2 has-focus-visible:ring-ring">
                <UploadIcon className="size-4" />
                {busy === "reading" || busy === "previewing" ? "Reading…" : "Choose a .json or .yaml spec"}
                <input id="spec-file" type="file" accept=".json,.yaml,.yml,application/json,application/yaml,text/yaml" className="sr-only" onChange={loadFile} disabled={busy !== null} />
              </Label>
            </TabsContent>
            <TabsContent value="url" className="pt-3">
              <form onSubmit={fetchUrl} className="flex flex-col gap-2 sm:flex-row">
                <Input value={url} onChange={(e) => setUrl(e.target.value)} placeholder="https://api.example.com/openapi.json" className="font-mono" aria-label="Spec URL" type="url" required />
                <Button type="submit" disabled={busy !== null || !url.trim()}>
                  {busy === "fetching" ? "Fetching…" : "Fetch spec"}
                </Button>
              </form>
            </TabsContent>
          </Tabs>
          {error && (
            <Alert variant="destructive" className="mt-4">
              <AlertDescription>{error}</AlertDescription>
            </Alert>
          )}
        </CardContent>
      </Card>

      {preview && source && (
        <>
          <Card>
            <CardHeader>
              <CardTitle>
                {preview.info.title} {preview.info.api_version && <span className="font-normal text-muted-foreground">v{preview.info.api_version}</span>}
              </CardTitle>
              <CardDescription>
                {preview.info.kind === "swagger2" ? "Swagger 2.0" : `OpenAPI ${preview.info.openapi}`}. {preview.operations.length} operations.
              </CardDescription>
            </CardHeader>
            <CardContent className="grid gap-4 lg:grid-cols-2">
              <div className="grid gap-1.5">
                <Label htmlFor="imp-base">Base URL</Label>
                <div className="flex gap-2">
                  <Input id="imp-base" value={baseUrl} onChange={(e) => setBaseUrl(e.target.value)} placeholder="https://api.example.com/v1" className="font-mono" />
                  <Button type="button" variant="outline" onClick={() => void applyBaseUrl()} disabled={busy !== null || !baseChanged}>
                    Apply
                  </Button>
                </div>
                <p className="text-xs text-muted-foreground">
                  {preview.info.base_url
                    ? "From the spec's servers. Change it to point at another environment."
                    : preview.info.servers.length
                      ? `The spec's server (${preview.info.servers[0]}) is relative. Enter the full URL.`
                      : "The spec lists no servers. Enter the API's base URL."}
                </p>
              </div>
              <div className="grid gap-1.5">
                <Label htmlFor="imp-auth">Auth for operations that need it</Label>
                <Select value={authRef ?? "none"} onValueChange={(v) => setAuthRef(v === "none" ? null : v)}>
                  <SelectTrigger id="imp-auth" className="w-full">
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
                {preview.info.security_schemes.length > 0 ? (
                  <p className="text-xs text-muted-foreground">
                    The spec expects {preview.info.security_schemes.map((s) => s.description).join(" or ")}.{" "}
                    {auths.length === 0 && (
                      <a className="underline" href={href.project(projectId, "auth")}>
                        Add an auth config
                      </a>
                    )}
                  </p>
                ) : (
                  <p className="text-xs text-muted-foreground">The spec doesn't describe auth; this applies to every imported tool.</p>
                )}
              </div>
            </CardContent>
          </Card>

          <section aria-label="Operations" className="space-y-3">
            <div className="flex flex-wrap items-end gap-3">
              <div className="grid gap-1">
                <Label htmlFor="f-tag" className="text-xs text-muted-foreground">
                  Tag
                </Label>
                <Select value={tag} onValueChange={setTag}>
                  <SelectTrigger id="f-tag" className="w-48">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="all">All tags</SelectItem>
                    {tags.map((t) => (
                      <SelectItem key={t} value={t}>
                        {t}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
              <div className="grid gap-1">
                <span className="text-xs text-muted-foreground" id="f-methods">
                  Methods
                </span>
                <div className="flex flex-wrap gap-1" role="group" aria-labelledby="f-methods">
                  {METHODS.filter((m) => preview.operations.some((o) => o.method === m)).map((m) => (
                    <Button
                      key={m}
                      type="button"
                      size="sm"
                      variant={methods.has(m) ? "secondary" : "ghost"}
                      aria-pressed={methods.has(m)}
                      onClick={() =>
                        setMethods((s) => {
                          const n = new Set(s);
                          if (n.has(m)) n.delete(m);
                          else n.add(m);
                          return n;
                        })
                      }
                    >
                      <MethodLabel method={m} className="text-xs" />
                    </Button>
                  ))}
                </div>
              </div>
              <div className="grid min-w-56 flex-1 gap-1">
                <Label htmlFor="f-path" className="text-xs text-muted-foreground">
                  Path
                </Label>
                <Input id="f-path" value={glob} onChange={(e) => setGlob(e.target.value)} placeholder="/repos/*/*/issues or /pets/**" className="font-mono" />
              </div>
            </div>

            <div className="rounded-lg border bg-card">
              <div className="flex items-center gap-3 border-b px-4 py-2 text-sm">
                <Checkbox
                  checked={allVisibleSelected ? true : someVisibleSelected ? "indeterminate" : false}
                  onCheckedChange={(v) => toggleVisible(v === true)}
                  aria-label="Select all shown operations"
                  disabled={visibleSupported.length === 0}
                />
                <span className="text-muted-foreground">
                  Showing {visible.length} of {preview.operations.length}. {selected.size} selected.
                </span>
              </div>
              <ul className="max-h-[60vh] divide-y overflow-y-auto">
                {visible.map((o) => (
                  <OperationRow key={o.key} op={o} checked={selected.has(o.key)} onChange={(v) => toggle(o.key, v)} />
                ))}
                {visible.length === 0 && <li className="px-4 py-6 text-center text-sm text-muted-foreground">No operations match these filters.</li>}
              </ul>
            </div>
          </section>

          <div className="sticky bottom-0 -mx-1 flex flex-wrap items-center gap-3 border-t bg-background/95 px-1 py-3 backdrop-blur">
            <Button onClick={() => void runImport()} disabled={busy !== null || selected.size === 0 || baseChanged}>
              {busy === "importing" ? "Importing…" : `Import ${selected.size} ${selected.size === 1 ? "tool" : "tools"}`}
            </Button>
            {baseChanged && <span className="text-sm text-muted-foreground">Apply the base URL first.</span>}
          </div>
        </>
      )}
    </div>
  );
}

function OperationRow({ op, checked, onChange }: { op: ImportOperation; checked: boolean; onChange: (v: boolean) => void }) {
  const id = `op-${op.key.replace(/[^A-Za-z0-9]/g, "_")}`;
  const blocker = op.warnings.find((w) => w.startsWith("Can't import"));
  const notes = op.warnings.filter((w) => w !== blocker);
  return (
    <li className={`flex gap-3 px-4 py-2.5 ${op.supported ? "" : "opacity-70"}`}>
      <Checkbox id={id} checked={checked} onCheckedChange={(v) => onChange(v === true)} disabled={!op.supported} className="mt-1" />
      <label htmlFor={id} className="min-w-0 flex-1 cursor-pointer">
        <span className="flex flex-wrap items-baseline gap-x-3 gap-y-0.5">
          <span className="inline-flex min-w-0 items-baseline gap-2 font-mono text-sm">
            <MethodLabel method={op.method} className="w-[4.5em] shrink-0" />
            <span className="break-all">
              {splitTemplate(op.path).map((s, i) =>
                s.kind === "text" ? <span key={i}>{s.text}</span> : <span key={i} className="text-primary">{`{${s.name}}`}</span>,
              )}
            </span>
          </span>
          <span className="font-mono text-xs text-muted-foreground">{op.name}</span>
          {op.deprecated && <Badge variant="outline">Deprecated</Badge>}
        </span>
        {op.summary && <span className="mt-0.5 block line-clamp-1 text-sm text-muted-foreground">{op.summary}</span>}
        {blocker && <span className="mt-0.5 block text-sm text-destructive">{blocker}</span>}
      </label>
      <span className="flex shrink-0 items-start gap-1.5 pt-1">
        {op.needs_auth && (
          <Tooltip>
            <TooltipTrigger asChild>
              <KeyRoundIcon className="size-4 text-muted-foreground" aria-label="Needs auth" />
            </TooltipTrigger>
            <TooltipContent>The spec says this needs credentials</TooltipContent>
          </Tooltip>
        )}
        {notes.length > 0 && (
          <Tooltip>
            <TooltipTrigger asChild>
              <AlertTriangleIcon className="size-4 text-m-put" aria-label={`${notes.length} notes`} />
            </TooltipTrigger>
            <TooltipContent className="max-w-sm">
              <ul className="list-disc pl-4">
                {notes.map((n, i) => (
                  <li key={i}>{n}</li>
                ))}
              </ul>
            </TooltipContent>
          </Tooltip>
        )}
      </span>
    </li>
  );
}
