import { useCallback, useEffect, useState } from "react";
import { ActivityIcon, RefreshCwIcon, XIcon } from "lucide-react";

import { api, ApiError } from "@/api";
import { EmptyState } from "@/components/EmptyState";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Sheet, SheetContent, SheetDescription, SheetHeader, SheetTitle } from "@/components/ui/sheet";
import { Skeleton } from "@/components/ui/skeleton";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { ms } from "@/lib/format";
import { cn } from "@/lib/utils";
import type { CallEntry, CallFilter, CallStatus, CallSummary, CallWindow } from "@/types";

const PAGE = 100;

const STATUS: Record<CallStatus, { label: string; variant: "secondary" | "destructive" | "outline" }> = {
  ok: { label: "OK", variant: "secondary" },
  error: { label: "Error", variant: "destructive" },
  unknown_tool: { label: "Unknown tool", variant: "outline" },
};

const WINDOWS: { value: CallWindow; label: string }[] = [
  { value: "24h", label: "Last 24 hours" },
  { value: "7d", label: "Last 7 days" },
  { value: "30d", label: "Last 30 days" },
];

function origin(c: CallEntry): string {
  return c.origin === "mcp" ? (c.client ?? "agent") : `test console${c.client ? ` · ${c.client}` : ""}`;
}

