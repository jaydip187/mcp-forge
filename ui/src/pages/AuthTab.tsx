import { useCallback, useEffect, useState, type FormEvent } from "react";
import { KeyRoundIcon } from "lucide-react";
import { toast } from "sonner";

import { api, ApiError } from "@/api";
import { EmptyState } from "@/components/EmptyState";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { fieldError } from "@/lib/projects";
import type { AuthConfig, AuthConfigInput, AuthKind } from "@/types";

export function describeAuth(kind: AuthKind): string {
  switch (kind.type) {
    case "bearer":
      return "Bearer token in the Authorization header";
    case "basic":
      return `Basic auth as ${kind.username}`;
    case "api_key":
      return kind.location === "header" ? `API key in header ${kind.name}` : `API key in query parameter ${kind.name}`;
  }
}

const EMPTY: AuthConfigInput = { name: "", kind: { type: "bearer" }, secret_ref: "env:" };

export function AuthTab({ projectId }: { projectId: string }) {
  const [configs, setConfigs] = useState<AuthConfig[] | null>(null);
  const [editing, setEditing] = useState<AuthConfig | null>(null);
  const [confirmDelete, setConfirmDelete] = useState<string | null>(null);

  const load = useCallback(async () => {
    setConfigs(await api.listAuth(projectId));
  }, [projectId]);
  useEffect(() => {
    void load();
  }, [load]);

  async function remove(c: AuthConfig) {
    try {
      await api.deleteAuth(projectId, c.id);
      toast.success(`Deleted ${c.name}`);
      setConfirmDelete(null);
      if (editing?.id === c.id) setEditing(null);
      await load();
    } catch (e) {
      toast.error(e instanceof ApiError ? e.message : String(e));
    }
  }

  return (
    <div className="grid items-start gap-6 lg:grid-cols-[minmax(0,1fr)_22rem]">
      <section className="min-w-0 space-y-3">
        <p className="max-w-prose text-sm text-muted-foreground">
          An auth config says how to sign requests and where the secret lives. mcp-forge stores only the
          reference and reads the secret each time a tool runs.
        </p>
        {!configs ? (
          <Skeleton className="h-24 w-full" />
        ) : configs.length === 0 ? (
          <EmptyState
            icon={KeyRoundIcon}
            title="No auth configs yet"
            description="Add one with the form if your API needs a key or token. Tools then sign their requests with it."
          />
        ) : (
          <ul className="divide-y overflow-hidden rounded-xl border bg-card">
            {configs.map((c) => (
              <li key={c.id} className="flex flex-wrap items-center gap-3 px-4 py-3">
                <div className="min-w-0 flex-1">
                  <p className="font-medium">{c.name}</p>
                  <p className="text-sm text-muted-foreground">{describeAuth(c.kind)}</p>
                  <p className="font-mono text-xs text-muted-foreground">{c.secret_ref}</p>
                </div>
                {confirmDelete === c.id ? (
                  <div className="flex gap-2">
                    <Button size="sm" variant="destructive" onClick={() => void remove(c)}>
                      Delete
                    </Button>
                    <Button size="sm" variant="outline" onClick={() => setConfirmDelete(null)}>
                      Keep
                    </Button>
                  </div>
                ) : (
                  <div className="flex gap-2">
                    <Button size="sm" variant="outline" onClick={() => setEditing(c)}>
                      Edit
                    </Button>
                    <Button size="sm" variant="ghost" className="text-destructive" onClick={() => setConfirmDelete(c.id)}>
                      Delete
                    </Button>
                  </div>
                )}
              </li>
            ))}
          </ul>
        )}
      </section>

      <AuthForm
        key={editing?.id ?? "new"}
        projectId={projectId}
        existing={editing}
        onDone={async () => {
          setEditing(null);
          await load();
        }}
      />
    </div>
  );
}

