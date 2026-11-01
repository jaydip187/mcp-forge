import { useCallback, useEffect, useState } from "react";
import { SparklesIcon } from "lucide-react";
import { toast } from "sonner";

import { api, ApiError } from "@/api";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Skeleton } from "@/components/ui/skeleton";
import { useCanWrite } from "@/lib/permissions";
import { href } from "@/lib/router";
import type { Improvement, LintReport, Project, Tool, ToolKind } from "@/types";

/** Copies suggested parameter descriptions into a tool's kind. */
export function withParamDescriptions(kind: ToolKind, descs: Record<string, string>): ToolKind {
  const pick = (name: string, old: string | null) => (name in descs ? descs[name]! : old);
  switch (kind.type) {
    case "http":
      return { ...kind, params: kind.params.map((p) => ({ ...p, description: pick(p.name, p.description) })) };
    case "graphql":
      return { ...kind, variables: kind.variables.map((v) => ({ ...v, description: pick(v.name, v.description) ?? "" })) };
    case "nosql":
    case "chain":
      return { ...kind, params: kind.params.map((p) => ({ ...p, description: pick(p.name, p.description) })) };
    case "sql":
      return kind.mode.type === "query"
        ? { ...kind, mode: { ...kind.mode, params: kind.mode.params.map((p) => ({ ...p, description: pick(p.name, p.description) })) } }
        : kind;
  }
}

