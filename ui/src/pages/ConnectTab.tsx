import { CopyBlock } from "@/components/CopyBlock";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { buildSnippets } from "@/lib/snippets";
import type { Project, ServerInfo } from "@/types";

export function mcpUrl(slug: string): string {
  return `${window.location.origin}/mcp/${slug}`;
}

export function ConnectTab({ project, info }: { project: Project; info: ServerInfo }) {
  const snippets = buildSnippets(project.slug, mcpUrl(project.slug), info);
  return (
    <div className="space-y-6">
      <p className="max-w-prose text-sm text-muted-foreground">
        Use HTTP when this server is running. Use stdio when you'd rather the client start mcp-forge for you.
        Both serve the same tools.
      </p>
      {info.auth_required && (
        <Alert>
          <AlertTitle>This server needs the admin token</AlertTitle>
          <AlertDescription>
            Replace <code>&lt;your-admin-token&gt;</code> in the HTTP snippets with the token you started the server with.
          </AlertDescription>
        </Alert>
      )}
      <div className="grid gap-6 xl:grid-cols-2">
        {snippets.map((s) => (
          <section key={s.id} className="min-w-0 space-y-2">
            <div>
              <h3 className="text-sm font-medium">{s.title}</h3>
              <p className="text-xs text-muted-foreground">{s.hint}</p>
            </div>
            <CopyBlock code={s.code} label={s.title} />
          </section>
        ))}
      </div>
    </div>
  );
}
