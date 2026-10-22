import { useEffect, useState, type FormEvent } from "react";
import { PlayIcon } from "lucide-react";

import { api, ApiError } from "@/api";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Textarea } from "@/components/ui/textarea";
import { buildArgs, prettyBody } from "@/lib/args";
import { schemaType } from "@/lib/template";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import type { Json, McpToolView, SqlTestRun, TestRun } from "@/types";

export function TestConsole({ projectId, toolId, version, dirty }: { projectId: string; toolId: string | null; version: string; dirty: boolean }) {
  const [view, setView] = useState<McpToolView | null>(null);
  const [values, setValues] = useState<Record<string, string>>({});
  const [argErrors, setArgErrors] = useState<Record<string, string>>({});
  const [run, setRun] = useState<TestRun | null>(null);
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState<string | null>(null);

  useEffect(() => {
    if (!toolId) return;
    api.toolMcpView(projectId, toolId).then(setView, (e) => setFailure(String(e)));
  }, [projectId, toolId, version]);

  if (!toolId)
    return (
      <Card>
        <CardHeader>
          <CardTitle>Test console</CardTitle>
          <CardDescription>Save the tool to try it here with real arguments.</CardDescription>
        </CardHeader>
      </Card>
    );

  const props = view?.inputSchema.properties ?? {};
  const required = new Set(view?.inputSchema.required ?? []);

  async function submit(e: FormEvent) {
    e.preventDefault();
    if (!toolId) return;
    const { args, errors } = buildArgs(props, values);
    setArgErrors(errors);
    if (Object.keys(errors).length) return;
    setBusy(true);
    setFailure(null);
    try {
      setRun(await api.testTool(projectId, toolId, args));
    } catch (err) {
      setFailure(err instanceof ApiError ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle>Test console</CardTitle>
        <CardDescription>
          {dirty ? "Runs the saved version. Save to test your latest changes." : "Runs the tool for real, the same way an agent would."}
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        <form onSubmit={submit} className="grid gap-3">
          {Object.keys(props).length === 0 && <p className="text-sm text-muted-foreground">This tool takes no arguments.</p>}
          {Object.entries(props).map(([name, schema]) => {
            const type = schemaType(schema);
            const id = `arg-${name}`;
            const desc = typeof schema.description === "string" ? schema.description : null;
            return (
              <div key={name} className="grid gap-1">
                <Label htmlFor={id} className="font-mono text-xs">
                  {name}
                  {required.has(name) && <span className="text-destructive"> *</span>}
                  <span className="ml-auto font-sans text-muted-foreground">{type}</span>
                </Label>
                {type === "boolean" ? (
                  <Select value={values[name] ?? "unset"} onValueChange={(v) => setValues({ ...values, [name]: v === "unset" ? "" : v })}>
                    <SelectTrigger id={id} className="w-full">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="unset">Not set</SelectItem>
                      <SelectItem value="true">true</SelectItem>
                      <SelectItem value="false">false</SelectItem>
                    </SelectContent>
                  </Select>
                ) : type === "object" || type === "array" || type === "custom" ? (
                  <Textarea id={id} value={values[name] ?? ""} onChange={(e) => setValues({ ...values, [name]: e.target.value })} className="min-h-20 font-mono text-xs" spellCheck={false} placeholder={type === "array" ? "[ ]" : "{ }"} aria-invalid={!!argErrors[name]} />
                ) : (
                  <Input id={id} value={values[name] ?? ""} onChange={(e) => setValues({ ...values, [name]: e.target.value })} className="font-mono" inputMode={type === "integer" || type === "number" ? "decimal" : undefined} aria-invalid={!!argErrors[name]} />
                )}
                {argErrors[name] ? <p className="text-xs text-destructive">{argErrors[name]}</p> : desc && <p className="text-xs text-muted-foreground">{desc}</p>}
              </div>
            );
          })}
          <Button type="submit" disabled={busy || !view}>
            <PlayIcon /> {busy ? "Running…" : "Run test"}
          </Button>
        </form>

        {failure && (
          <Alert variant="destructive">
            <AlertDescription>{failure}</AlertDescription>
          </Alert>
        )}
        {run && <RunResult run={run} />}
      </CardContent>
    </Card>
  );
}

const SQL_ERROR_LABEL: Record<NonNullable<SqlTestRun["error"]>["kind"], string> = {
  invalid_args: "Invalid arguments",
  rejected: "Query rejected",
  config: "Configuration error",
  connect: "Couldn't connect",
  timeout: "Timed out",
  database: "Database error",
};

function RunResult({ run }: { run: TestRun }) {
  if (run.kind === "sql") return <SqlRunResult run={run} />;
  const status = run.response ? (
    <Badge variant={run.response.status < 400 ? "secondary" : "destructive"} className="font-mono">
      {run.response.status_line}
    </Badge>
  ) : (
    <Badge variant="destructive">{run.error?.kind === "invalid_args" ? "Invalid arguments" : run.error?.kind === "config" ? "Configuration error" : "Request failed"}</Badge>
  );
  return (
    <section aria-label="Test result" className="space-y-2">
      <div className="flex items-center gap-2">
        {status}
        {run.shaped !== null && <Badge variant="outline">Shaped</Badge>}
      </div>
      <Tabs defaultValue="agent">
        <TabsList className="w-full">
          <TabsTrigger value="agent">Agent sees</TabsTrigger>
          <TabsTrigger value="response" disabled={!run.response}>
            Response
          </TabsTrigger>
          <TabsTrigger value="request" disabled={!run.request}>
            Request
          </TabsTrigger>
        </TabsList>
        <TabsContent value="agent" className="space-y-2">
          <p className="text-xs text-muted-foreground">{run.agent_view.is_error ? "Returned to the agent as an error." : "Returned to the agent as the tool result."}</p>
          <Output text={run.agent_view.text} />
        </TabsContent>
        {run.response && (
          <TabsContent value="response" className="space-y-2">
            <p className="text-xs text-muted-foreground">
              {run.response.content_type ?? "No content type"}
              {run.response.truncated && ". Cut off at the size limit"}
            </p>
            <Output text={prettyBody(run.response.body, run.response.content_type)} />
            <p className="text-xs text-muted-foreground">
              {run.shaped === null
                ? "Nothing was shaped: the agent gets this body as is."
                : "Shaped by the response shape, null stripping or the size limit. The agent gets the version under “Agent sees”."}
            </p>
          </TabsContent>
        )}
        {run.request && (
          <TabsContent value="request">
            <Output text={`${run.request.method} ${run.request.url}`} />
            <p className="mt-2 text-xs text-muted-foreground">Secrets are shown as [REDACTED].</p>
          </TabsContent>
        )}
      </Tabs>
    </section>
  );
}

function SqlRunResult({ run }: { run: SqlTestRun }) {
  const r = run.response;
  return (
    <section aria-label="Test result" className="space-y-2">
      <div className="flex flex-wrap items-center gap-2">
        {r ? (
          <Badge variant="secondary" className="font-mono">
            {r.row_count} {r.row_count === 1 ? "row" : "rows"}
            {r.truncated && " (limit reached)"} · {r.elapsed_ms} ms
          </Badge>
        ) : (
          <Badge variant="destructive">{run.error ? SQL_ERROR_LABEL[run.error.kind] : "Failed"}</Badge>
        )}
        {r && r.masked_columns.length > 0 && <Badge variant="outline">Masked: {r.masked_columns.join(", ")}</Badge>}
        {run.shaped !== null && <Badge variant="outline">Shaped</Badge>}
      </div>
      <Tabs defaultValue="agent">
        <TabsList className="w-full">
          <TabsTrigger value="agent">Agent sees</TabsTrigger>
          <TabsTrigger value="rows" disabled={!r}>
            Rows
          </TabsTrigger>
          <TabsTrigger value="query" disabled={!run.request}>
            Query
          </TabsTrigger>
        </TabsList>
        <TabsContent value="agent" className="space-y-2">
          <p className="text-xs text-muted-foreground">{run.agent_view.is_error ? "Returned to the agent as an error." : "Returned to the agent as the tool result."}</p>
          <Output text={run.agent_view.is_error ? run.agent_view.text : prettyBody(run.agent_view.text, "application/json")} />
        </TabsContent>
        {r && (
          <TabsContent value="rows">
            {r.columns.length === 0 ? (
              <p className="text-sm text-muted-foreground">The query returned no columns.</p>
            ) : (
              <div className="max-h-96 overflow-auto rounded-md border">
                <Table>
                  <TableHeader>
                    <TableRow>
                      {r.columns.map((c, i) => (
                        <TableHead key={i} className="font-mono text-xs">
                          {c}
                        </TableHead>
                      ))}
                    </TableRow>
                  </TableHeader>
                  <TableBody>
                    {r.rows.map((row, i) => (
                      <TableRow key={i}>
                        {row.map((v, j) => (
                          <TableCell key={j} className="max-w-48 truncate font-mono text-xs" title={cellText(v)}>
                            {v === null ? <span className="text-muted-foreground">NULL</span> : cellText(v)}
                          </TableCell>
                        ))}
                      </TableRow>
                    ))}
                  </TableBody>
                </Table>
              </div>
            )}
            {r.rows.length === 0 && r.columns.length > 0 && <p className="mt-2 text-sm text-muted-foreground">No rows matched.</p>}
          </TabsContent>
        )}
        {run.request && (
          <TabsContent value="query" className="space-y-2">
            <Output text={run.request.sql} />
            {run.request.params.length > 0 && (
              <>
                <p className="text-xs text-muted-foreground">Values sent alongside the SQL, in order:</p>
                <Output text={run.request.params.map((v, i) => `${i + 1}: ${JSON.stringify(v)}`).join("\n")} />
              </>
            )}
          </TabsContent>
        )}
      </Tabs>
      {run.error && run.error.kind !== "invalid_args" && <p className="text-xs text-muted-foreground">{run.error.message}</p>}
    </section>
  );
}

function cellText(v: Json): string {
  return typeof v === "string" ? v : JSON.stringify(v);
}

function Output({ text }: { text: string }) {
  return <pre className="max-h-96 overflow-auto rounded-md border bg-muted/60 p-3 font-mono text-xs leading-relaxed whitespace-pre-wrap break-all">{text || " "}</pre>;
}