/** Every tool call, from agents and the test console: a summary, then the calls themselves. */
export function CallsTab({ projectId }: { projectId: string }) {
  const [range, setRange] = useState<CallWindow>("24h");
  const [summary, setSummary] = useState<CallSummary | null>(null);
  const [filter, setFilter] = useState<CallFilter>({});
  const [calls, setCalls] = useState<CallEntry[] | null>(null);
  const [more, setMore] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [open, setOpen] = useState<CallEntry | null>(null);

  const load = useCallback(async () => {
    try {
      const [s, page] = await Promise.all([api.callSummary(projectId, range), api.listCalls(projectId, filter)]);
      setSummary(s);
      setCalls(page);
      setMore(page.length === PAGE);
      setError(null);
    } catch (e) {
      setError(e instanceof ApiError ? e.message : String(e));
    }
  }, [projectId, range, filter]);
  useEffect(() => {
    void load();
  }, [load]);

  async function older() {
    const last = calls?.at(-1);
    if (!last) return;
    const page = await api.listCalls(projectId, filter, last.ts);
    setCalls([...(calls ?? []), ...page]);
    setMore(page.length === PAGE);
  }

  if (error)
    return (
      <Alert variant="destructive">
        <AlertDescription>{error}</AlertDescription>
      </Alert>
    );
  if (!summary || !calls) return <Skeleton className="h-64 w-full" />;

  const filtered = Object.values(filter).some(Boolean);
  if (summary.total === 0 && calls.length === 0 && !filtered)
    return (
      <div className="space-y-3">
        <Toolbar range={range} setRange={setRange} onRefresh={() => void load()} />
        <EmptyState
          icon={ActivityIcon}
          title="No calls yet"
          description="Calls appear here when an agent uses one of this project's tools, or when you run one in the test console."
        />
      </div>
    );

  return (
    <div className="space-y-6">
      <Toolbar range={range} setRange={setRange} onRefresh={() => void load()} />
      <Stats summary={summary} />
      <CallsChart summary={summary} />
      <ToolTable summary={summary} onPick={(tool) => setFilter({ ...filter, tool })} />

      <section className="space-y-3" aria-labelledby="calls-heading">
        <div className="flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
          <h2 id="calls-heading" className="text-sm font-medium">
            Calls
          </h2>
          <Filters filter={filter} setFilter={setFilter} tools={summary.tools.map((t) => t.tool_name)} />
        </div>
        {calls.length === 0 ? (
          <p className="rounded-xl border border-dashed px-4 py-8 text-center text-sm text-muted-foreground">No calls match these filters.</p>
        ) : (
          <div className="overflow-hidden rounded-xl border bg-card">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>When</TableHead>
                  <TableHead>Tool</TableHead>
                  <TableHead>Result</TableHead>
                  <TableHead className="text-right">Time</TableHead>
                  <TableHead className="hidden md:table-cell">From</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {calls.map((c) => (
                  <TableRow key={c.id} className="cursor-pointer" onClick={() => setOpen(c)}>
                    <TableCell className="whitespace-nowrap text-xs text-muted-foreground" title={c.ts}>
                      {new Date(c.ts).toLocaleString()}
                    </TableCell>
                    <TableCell>
                      <button type="button" className="font-mono text-sm hover:underline" onClick={() => setOpen(c)}>
                        {c.tool_name}
                      </button>
                    </TableCell>
                    <TableCell>
                      <div className="flex items-center gap-2">
                        <Badge variant={STATUS[c.status].variant}>{STATUS[c.status].label}</Badge>
                        {c.upstream_status !== null && <span className="font-mono text-xs text-muted-foreground">HTTP {c.upstream_status}</span>}
                      </div>
                    </TableCell>
                    <TableCell className="text-right font-mono text-xs">{ms(c.duration_ms)}</TableCell>
                    <TableCell className="hidden max-w-56 truncate text-xs text-muted-foreground md:table-cell">{origin(c)}</TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </div>
        )}
        {more && (
          <Button variant="outline" size="sm" onClick={() => void older()}>
            Load older calls
          </Button>
        )}
      </section>

      <CallSheet call={open} onClose={() => setOpen(null)} />
    </div>
  );
}

function Toolbar({ range, setRange, onRefresh }: { range: CallWindow; setRange: (w: CallWindow) => void; onRefresh: () => void }) {
  return (
    <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
      <p className="max-w-prose text-sm text-muted-foreground">
        Every tool call from agents and the test console. Argument values whose names look secret (password, token, key…) are
        stored as <code className="font-mono text-xs">[redacted]</code>.
      </p>
      <div className="flex shrink-0 gap-2">
        <Select value={range} onValueChange={(v) => setRange(v as CallWindow)}>
          <SelectTrigger size="sm" className="w-40" aria-label="Time range">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {WINDOWS.map((w) => (
              <SelectItem key={w.value} value={w.value}>
                {w.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <Button variant="outline" size="sm" onClick={onRefresh}>
          <RefreshCwIcon /> Refresh
        </Button>
      </div>
    </div>
  );
}

function Stats({ summary: s }: { summary: CallSummary }) {
  const rate = s.total ? (s.errors / s.total) * 100 : 0;
  const tiles = [
    { label: "Calls", value: s.total.toLocaleString() },
    { label: "Error rate", value: s.total ? `${rate < 10 ? rate.toFixed(1) : Math.round(rate)}%` : "–", hint: `${s.errors.toLocaleString()} failed` },
    { label: "Median time", value: ms(s.p50_ms) },
    { label: "95th percentile", value: ms(s.p95_ms) },
  ];
  return (
    <dl className="grid grid-cols-2 gap-3 lg:grid-cols-4">
      {tiles.map((t) => (
        <div key={t.label} className="rounded-xl border bg-card px-4 py-3">
          <dt className="text-xs text-muted-foreground">{t.label}</dt>
          <dd className="mt-1 font-mono text-2xl font-medium tracking-tight tabular-nums">{t.value}</dd>
          {t.hint && <dd className="text-xs text-muted-foreground">{t.hint}</dd>}
        </div>
      ))}
    </dl>
  );
}

function bucketLabel(start: string, bucket: CallSummary["bucket"]): string {
  const d = new Date(start);
  return bucket === "hour"
    ? d.toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" })
    : d.toLocaleDateString(undefined, { month: "short", day: "numeric", timeZone: "UTC" });
}

/** Calls per hour or day, OK and errors stacked. Hover a column for its numbers. */
function CallsChart({ summary: s }: { summary: CallSummary }) {
  const [hover, setHover] = useState<number | null>(null);
  const max = Math.max(1, ...s.buckets.map((b) => b.ok + b.errors));
  const h = hover === null ? null : s.buckets[hover];
  const ticks = [0, Math.floor((s.buckets.length - 1) / 2), s.buckets.length - 1];
  return (
    <figure className="rounded-xl border bg-card p-4">
      <figcaption className="mb-3 flex flex-wrap items-center justify-between gap-2">
        <span className="text-sm font-medium">Calls per {s.bucket}</span>
        <span className="flex items-center gap-3 text-xs text-muted-foreground">
          <span className="flex items-center gap-1.5">
            <span className="size-2.5 rounded-sm bg-chart-ok" aria-hidden="true" /> OK
          </span>
          <span className="flex items-center gap-1.5">
            <span className="size-2.5 rounded-sm bg-chart-error" aria-hidden="true" /> Errors
          </span>
        </span>
      </figcaption>
      <div className="relative">
        <div className="pointer-events-none absolute inset-x-0 top-0 border-t border-dashed border-border" aria-hidden="true" />
        <span className="pointer-events-none absolute -top-2 left-0 bg-card pr-1 font-mono text-[10px] text-muted-foreground tabular-nums">
          {max}
        </span>
        <div
          className="flex h-36 items-end gap-0.5 border-b border-border"
          role="img"
          aria-label={`${s.total} calls, ${s.errors} errors, over the ${WINDOWS.find((w) => w.value === s.window)?.label.toLowerCase()}`}
          onMouseLeave={() => setHover(null)}
        >
          {s.buckets.map((b, i) => {
            const total = b.ok + b.errors;
            return (
              <div
                key={b.start}
                className={cn("flex h-full flex-1 flex-col justify-end", hover === i && "bg-muted/60")}
                onMouseEnter={() => setHover(i)}
              >
                {total > 0 && (
                  <div className="flex flex-col gap-0.5" style={{ height: `${(total / max) * 100}%` }}>
                    {b.errors > 0 && <div className="min-h-0.5 rounded-t-[4px] bg-chart-error" style={{ flexGrow: b.errors }} />}
                    {b.ok > 0 && <div className={cn("min-h-0.5 bg-chart-ok", b.errors === 0 && "rounded-t-[4px]")} style={{ flexGrow: b.ok }} />}
                  </div>
                )}
              </div>
            );
          })}
        </div>
        {h && hover !== null && (
          <div
            className={cn(
              "pointer-events-none absolute -top-1 z-10 -translate-y-full rounded-md border bg-popover px-2.5 py-1.5 text-xs whitespace-nowrap shadow-md",
              // Near the edges, pin the tooltip so it stays inside the card.
              hover < s.buckets.length * 0.15 ? "" : hover > s.buckets.length * 0.85 ? "-translate-x-full" : "-translate-x-1/2",
            )}
            style={{
              left: `${((hover + (hover < s.buckets.length * 0.15 ? 0 : hover > s.buckets.length * 0.85 ? 1 : 0.5)) / s.buckets.length) * 100}%`,
            }}
          >
            <div className="font-medium">{bucketLabel(h.start, s.bucket)}</div>
            <div className="text-muted-foreground tabular-nums">
              {h.ok} OK · {h.errors} errors
            </div>
          </div>
        )}
      </div>
      <div className="relative mt-1.5 h-4 text-[10px] text-muted-foreground">
        {ticks.map((i, n) => (
          <span
            key={n}
            className={cn("absolute", n === 0 ? "left-0" : n === 2 ? "right-0" : "-translate-x-1/2")}
            style={n === 1 ? { left: `${((i + 0.5) / s.buckets.length) * 100}%` } : undefined}
          >
            {bucketLabel(s.buckets[i]?.start ?? s.since, s.bucket)}
          </span>
        ))}
      </div>
    </figure>
  );
}

function ToolTable({ summary: s, onPick }: { summary: CallSummary; onPick: (tool: string) => void }) {
  if (s.tools.length === 0) return null;
  return (
    <section className="space-y-3" aria-labelledby="tools-heading">
      <h2 id="tools-heading" className="text-sm font-medium">
        By tool
      </h2>
      <div className="overflow-hidden rounded-xl border bg-card">
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Tool</TableHead>
              <TableHead className="text-right">Calls</TableHead>
              <TableHead className="text-right">Errors</TableHead>
              <TableHead className="text-right">Median</TableHead>
              <TableHead className="text-right">p95</TableHead>
              <TableHead className="hidden text-right sm:table-cell">Last call</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {s.tools.map((t) => (
              <TableRow key={t.tool_name}>
                <TableCell>
                  <button
                    type="button"
                    className="font-mono text-sm hover:underline"
                    title="Show only this tool's calls"
                    onClick={() => onPick(t.tool_name)}
                  >
                    {t.tool_name}
                  </button>
                </TableCell>
                <TableCell className="text-right font-mono text-xs tabular-nums">{t.calls}</TableCell>
                <TableCell className={cn("text-right font-mono text-xs tabular-nums", t.errors > 0 && "text-destructive")}>
                  {t.errors}
                </TableCell>
                <TableCell className="text-right font-mono text-xs">{ms(t.p50_ms)}</TableCell>
                <TableCell className="text-right font-mono text-xs">{ms(t.p95_ms)}</TableCell>
                <TableCell className="hidden text-right text-xs text-muted-foreground sm:table-cell" title={t.last_call}>
                  {new Date(t.last_call).toLocaleString()}
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </div>
    </section>
  );
}

const ALL = "all";

function Filters({ filter, setFilter, tools }: { filter: CallFilter; setFilter: (f: CallFilter) => void; tools: string[] }) {
  const toolOptions = filter.tool && !tools.includes(filter.tool) ? [filter.tool, ...tools] : tools;
  const set = (k: keyof CallFilter) => (v: string) => setFilter({ ...filter, [k]: v === ALL ? undefined : v });
  return (
    <div className="flex flex-wrap items-center gap-2">
      <Select value={filter.tool ?? ALL} onValueChange={set("tool")}>
        <SelectTrigger size="sm" className="w-44" aria-label="Tool">
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value={ALL}>All tools</SelectItem>
          {toolOptions.map((t) => (
            <SelectItem key={t} value={t} className="font-mono">
              {t}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
      <Select value={filter.status ?? ALL} onValueChange={set("status")}>
        <SelectTrigger size="sm" className="w-36" aria-label="Result">
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value={ALL}>Any result</SelectItem>
          <SelectItem value="ok">OK</SelectItem>
          <SelectItem value="error">Error</SelectItem>
          <SelectItem value="unknown_tool">Unknown tool</SelectItem>
        </SelectContent>
      </Select>
      <Select value={filter.origin ?? ALL} onValueChange={set("origin")}>
        <SelectTrigger size="sm" className="w-36" aria-label="From">
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value={ALL}>Anywhere</SelectItem>
          <SelectItem value="mcp">Agents</SelectItem>
          <SelectItem value="console">Test console</SelectItem>
        </SelectContent>
      </Select>
      {Object.values(filter).some(Boolean) && (
        <Button variant="ghost" size="sm" onClick={() => setFilter({})}>
          <XIcon /> Clear
        </Button>
      )}
    </div>
  );
}

function CallSheet({ call: c, onClose }: { call: CallEntry | null; onClose: () => void }) {
  return (
    <Sheet open={c !== null} onOpenChange={(o) => !o && onClose()}>
      <SheetContent className="w-full overflow-y-auto sm:max-w-lg">
        {c && (
          <>
            <SheetHeader>
              <SheetTitle className="font-mono">{c.tool_name}</SheetTitle>
              <SheetDescription>{new Date(c.ts).toLocaleString()}</SheetDescription>
            </SheetHeader>
            <div className="space-y-5 px-4 pb-6 text-sm">
              <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-2">
                <dt className="text-muted-foreground">Result</dt>
                <dd>
                  <Badge variant={STATUS[c.status].variant}>{STATUS[c.status].label}</Badge>
                  {c.upstream_status !== null && <span className="ml-2 font-mono text-xs">HTTP {c.upstream_status}</span>}
                </dd>
                <dt className="text-muted-foreground">Time</dt>
                <dd className="font-mono text-xs">{ms(c.duration_ms)}</dd>
                <dt className="text-muted-foreground">Response</dt>
                <dd className="font-mono text-xs">{c.response_chars.toLocaleString()} characters</dd>
                <dt className="text-muted-foreground">From</dt>
                <dd className="text-xs">{origin(c)}</dd>
              </dl>
              <div className="space-y-1.5">
                <h3 className="text-xs font-medium text-muted-foreground">Arguments</h3>
                <pre className="max-h-72 overflow-auto rounded-lg border bg-muted/40 p-3 font-mono text-xs whitespace-pre-wrap break-all">
                  {typeof c.arguments === "string" ? c.arguments : JSON.stringify(c.arguments, null, 2)}
                </pre>
              </div>
              {c.error && (
                <div className="space-y-1.5">
                  <h3 className="text-xs font-medium text-muted-foreground">What the agent was told</h3>
                  <pre className="max-h-72 overflow-auto rounded-lg border border-destructive/30 bg-destructive/5 p-3 font-mono text-xs whitespace-pre-wrap break-words">
                    {c.error}
                  </pre>
                </div>
              )}
            </div>
          </>
        )}
      </SheetContent>
    </Sheet>
  );
}
