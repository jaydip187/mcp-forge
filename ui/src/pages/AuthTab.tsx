import { useCallback, useEffect, useState } from "react";
import { KeyRoundIcon, LinkIcon, PlusIcon, UnlinkIcon } from "lucide-react";
import { toast } from "sonner";

import { api, ApiError } from "@/api";
import { CopyBlock } from "@/components/CopyBlock";
import { EmptyState } from "@/components/EmptyState";
import { checkError, StepDialog, type FormStep } from "@/components/StepDialog";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { fieldError } from "@/lib/projects";
import type { AuthConfig, AuthConfigInput, AuthKind, OAuthClientAuth, OAuthStatus } from "@/types";

/** Where the provider sends the browser back after Connect: register it there. */
export function oauthRedirectUri(origin = window.location.origin): string {
  return `${origin}/oauth/callback`;
}

function host(url: string): string {
  try {
    return new URL(url).host;
  } catch {
    return url;
  }
}

export function describeAuth(kind: AuthKind): string {
  switch (kind.type) {
    case "bearer":
      return "Bearer token in the Authorization header";
    case "basic":
      return `Basic auth as ${kind.username}`;
    case "api_key":
      return kind.location === "header" ? `API key in header ${kind.name}` : `API key in query parameter ${kind.name}`;
    case "oauth2_client_credentials":
      return `OAuth2 client credentials from ${host(kind.token_url)}`;
    case "oauth2_authorization_code":
      return `OAuth2 sign-in with ${host(kind.auth_url)}`;
  }
}

const EMPTY: AuthConfigInput = { name: "", kind: { type: "bearer" }, secret_ref: "env:" };

