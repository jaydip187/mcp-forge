import { useCallback, useEffect, useState } from "react";
import { KeyRoundIcon, PlusIcon, TriangleAlertIcon } from "lucide-react";
import { toast } from "sonner";

import { api, ApiError } from "@/api";
import { CopyBlock } from "@/components/CopyBlock";
import { StepDialog } from "@/components/StepDialog";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { WriteGuard } from "@/lib/permissions";
import { fieldError } from "@/lib/projects";
import { buildSnippets } from "@/lib/snippets";
import type { ApiKey, ConnectedApp, Project, ServerInfo, Tool } from "@/types";

export function mcpUrl(slug: string): string {
  return `${window.location.origin}/mcp/${slug}`;
}

/** Expiry choices for new keys, in days (null = never). */
const EXPIRY: { value: string; label: string; days: number | null }[] = [
  { value: "never", label: "Never", days: null },
  { value: "30", label: "In 30 days", days: 30 },
  { value: "90", label: "In 90 days", days: 90 },
  { value: "365", label: "In a year", days: 365 },
];

export function ConnectTab({ project, info, onSaved }: { project: Project; info: ServerInfo; onSaved: (p: Project) => void }) {
  const [keys, setKeys] = useState<ApiKey[] | null>(null);
  const loadKeys = useCallback(async () => setKeys(await api.listKeys(project.id)), [project.id]);
  useEffect(() => {
    void loadKeys();
  }, [loadKeys]);

  const useKey = project.settings.require_key || (keys?.length ?? 0) > 0;
  const snippets = buildSnippets(project.slug, mcpUrl(project.slug), info, useKey);
  return (
    <div className="space-y-8">
      <div className="space-y-6">
        <p className="max-w-prose text-sm text-muted-foreground">
          Use HTTP when this server is running. Use stdio when you'd rather the client start mcp-forge for you.
          Both serve the same tools.
        </p>
        {useKey ? (
          <Alert>
            <AlertTitle>Connect with an access key</AlertTitle>
            <AlertDescription>
              Replace <code>&lt;your-access-key&gt;</code> in the HTTP snippets with a key from below
              {info.auth_required && " (or the admin token)"}. stdio doesn't need one.
            </AlertDescription>
          </Alert>
        ) : (
          info.auth_required && (
            <Alert>
              <AlertTitle>This server needs the admin token</AlertTitle>
              <AlertDescription>
                Replace <code>&lt;your-admin-token&gt;</code> in the HTTP snippets with the token you started the server
                with, or create an access key below.
              </AlertDescription>
            </Alert>
          )
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
      <WriteGuard projectId={project.id}>
        <KeysCard project={project} keys={keys} onChanged={loadKeys} onSaved={onSaved} />
        <ConnectedAppsCard project={project} />
      </WriteGuard>
    </div>
  );
}

function KeysCard({
  project,
  keys,
  onChanged,
  onSaved,
}: {
  project: Project;
  keys: ApiKey[] | null;
  onChanged: () => Promise<void>;
  onSaved: (p: Project) => void;
}) {
  const [creating, setCreating] = useState(false);
  const [secret, setSecret] = useState<{ name: string; value: string } | null>(null);
  const [confirmDelete, setConfirmDelete] = useState<string | null>(null);

  async function setRequired(v: boolean) {
    try {
      onSaved(await api.updateSettings(project.id, { ...project.settings, require_key: v }));
      toast.success(v ? "This project now needs an access key" : "Access keys are optional again on this machine");
    } catch (e) {
      toast.error(e instanceof ApiError ? e.message : String(e));
    }
  }

  async function remove(k: ApiKey) {
    try {
      await api.deleteKey(project.id, k.id);
      toast.success(`Deleted ${k.name}: clients using it can't connect any more`);
      setConfirmDelete(null);
      await onChanged();
    } catch (e) {
      toast.error(e instanceof ApiError ? e.message : String(e));
    }
  }

  const now = Date.now();
  return (
    <Card>
      <CardHeader>
        <CardTitle>Access keys</CardTitle>
        <CardDescription>
          Give each client or teammate its own key for this project. A key can be limited to some tools and can expire;
          delete it to cut that client off. Keys are shown once and stored only as a hash.
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        <div className="flex items-start gap-2">
          <Switch id="k-require" checked={project.settings.require_key} onCheckedChange={(v) => void setRequired(v)} className="mt-0.5" />
          <div className="grid gap-0.5">
            <Label htmlFor="k-require">Require a key on this machine too</Label>
            <p className="text-xs text-muted-foreground">
              Off: clients on this computer connect without a key. Keys are always needed when the server listens on the
              network.
            </p>
          </div>
        </div>

        {keys && keys.length > 0 && (
          <ul className="divide-y overflow-hidden rounded-xl border">
            {keys.map((k) => {
              const expired = k.expires_at !== null && new Date(k.expires_at).getTime() <= now;
              return (
                <li key={k.id} className="flex flex-wrap items-center gap-3 px-4 py-3">
                  <div className="min-w-0 flex-1 space-y-0.5">
                    <div className="flex flex-wrap items-center gap-2">
                      <span className="font-medium">{k.name}</span>
                      <code className="font-mono text-xs text-muted-foreground">{k.prefix}…</code>
                      {expired && <Badge variant="destructive">Expired</Badge>}
                    </div>
                    <p className="text-xs text-muted-foreground">
                      {k.tools ? `Only ${k.tools.join(", ")}` : "All tools"}
                      {" · "}
                      {k.expires_at ? `${expired ? "expired" : "expires"} ${new Date(k.expires_at).toLocaleDateString()}` : "never expires"}
                      {" · "}
                      {k.last_used_at ? `last used ${new Date(k.last_used_at).toLocaleString()}` : "never used"}
                      {k.created_by && ` · created by ${k.created_by}`}
                    </p>
                  </div>
                  {confirmDelete === k.id ? (
                    <div className="flex gap-2">
                      <Button size="sm" variant="destructive" onClick={() => void remove(k)}>
                        Delete
                      </Button>
                      <Button size="sm" variant="outline" onClick={() => setConfirmDelete(null)}>
                        Keep
                      </Button>
                    </div>
                  ) : (
                    <Button size="sm" variant="ghost" className="text-destructive" onClick={() => setConfirmDelete(k.id)}>
                      Delete
                    </Button>
                  )}
                </li>
              );
            })}
          </ul>
        )}

        <Button variant="outline" onClick={() => setCreating(true)}>
          {keys && keys.length > 0 ? <PlusIcon /> : <KeyRoundIcon />} {keys && keys.length > 0 ? "New key" : "Create the first key"}
        </Button>
        {creating && (
          <NewKeyForm
            projectId={project.id}
            onCancel={() => setCreating(false)}
            onCreated={async (name, value) => {
              setCreating(false);
              setSecret({ name, value });
              await onChanged();
            }}
          />
        )}
      </CardContent>

      <Dialog open={secret !== null} onOpenChange={(o) => !o && setSecret(null)}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Copy the key for {secret?.name}</DialogTitle>
            <DialogDescription>
              This is the only time it's shown. Put it in the client's config as <code>Authorization: Bearer …</code>.
            </DialogDescription>
          </DialogHeader>
          {secret && <CopyBlock code={secret.value} label="Copy access key" />}
          <p className="flex items-start gap-2 text-xs text-muted-foreground">
            <TriangleAlertIcon className="mt-0.5 size-3.5 shrink-0 text-warning" />
            Anyone with this key can use this project's tools it allows. Lost it? Delete the key and create a new one.
          </p>
          <DialogFooter>
            <Button onClick={() => setSecret(null)}>I've copied it</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </Card>
  );
}

function NewKeyForm({
  projectId,
  onCancel,
  onCreated,
}: {
  projectId: string;
  onCancel: () => void;
  onCreated: (name: string, secret: string) => Promise<void>;
}) {
  const [name, setName] = useState("");
  const [expiry, setExpiry] = useState("never");
  const [allTools, setAllTools] = useState(true);
  const [picked, setPicked] = useState<string[]>([]);
  const [tools, setTools] = useState<Tool[] | null>(null);
  const [err, setErr] = useState<unknown>(null);

  useEffect(() => {
    void api.listTools(projectId).then(setTools);
  }, [projectId]);

  async function submit() {
    const days = EXPIRY.find((x) => x.value === expiry)?.days ?? null;
    try {
      const { key, secret } = await api.createKey(projectId, {
        name: name.trim(),
        tools: allTools ? null : picked,
        expires_at: days === null ? null : new Date(Date.now() + days * 86_400_000).toISOString(),
      });
      setErr(null);
      await onCreated(key.name, secret);
    } catch (e) {
      setErr(e);
      if (!(e instanceof ApiError && e.fields.length)) toast.error(e instanceof Error ? e.message : String(e));
    }
  }

  return (
    <StepDialog
      title="New access key"
      description="Each client or teammate gets its own key. It's shown once, then stored only as a hash."
      steps={[
        {
          title: "Name and expiry",
          description: "Name it after who or what uses it, so you know which key to delete later.",
          fields: ["name", "expires_at"],
          content: (
            <>
              <div className="grid gap-4 sm:grid-cols-2">
                <div className="grid gap-1.5">
                  <Label htmlFor="k-name">Name</Label>
                  <Input id="k-name" value={name} onChange={(e) => setName(e.target.value)} placeholder="laptop, ci, alice" required aria-invalid={!!fieldError(err, "name")} />
                  {fieldError(err, "name") && <p className="text-xs text-destructive">{fieldError(err, "name")}</p>}
                </div>
                <div className="grid gap-1.5">
                  <Label htmlFor="k-expiry">Expires</Label>
                  <Select value={expiry} onValueChange={setExpiry}>
                    <SelectTrigger id="k-expiry" className="w-full">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {EXPIRY.map((x) => (
                        <SelectItem key={x.value} value={x.value}>
                          {x.label}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>
              </div>
            </>
          ),
        },
        {
          title: "Which tools",
          description: "Limit the key to some tools, or let it use every tool agents can see.",
          fields: ["tools"],
          content: (
            <>
              <div className="grid gap-2">
                <div className="flex items-center gap-2">
                  <Checkbox id="k-all" checked={allTools} onCheckedChange={(v) => setAllTools(v === true)} />
                  <Label htmlFor="k-all" className="font-normal">
                    All tools agents can see
                  </Label>
                </div>
                {!allTools && (
                  <div className="grid max-h-56 gap-1.5 overflow-y-auto rounded-lg border p-3 sm:grid-cols-2">
                    {tools === null && <p className="text-xs text-muted-foreground">Loading tools…</p>}
                    {tools?.length === 0 && <p className="text-xs text-muted-foreground">This project has no tools yet.</p>}
                    {tools?.map((t) => (
                      <label key={t.id} className="flex items-center gap-2 text-sm">
                        <Checkbox
                          checked={picked.includes(t.name)}
                          onCheckedChange={(v) => setPicked(v === true ? [...picked, t.name] : picked.filter((n) => n !== t.name))}
                        />
                        <span className="truncate font-mono">{t.name}</span>
                      </label>
                    ))}
                  </div>
                )}
                {fieldError(err, "tools") && <p className="text-xs text-destructive">{fieldError(err, "tools")}</p>}
              </div>
            </>
          ),
        },
      ]}
      errorFields={err instanceof ApiError ? err.fields.map((f) => f.field) : []}
      submitLabel="Create key"
      onSubmit={submit}
      onClose={onCancel}
    />
  );
}

/** Apps connected by signing in (OAuth), e.g. claude.ai custom connectors. */
function ConnectedAppsCard({ project }: { project: Project }) {
  const [apps, setApps] = useState<ConnectedApp[] | null>(null);
  const load = useCallback(async () => setApps(await api.listConnectedApps(project.id)), [project.id]);
  useEffect(() => {
    void load();
  }, [load]);

  async function disconnect(a: ConnectedApp) {
    try {
      await api.disconnectApp(project.id, a.id);
      toast.success(`Disconnected ${a.client_name}`);
      await load();
    } catch (e) {
      toast.error(e instanceof ApiError ? e.message : String(e));
    }
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle>Connected apps</CardTitle>
        <CardDescription>
          Apps that support MCP sign-in (claude.ai custom connectors, MCP Inspector and others) need only the URL{" "}
          <code className="font-mono text-xs">{mcpUrl(project.slug)}</code>: they open a sign-in page here, and you
          approve them. The app must be able to reach this server, for example through an HTTPS reverse proxy.
        </CardDescription>
      </CardHeader>
      <CardContent>
        {apps === null ? null : apps.length === 0 ? (
          <p className="text-sm text-muted-foreground">No apps connected yet.</p>
        ) : (
          <ul className="divide-y overflow-hidden rounded-xl border">
            {apps.map((a) => (
              <li key={a.id} className="flex flex-wrap items-center gap-3 px-4 py-3">
                <div className="min-w-0 flex-1 space-y-0.5">
                  <p className="font-medium">{a.client_name}</p>
                  <p className="text-xs text-muted-foreground">
                    Connected {new Date(a.created_at).toLocaleString()}
                    {a.approved_by && ` by ${a.approved_by}`}
                    {" · "}
                    {a.last_used_at ? `last used ${new Date(a.last_used_at).toLocaleString()}` : "not used yet"}
                    {" · "}
                    signed in until {new Date(a.refresh_expires_at).toLocaleDateString()} (renewed when used)
                  </p>
                </div>
                <Button size="sm" variant="ghost" className="text-destructive" onClick={() => void disconnect(a)}>
                  Disconnect
                </Button>
              </li>
            ))}
          </ul>
        )}
      </CardContent>
    </Card>
  );
}