function AuthForm({ projectId, existing, onDone }: { projectId: string; existing: AuthConfig | null; onDone: () => Promise<void> }) {
  const [draft, setDraft] = useState<AuthConfigInput>(existing ?? EMPTY);
  const [err, setErr] = useState<unknown>(null);

  function setType(type: AuthKind["type"]) {
    const kind: AuthKind =
      type === "bearer" ? { type } : type === "basic" ? { type, username: "" } : { type, location: "header", name: "X-API-Key" };
    setDraft((d) => ({ ...d, kind }));
  }

  async function submit(e: FormEvent) {
    e.preventDefault();
    try {
      if (existing) await api.updateAuth(projectId, existing.id, draft);
      else await api.createAuth(projectId, draft);
      toast.success(existing ? `Saved ${draft.name}` : `Added ${draft.name}`);
      if (!existing) setDraft(EMPTY);
      setErr(null);
      await onDone();
    } catch (e) {
      setErr(e);
    }
  }

  const k = draft.kind;
  return (
    <Card className="h-fit">
      <CardHeader>
        <CardTitle>{existing ? `Edit ${existing.name}` : "Add auth config"}</CardTitle>
        <CardDescription>The secret itself never goes into mcp-forge.</CardDescription>
      </CardHeader>
      <CardContent>
        <form onSubmit={submit} className="grid gap-4">
          <div className="grid gap-1.5">
            <Label htmlFor="a-name">Name</Label>
            <Input id="a-name" value={draft.name} onChange={(e) => setDraft({ ...draft, name: e.target.value })} placeholder="github-token" required aria-invalid={!!fieldError(err, "name")} />
            {fieldError(err, "name") && <p className="text-xs text-destructive">{fieldError(err, "name")}</p>}
          </div>
          <div className="grid gap-1.5">
            <Label htmlFor="a-type">How the secret is sent</Label>
            <Select value={k.type} onValueChange={(v) => setType(v as AuthKind["type"])}>
              <SelectTrigger id="a-type" className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="bearer">Bearer token</SelectItem>
                <SelectItem value="api_key">API key</SelectItem>
                <SelectItem value="basic">Basic auth</SelectItem>
              </SelectContent>
            </Select>
          </div>
          {k.type === "api_key" && (
            <div className="grid grid-cols-[7rem_1fr] gap-2">
              <div className="grid gap-1.5">
                <Label htmlFor="a-loc">In</Label>
                <Select value={k.location} onValueChange={(v) => setDraft({ ...draft, kind: { ...k, location: v as "header" | "query" } })}>
                  <SelectTrigger id="a-loc" className="w-full">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="header">Header</SelectItem>
                    <SelectItem value="query">Query</SelectItem>
                  </SelectContent>
                </Select>
              </div>
              <div className="grid gap-1.5">
                <Label htmlFor="a-keyname">{k.location === "header" ? "Header name" : "Parameter name"}</Label>
                <Input id="a-keyname" className="font-mono" value={k.name} onChange={(e) => setDraft({ ...draft, kind: { ...k, name: e.target.value } })} aria-invalid={!!fieldError(err, "kind.name")} />
              </div>
              {fieldError(err, "kind.name") && <p className="col-span-2 text-xs text-destructive">{fieldError(err, "kind.name")}</p>}
            </div>
          )}
          {k.type === "basic" && (
            <div className="grid gap-1.5">
              <Label htmlFor="a-user">Username</Label>
              <Input id="a-user" value={k.username} onChange={(e) => setDraft({ ...draft, kind: { ...k, username: e.target.value } })} aria-invalid={!!fieldError(err, "kind.username")} />
              {fieldError(err, "kind.username") && <p className="text-xs text-destructive">{fieldError(err, "kind.username")}</p>}
            </div>
          )}
          <div className="grid gap-1.5">
            <Label htmlFor="a-ref">{k.type === "basic" ? "Password reference" : "Secret reference"}</Label>
            <Input id="a-ref" className="font-mono" value={draft.secret_ref} onChange={(e) => setDraft({ ...draft, secret_ref: e.target.value })} placeholder="env:GITHUB_TOKEN" spellCheck={false} autoComplete="off" />
            <p className="text-xs text-muted-foreground">
              <code>env:NAME</code> reads an environment variable of the mcp-forge process. <code>keyring:NAME</code> reads
              your OS keychain (service <code>mcp-forge</code>, account <code>NAME</code>).
            </p>
          </div>
          {err instanceof ApiError && err.fields.length === 0 && (
            <Alert variant="destructive">
              <AlertDescription>
                {err.status === 400 && err.message.includes("secret_ref")
                  ? "That looks like a secret, not a reference. Use env:NAME or keyring:NAME."
                  : err.message}
              </AlertDescription>
            </Alert>
          )}
          <div className="flex gap-2">
            <Button type="submit">{existing ? "Save auth config" : "Add auth config"}</Button>
            {existing && (
              <Button type="button" variant="ghost" onClick={() => void onDone()}>
                Cancel
              </Button>
            )}
          </div>
        </form>
      </CardContent>
    </Card>
  );
}
