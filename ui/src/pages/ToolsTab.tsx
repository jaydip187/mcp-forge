import { useCallback, useEffect, useState } from "react";
import { DatabaseIcon, FileJsonIcon, KeyRoundIcon, PlusIcon, WrenchIcon } from "lucide-react";
import { toast } from "sonner";

import { api, ApiError } from "@/api";
import { EmptyState } from "@/components/EmptyState";
import { RequestLine } from "@/components/RequestLine";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { Switch } from "@/components/ui/switch";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { href } from "@/lib/router";
import { looksLikeWrite } from "@/lib/sqlForm";
import { HIDDEN_LABEL, hiddenReason } from "@/lib/visibility";
import type { ProjectSettings, Tool } from "@/types";

export function ToolsTab({ projectId, settings }: { projectId: string; settings: ProjectSettings }) {
  const [tools, setTools] = useState<Tool[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [connections, setConnections] = useState<Map<string, string>>(new Map());

  const load = useCallback(async () => {
    try {
      const [ts, cs] = await Promise.all([api.listTools(projectId), api.listConnections(projectId)]);
      setTools(ts);
      setConnections(new Map(cs.map((c) => [c.id, c.name])));
    } catch (e) {
      setError(e instanceof ApiError ? e.message : String(e));
    }
  }, [projectId]);
  useEffect(() => {
    void load();
  }, [load]);

  async function setEnabled(t: Tool, enabled: boolean) {
    setTools((ts) => ts?.map((x) => (x.id === t.id ? { ...x, enabled } : x)) ?? null);
    try {
      await api.updateTool(projectId, t.id, {
        name: t.name,
        description: t.description,
        kind: t.kind,
        enabled,
        tags: t.tags,
        response_shape: t.response_shape,
      });
      toast.success(enabled ? `${t.name} is now visible to agents` : `${t.name} is now hidden from agents`);
    } catch (e) {
      toast.error(e instanceof ApiError ? e.message : String(e));
      void load();
    }
  }

  if (error)
    return (
      <Alert variant="destructive">
        <AlertDescription>{error}</AlertDescription>
      </Alert>
    );
  if (!tools) return <Skeleton className="h-40 w-full" />;

  return (
    <div className="space-y-4">
      {tools.length > 0 && (
      <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
        <p className="text-sm text-muted-foreground">
          {`${tools.filter((t) => hiddenReason(t, settings) === null).length} of ${tools.length} tools visible to agents.`}
        </p>
        <div className="flex flex-wrap gap-2">
          <Button asChild variant="outline">
            <a href={href.import(projectId)}>
              <FileJsonIcon /> Import API spec
            </a>
          </Button>
          <Button asChild variant="outline">
            <a href={href.tool(projectId, null, "sql")}>
              <DatabaseIcon /> New SQL tool
            </a>
          </Button>
          <Button asChild>
            <a href={href.tool(projectId, null)}>
              <PlusIcon /> New HTTP tool
            </a>
          </Button>
        </div>
      </div>
      )}

      {tools.length === 0 ? (
        <EmptyState
          icon={WrenchIcon}
          title="Describe an HTTP request or a SQL query, and agents can call it"
          description="Say which parts the agent fills in, and test it right here before any agent sees it."
        >
            <Button asChild>
              <a href={href.tool(projectId, null)}>Create the first tool</a>
            </Button>
            <Button asChild variant="outline">
              <a href={href.import(projectId)}>Import from an API spec</a>
            </Button>
            <Button asChild variant="outline">
              <a href={href.tool(projectId, null, "sql")}>Add a SQL query</a>
            </Button>
        </EmptyState>
      ) : (
        <ul className="divide-y overflow-hidden rounded-xl border bg-card">
          {tools.map((t) => (
            <li key={t.id} className="flex items-center gap-4 px-4 py-3 hover:bg-muted/30">
              <a href={href.tool(projectId, t.id)} className="group min-w-0 flex-1 rounded-sm outline-none focus-visible:ring-2 focus-visible:ring-ring">
                <div className="flex items-center gap-2">
                  <span className="font-medium group-hover:underline">{t.name}</span>
                  {t.kind.type === "sql" && t.kind.mode.type === "query" && looksLikeWrite(t.kind.mode.sql) && (
                    <span className="rounded bg-destructive/10 px-1.5 py-0.5 text-xs font-medium text-destructive">writes</span>
                  )}
                  {t.kind.type === "http" && t.kind.auth_ref && (
                    <Tooltip>
                      <TooltipTrigger asChild>
                        <KeyRoundIcon className="size-3.5 text-muted-foreground" aria-label="Uses an auth config" />
                      </TooltipTrigger>
                      <TooltipContent>Uses an auth config</TooltipContent>
                    </Tooltip>
                  )}
                </div>
                {t.kind.type === "http" ? (
                  <RequestLine method={t.kind.method} baseUrl={t.kind.base_url} path={t.kind.path_template} size="sm" showBase={false} className="mt-0.5" />
                ) : (
                  <SqlLine
                    sql={t.kind.mode.type === "query" ? t.kind.mode.sql : t.kind.mode.type}
                    connection={connections.get(t.kind.connection_id)}
                  />
                )}
                {t.description && <p className="mt-1 line-clamp-1 text-sm text-muted-foreground">{t.description}</p>}
                {(() => {
                  const why = hiddenReason(t, settings);
                  return why && why !== "disabled" ? (
                    <p className="mt-1 text-xs text-warning">{HIDDEN_LABEL[why]}</p>
                  ) : null;
                })()}
              </a>
              <Switch
                checked={t.enabled}
                onCheckedChange={(v) => void setEnabled(t, v)}
                aria-label={`${t.name} visible to agents`}
              />
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

/** One-line summary of a SQL tool: connection and the start of the query. */
function SqlLine({ sql, connection }: { sql: string; connection: string | undefined }) {
  const oneLine = sql.replace(/\s+/g, " ").trim();
  return (
    <p className="mt-0.5 flex min-w-0 items-center gap-2 font-mono text-xs text-muted-foreground">
      <span className="shrink-0 rounded-[4px] bg-m-other/12 px-1.5 py-px text-[0.85em] font-semibold text-foreground">SQL</span>
      {connection && <span className="shrink-0">{connection}</span>}
      <span className="truncate">{oneLine}</span>
    </p>
  );
}
