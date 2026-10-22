import { useEffect, useState } from "react";
import { FileJsonIcon, RefreshCwIcon } from "lucide-react";

import { api, ApiError } from "@/api";
import { EmptyState } from "@/components/EmptyState";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { href } from "@/lib/router";
import type { SpecRecord, Tool } from "@/types";

export function SpecsTab({ projectId }: { projectId: string }) {
  const [specs, setSpecs] = useState<SpecRecord[] | null>(null);
  const [tools, setTools] = useState<Tool[]>([]);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    Promise.all([api.listSpecs(projectId), api.listTools(projectId)]).then(
      ([s, t]) => {
        setSpecs(s);
        setTools(t);
      },
      (e) => setError(e instanceof ApiError ? e.message : String(e)),
    );
  }, [projectId]);

  if (error)
    return (
      <Alert variant="destructive">
        <AlertDescription>{error}</AlertDescription>
      </Alert>
    );
  if (!specs) return <Skeleton className="h-24 w-full" />;

  return (
    <div className="space-y-4">
      <p className="max-w-prose text-sm text-muted-foreground">
        Specs you imported tools from. Re-import a newer version to update those tools. Anything you edited by hand is kept.
      </p>
      {specs.length === 0 ? (
        <EmptyState
          icon={FileJsonIcon}
          title="No imported specs yet"
          description="Import an OpenAPI or Swagger spec to create tools from its operations, and re-import it later to pick up changes."
        >
          <Button asChild>
            <a href={href.import(projectId)}>Import a spec</a>
          </Button>
        </EmptyState>
      ) : (
        <ul className="divide-y overflow-hidden rounded-xl border bg-card">
          {specs.map((s) => {
            const count = tools.filter((t) => t.source.type !== "manual" && t.source.spec_id === s.id).length;
            return (
              <li key={s.id} className="flex flex-wrap items-center gap-4 px-4 py-3">
                <div className="min-w-0 flex-1">
                  <p className="font-medium">
                    {s.title} {s.version && <span className="font-normal text-muted-foreground">v{s.version}</span>}
                  </p>
                  <p className="text-sm text-muted-foreground">
                    {s.kind === "swagger2" ? "Swagger 2.0" : "OpenAPI 3"}, {count} {count === 1 ? "tool" : "tools"}.{" "}
                    {s.updated_at
                      ? `Re-imported ${new Date(s.updated_at).toLocaleString()}.`
                      : `Imported ${new Date(s.created_at).toLocaleString()}.`}
                  </p>
                  {(s.source_url || s.filename) && (
                    <p className="truncate font-mono text-xs text-muted-foreground">{s.source_url ?? s.filename}</p>
                  )}
                </div>
                <Button asChild variant="outline">
                  <a href={href.reimport(projectId, s.id)}>
                    <RefreshCwIcon /> Re-import
                  </a>
                </Button>
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}