export function AuthTab({ projectId }: { projectId: string }) {
  const [configs, setConfigs] = useState<AuthConfig[] | null>(null);
  const [oauth, setOauth] = useState<OAuthStatus[]>([]);
  const [editing, setEditing] = useState<AuthConfig | "new" | null>(null);
  const [confirmDelete, setConfirmDelete] = useState<string | null>(null);

  const load = useCallback(async () => {
    const [list, status] = await Promise.all([api.listAuth(projectId), api.oauthStatus(projectId)]);
    setConfigs(list);
    setOauth(status);
  }, [projectId]);

  async function connect(c: AuthConfig) {
    try {
      const { authorize_url } = await api.oauthStart(projectId, c.id);
      // The provider sends the browser back to /oauth/callback, which returns here.
      window.location.assign(authorize_url);
    } catch (e) {
      toast.error(e instanceof ApiError ? e.message : String(e));
    }
  }

  async function disconnect(c: AuthConfig) {
    try {
      await api.oauthDisconnect(projectId, c.id);
      toast.success(`Disconnected ${c.name}`);
      await load();
    } catch (e) {
      toast.error(e instanceof ApiError ? e.message : String(e));
    }
  }
  useEffect(() => {
    void load();
  }, [load]);

  async function remove(c: AuthConfig) {
    try {
      await api.deleteAuth(projectId, c.id);
      toast.success(`Deleted ${c.name}`);
      setConfirmDelete(null);
      if (editing !== "new" && editing?.id === c.id) setEditing(null);
      await load();
    } catch (e) {
      toast.error(e instanceof ApiError ? e.message : String(e));
    }
  }

  return (
    <div className="space-y-4">
      <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
        <p className="max-w-prose text-sm text-muted-foreground">
          An auth config says how to sign requests and where the secret lives. mcp-forge stores only the
          reference and reads the secret each time a tool runs.
        </p>
        <Button className="shrink-0" onClick={() => setEditing("new")}>
          <PlusIcon /> Add auth config
        </Button>
      </div>
      {!configs ? (
        <Skeleton className="h-24 w-full" />
      ) : configs.length === 0 ? (
        <EmptyState
          icon={KeyRoundIcon}
          title="No auth configs yet"
          description="Add one if your API needs a key or token. Tools then sign their requests with it."
        >
          <Button onClick={() => setEditing("new")}>Add an auth config</Button>
        </EmptyState>
      ) : (
        <ul className="divide-y overflow-hidden rounded-xl border bg-card">
          {configs.map((c) => (
            <li key={c.id} className="flex flex-wrap items-center gap-3 px-4 py-3">
              <div className="min-w-0 flex-1">
                <p className="font-medium">{c.name}</p>
                <p className="text-sm text-muted-foreground">{describeAuth(c.kind)}</p>
                <p className="font-mono text-xs text-muted-foreground">{c.secret_ref}</p>
                {c.kind.type === "oauth2_authorization_code" && (
                  <ConnectionLine status={oauth.find((o) => o.auth_config_id === c.id)} />
                )}
              </div>
              {c.kind.type === "oauth2_authorization_code" && confirmDelete !== c.id && (
                <div className="flex gap-2">
                  {oauth.some((o) => o.auth_config_id === c.id) ? (
                    <>
                      <Button size="sm" variant="outline" onClick={() => void connect(c)}>
                        <LinkIcon /> Reconnect
                      </Button>
                      <Button size="sm" variant="ghost" onClick={() => void disconnect(c)}>
                        <UnlinkIcon /> Disconnect
                      </Button>
                    </>
                  ) : (
                    <Button size="sm" onClick={() => void connect(c)}>
                      <LinkIcon /> Connect
                    </Button>
                  )}
                </div>
              )}
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

      {editing && (
        <AuthForm
          key={editing === "new" ? "new" : editing.id}
          projectId={projectId}
          existing={editing === "new" ? null : editing}
          onDone={async () => {
            setEditing(null);
            await load();
          }}
          onClose={() => setEditing(null)}
        />
      )}
    </div>
  );
}

function ConnectionLine({ status }: { status: OAuthStatus | undefined }) {
  if (!status)
    return <p className="mt-1 text-xs font-medium text-warning">Not connected: tools using it fail until someone connects it.</p>;
  return (
    <p className="mt-1 text-xs text-muted-foreground">
      Connected {new Date(status.connected_at).toLocaleString()}
      {status.connected_by && ` by ${status.connected_by}`}
      {status.can_refresh ? " · refreshes itself" : " · no refresh token: reconnect when it expires"}
    </p>
  );
}

function kindFor(type: AuthKind["type"]): AuthKind {
  switch (type) {
    case "bearer":
      return { type };
    case "basic":
      return { type, username: "" };
    case "api_key":
      return { type, location: "header", name: "X-API-Key" };
    case "oauth2_client_credentials":
      return { type, token_url: "", client_id: "", scopes: [], audience: null, client_auth: "body" };
    case "oauth2_authorization_code":
      return { type, auth_url: "", token_url: "", client_id: "", scopes: [], client_auth: "body" };
  }
}

function AuthForm({
  projectId,
  existing,
  onDone,
  onClose,
}: {
  projectId: string;
  existing: AuthConfig | null;
  onDone: () => Promise<void>;
  onClose: () => void;
}) {
  const [draft, setDraft] = useState<AuthConfigInput>(existing ?? EMPTY);
  const [scopesText, setScopesText] = useState(() => ("scopes" in draft.kind ? draft.kind.scopes.join(" ") : ""));
  const [err, setErr] = useState<unknown>(null);

  function setType(type: AuthKind["type"]) {
    setDraft((d) => ({ ...d, kind: kindFor(type) }));
    setScopesText("");
  }

  function buildInput(): AuthConfigInput {
    if (!("scopes" in draft.kind)) return draft;
    const scopes = scopesText.split(/[\s,]+/).filter(Boolean);
    const kind = draft.kind.type === "oauth2_client_credentials" ? { ...draft.kind, scopes, audience: draft.kind.audience?.trim() || null } : { ...draft.kind, scopes };
    return { ...draft, kind };
  }

  async function submit() {
    const input = buildInput();
    try {
      if (existing) await api.updateAuth(projectId, existing.id, input);
      else await api.createAuth(projectId, input);
      toast.success(existing ? `Saved ${draft.name}` : `Added ${draft.name}`);
      setErr(null);
      await onDone();
    } catch (e) {
      setErr(e);
    }
  }

  const k = draft.kind;
  const steps: FormStep[] = [
    {
      title: "Name and type",
      description: "What tools call this config, and how the API expects the secret.",
      fields: ["name"],
      content: (
        <>
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
                <SelectItem value="oauth2_authorization_code">OAuth2: sign in (authorization code)</SelectItem>
                <SelectItem value="oauth2_client_credentials">OAuth2: client credentials</SelectItem>
              </SelectContent>
            </Select>
          </div>
        </>
      ),
    },
    {
      title: "Secret",
      description: "Where the secret lives. The secret itself never goes into mcp-forge.",
      fields: ["kind", "secret_ref"],
      content: (
        <>
          {(k.type === "oauth2_client_credentials" || k.type === "oauth2_authorization_code") && (
            <OAuthFields
              kind={k}
              onChange={(kind) => setDraft({ ...draft, kind })}
              scopesText={scopesText}
              onScopes={setScopesText}
              err={err}
            />
          )}
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
            <Label htmlFor="a-ref">
              {k.type === "basic" ? "Password reference" : "scopes" in k ? "Client secret reference" : "Secret reference"}
            </Label>
            <Input id="a-ref" className="font-mono" value={draft.secret_ref} onChange={(e) => setDraft({ ...draft, secret_ref: e.target.value })} placeholder="env:GITHUB_TOKEN" spellCheck={false} autoComplete="off" />
            <p className="text-xs text-muted-foreground">
              <code>env:NAME</code> reads an environment variable of the mcp-forge process. <code>keyring:NAME</code> reads
              your OS keychain (service <code>mcp-forge</code>, account <code>NAME</code>).
            </p>
          </div>
        </>
      ),
    },
  ];

  return (
    <StepDialog
      title={existing ? `Edit ${existing.name}` : "Add auth config"}
      steps={steps}
      errorFields={err instanceof ApiError ? err.fields.map((f) => f.field) : []}
      banner={
        <>
          {err instanceof ApiError && err.fields.length === 0 && (
            <Alert variant="destructive">
              <AlertDescription>
                {err.status === 400 && err.message.includes("secret_ref")
                  ? "That looks like a secret, not a reference. Use env:NAME or keyring:NAME."
                  : err.message}
              </AlertDescription>
            </Alert>
          )}
        </>
      }
      submitLabel={existing ? "Save auth config" : "Add auth config"}
      onSubmit={submit}
      onClose={onClose}
      check={() => api.checkAuth(projectId, buildInput(), existing?.id)}
      showErrors={(found) => setErr(checkError(found))}
    />
  );
}

type OAuthKind = Extract<AuthKind, { type: "oauth2_client_credentials" | "oauth2_authorization_code" }>;

function OAuthFields({
  kind: k,
  onChange,
  scopesText,
  onScopes,
  err,
}: {
  kind: OAuthKind;
  onChange: (k: OAuthKind) => void;
  scopesText: string;
  onScopes: (s: string) => void;
  err: unknown;
}) {
  const field = (id: string, label: string, name: string, value: string, set: (v: string) => void, placeholder: string) => (
    <div className="grid gap-1.5">
      <Label htmlFor={id}>{label}</Label>
      <Input id={id} className="font-mono" value={value} onChange={(e) => set(e.target.value)} placeholder={placeholder} spellCheck={false} aria-invalid={!!fieldError(err, name)} />
      {fieldError(err, name) && <p className="text-xs text-destructive">{fieldError(err, name)}</p>}
    </div>
  );
  return (
    <>
      {k.type === "oauth2_authorization_code" &&
        field("a-authurl", "Authorization URL", "kind.auth_url", k.auth_url, (v) => onChange({ ...k, auth_url: v }), "https://accounts.example.com/o/oauth2/auth")}
      {field("a-tokenurl", "Token URL", "kind.token_url", k.token_url, (v) => onChange({ ...k, token_url: v }), "https://oauth2.example.com/token")}
      {field("a-clientid", "Client ID", "kind.client_id", k.client_id, (v) => onChange({ ...k, client_id: v }), "1234.apps.example.com")}
      <div className="grid gap-1.5">
        <Label htmlFor="a-scopes">Scopes</Label>
        <Input id="a-scopes" className="font-mono" value={scopesText} onChange={(e) => onScopes(e.target.value)} placeholder="read:users email" spellCheck={false} />
        <p className="text-xs text-muted-foreground">Separated by spaces. Leave empty for the provider's default.</p>
      </div>
      {k.type === "oauth2_client_credentials" &&
        field("a-aud", "Audience (optional)", "kind.audience", k.audience ?? "", (v) => onChange({ ...k, audience: v }), "https://api.example.com")}
      <div className="grid gap-1.5">
        <Label htmlFor="a-clientauth">Send the client secret</Label>
        <Select value={k.client_auth} onValueChange={(v) => onChange({ ...k, client_auth: v as OAuthClientAuth })}>
          <SelectTrigger id="a-clientauth" className="w-full">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="body">In the request body (most providers)</SelectItem>
            <SelectItem value="basic">As HTTP Basic auth</SelectItem>
          </SelectContent>
        </Select>
      </div>
      {k.type === "oauth2_authorization_code" && (
        <div className="grid gap-1.5">
          <Label>Redirect URL to register with the provider</Label>
          <CopyBlock code={oauthRedirectUri()} label="Copy redirect URL" inline />
          <p className="text-xs text-muted-foreground">
            After saving, click Connect in the list and sign in. mcp-forge keeps the tokens (encrypted) and refreshes them.
          </p>
        </div>
      )}
    </>
  );
}