/** How well tools describe themselves to agents, with fixes. */
export function QualityTab({ project, onSaved }: { project: Project; onSaved: (p: Project) => void }) {
  const canWrite = useCanWrite(project.id);
  const [report, setReport] = useState<LintReport | null>(null);
  const [keyRef, setKeyRef] = useState(project.settings.anthropic_key_ref ?? "");
  const [keyErr, setKeyErr] = useState<string | null>(null);
  const [improving, setImproving] = useState<string | null>(null);
  const [suggestions, setSuggestions] = useState<Record<string, Improvement>>({});

  const load = useCallback(async () => {
    setReport(await api.lintProject(project.id));
  }, [project.id]);
  useEffect(() => {
    load().catch((e) => toast.error(String(e)));
  }, [load]);

  async function saveKey() {
    try {
      onSaved(await api.updateSettings(project.id, { ...project.settings, anthropic_key_ref: keyRef.trim() || null }));
      setKeyErr(null);
      toast.success(keyRef.trim() ? "Improve with Claude is on" : "Improve with Claude is off");
      await load();
    } catch (e) {
      setKeyErr(e instanceof ApiError ? (e.fields[0]?.message ?? e.message) : String(e));
    }
  }

  async function improve(id: string) {
    setImproving(id);
    try {
      const s = await api.improveTool(project.id, id);
      setSuggestions((m) => ({ ...m, [id]: s }));
    } catch (e) {
      toast.error(e instanceof Error ? e.message : String(e));
    } finally {
      setImproving(null);
    }
  }

  async function apply(id: string) {
    const s = suggestions[id];
    if (!s) return;
    try {
      const t: Tool = await api.getTool(project.id, id);
      const descs = Object.fromEntries(s.suggested.params.map((p) => [p.name, p.description]));
      await api.updateTool(project.id, id, {
        name: t.name,
        description: s.suggested.description,
        kind: withParamDescriptions(t.kind, descs),
        enabled: t.enabled,
        tags: t.tags,
        response_shape: t.response_shape,
        require_approval: t.require_approval,
      });
      toast.success(`Updated ${t.name}`);
      setSuggestions(({ [id]: _, ...rest }) => rest);
      await load();
    } catch (e) {
      toast.error(e instanceof ApiError ? (e.fields[0]?.message ?? e.message) : String(e));
    }
  }

  if (!report) return <Skeleton className="h-40 w-full" />;

  return (
    <div className="space-y-6">
      <Card>
        <CardHeader>
          <CardTitle>Description quality: {report.score}/100</CardTitle>
          <CardDescription>
            Agents pick tools by their names and descriptions. These checks run here, offline; the score averages the tools
            agents can see.
          </CardDescription>
        </CardHeader>
        <CardContent className="grid gap-1.5 sm:max-w-xl">
          <Label htmlFor="q-key">Improve with Claude (optional)</Label>
          <div className="flex gap-2">
            <Input id="q-key" value={keyRef} onChange={(e) => setKeyRef(e.target.value)} placeholder="env:ANTHROPIC_API_KEY" className="font-mono" disabled={!canWrite} />
            <Button variant="outline" onClick={() => void saveKey()} disabled={!canWrite || keyRef.trim() === (project.settings.anthropic_key_ref ?? "")}>
              Save
            </Button>
          </div>
          {keyErr ? (
            <p className="text-xs text-destructive">{keyErr}</p>
          ) : (
            <p className="text-xs text-muted-foreground">
              A reference to an Anthropic API key (<code>env:NAME</code> or <code>keyring:NAME</code>). When set, each tool can
              ask Claude for better wording; the tool's definition and findings are sent to the Anthropic API. Suggestions are
              shown first and only saved when you apply them.
            </p>
          )}
        </CardContent>
      </Card>

      {report.tools.length === 0 ? (
        <p className="text-sm text-muted-foreground">No tools yet.</p>
      ) : (
        <ul className="divide-y overflow-hidden rounded-xl border bg-card">
          {report.tools.map((t) => {
            const s = suggestions[t.id];
            return (
              <li key={t.id} className="space-y-2 px-4 py-3">
                <div className="flex flex-wrap items-center gap-2">
                  <a className="font-mono font-medium hover:underline" href={href.tool(project.id, t.id)}>
                    {t.name}
                  </a>
                  <Badge variant={t.score >= 90 ? "secondary" : t.score >= 60 ? "outline" : "destructive"}>{t.score}</Badge>
                  {!t.enabled && <span className="text-xs text-muted-foreground">turned off</span>}
                  <span className="flex-1" />
                  {report.improve_available && canWrite && t.findings.length > 0 && (
                    <Button size="sm" variant="outline" onClick={() => void improve(t.id)} disabled={improving !== null}>
                      <SparklesIcon /> {improving === t.id ? "Asking Claude…" : "Improve with Claude"}
                    </Button>
                  )}
                </div>
                {t.findings.length === 0 ? (
                  <p className="text-sm text-muted-foreground">No problems found.</p>
                ) : (
                  <ul className="grid gap-1">
                    {t.findings.map((f, i) => (
                      <li key={i} className="text-sm">
                        <span className={f.severity === "warning" ? "font-medium text-warning" : "font-medium text-muted-foreground"}>
                          {f.severity === "warning" ? "Fix" : "Consider"}:
                        </span>{" "}
                        {f.message} <span className="text-muted-foreground">{f.hint}</span>
                      </li>
                    ))}
                  </ul>
                )}
                {s && (
                  <div className="grid gap-3 rounded-md border bg-muted/30 p-3">
                    <p className="text-xs text-muted-foreground">Suggested by {s.model}. Nothing changes until you apply it.</p>
                    <div className="grid gap-3 md:grid-cols-2">
                      <div className="space-y-1">
                        <p className="text-xs font-medium">Now</p>
                        <p className="rounded border bg-background p-2 text-sm whitespace-pre-wrap">{s.current.description || <em className="text-muted-foreground">no description</em>}</p>
                        {s.current.params.map((p) => (
                          <p key={p.name} className="text-xs">
                            <code>{p.name}</code>: {p.description || <em className="text-muted-foreground">none</em>}
                          </p>
                        ))}
                      </div>
                      <div className="space-y-1">
                        <p className="text-xs font-medium">Suggested</p>
                        <p className="rounded border border-primary/40 bg-background p-2 text-sm whitespace-pre-wrap">{s.suggested.description}</p>
                        {s.suggested.params.map((p) => (
                          <p key={p.name} className="text-xs">
                            <code>{p.name}</code>: {p.description}
                          </p>
                        ))}
                      </div>
                    </div>
                    <div className="flex gap-2">
                      <Button size="sm" onClick={() => void apply(t.id)}>
                        Apply
                      </Button>
                      <Button size="sm" variant="outline" onClick={() => setSuggestions(({ [t.id]: _, ...rest }) => rest)}>
                        Dismiss
                      </Button>
                    </div>
                  </div>
                )}
              </li>
            );
          })}
        </ul>
      )}
      {!report.improve_available && report.tools.some((t) => t.findings.length > 0) && (
        <Alert>
          <AlertDescription>Fix tools in their editors, or add an Anthropic key reference above to get suggested wording.</AlertDescription>
        </Alert>
      )}
    </div>
  );
}
