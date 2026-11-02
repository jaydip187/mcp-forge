import { useCallback, useEffect, useState } from "react";
import { UserPlusIcon } from "lucide-react";
import { toast } from "sonner";

import { api, ApiError } from "@/api";
import { PageHeader } from "@/components/PageHeader";
import { StepDialog } from "@/components/StepDialog";
import { PasswordInput } from "@/components/PasswordInput";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { usePermissions } from "@/lib/permissions";
import { fieldError, useProjects } from "@/lib/projects";
import type { Access, Project, Role, UserView } from "@/types";

const ROLE_LABEL: Record<Role, string> = { admin: "Admin", member: "Member" };
const NONE = "none";

function errorText(e: unknown): string {
  return e instanceof ApiError ? e.message : String(e);
}

/** Accounts, their role, and members' access per project. Admins only. */
export function UsersPage() {
  const { isAdmin, user: me } = usePermissions();
  const { projects } = useProjects();
  const [users, setUsers] = useState<UserView[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [adding, setAdding] = useState(false);

  const load = useCallback(async () => {
    try {
      setUsers(await api.listUsers());
    } catch (e) {
      setError(errorText(e));
    }
  }, []);
  useEffect(() => {
    void load();
  }, [load]);

  if (!isAdmin)
    return (
      <Alert>
        <AlertDescription>Only admins manage users.</AlertDescription>
      </Alert>
    );

  return (
    <div className="space-y-6">
      <PageHeader
        title="Users"
        description="Admins can do everything. Members see only the projects you give them, and can view or change each one."
        actions={
          <Button onClick={() => setAdding(true)}>
            <UserPlusIcon /> Add user
          </Button>
        }
      />
      {error && (
        <Alert variant="destructive">
          <AlertDescription>{error}</AlertDescription>
        </Alert>
      )}
      {!users || !projects ? (
        <Skeleton className="h-48 w-full" />
      ) : (
        <ul className="space-y-3">
          {users.map((u) => (
            <li key={u.id}>
              <UserCard user={u} me={me?.id === u.id} projects={projects} onChanged={load} />
            </li>
          ))}
        </ul>
      )}
      {adding && (
        <NewUserDialog
          onClose={() => setAdding(false)}
          onCreated={async () => {
            setAdding(false);
            await load();
          }}
        />
      )}
    </div>
  );
}

function UserCard({ user: u, me, projects, onChanged }: { user: UserView; me: boolean; projects: Project[]; onChanged: () => Promise<void> }) {
  const [resetting, setResetting] = useState(false);
  const [password, setPassword] = useState("");
  const [confirmDelete, setConfirmDelete] = useState(false);

  async function run(action: () => Promise<unknown>, done: string) {
    try {
      await action();
      toast.success(done);
      await onChanged();
    } catch (e) {
      toast.error(errorText(e));
    }
  }

  const accessOf = (pid: string): Access | null => u.access.find((a) => a.project_id === pid)?.access ?? null;

  return (
    <Card>
      <CardContent className="space-y-4">
        <div className="flex flex-wrap items-center gap-3">
          <div className="min-w-0 flex-1">
            <p className="flex items-center gap-2 font-medium">
              {u.username}
              {me && <Badge variant="secondary">You</Badge>}
            </p>
            <p className="text-xs text-muted-foreground">Since {new Date(u.created_at).toLocaleDateString()}</p>
          </div>
          <Select value={u.role} onValueChange={(v) => void run(() => api.setRole(u.id, v as Role), `${u.username} is now ${ROLE_LABEL[v as Role].toLowerCase()}`)}>
            <SelectTrigger className="w-32" aria-label={`Role of ${u.username}`}>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="admin">Admin</SelectItem>
              <SelectItem value="member">Member</SelectItem>
            </SelectContent>
          </Select>
          <Button variant="outline" size="sm" onClick={() => setResetting(!resetting)}>
            Reset password
          </Button>
          {!me &&
            (confirmDelete ? (
              <>
                <Button size="sm" variant="destructive" onClick={() => void run(() => api.deleteUser(u.id), `Removed ${u.username}`)}>
                  Remove
                </Button>
                <Button size="sm" variant="outline" onClick={() => setConfirmDelete(false)}>
                  Keep
                </Button>
              </>
            ) : (
              <Button size="sm" variant="ghost" className="text-destructive" onClick={() => setConfirmDelete(true)}>
                Remove
              </Button>
            ))}
        </div>

        {resetting && (
          <form
            className="flex flex-wrap items-end gap-2"
            onSubmit={(e) => {
              e.preventDefault();
              void run(() => api.resetPassword(u.id, password), `New password set for ${u.username}${me ? "" : "; they're signed out everywhere"}`).then(() => {
                setPassword("");
                setResetting(false);
              });
            }}
          >
            <div className="grid min-w-64 gap-1.5">
              <Label htmlFor={`pw-${u.id}`}>New password</Label>
              <PasswordInput id={`pw-${u.id}`} value={password} onChange={(e) => setPassword(e.target.value)} autoComplete="new-password" />
            </div>
            <Button type="submit" size="sm" disabled={password.length < 8}>
              Set password
            </Button>
          </form>
        )}

        {u.role === "admin" ? (
          <p className="text-xs text-muted-foreground">Admins can see and change every project.</p>
        ) : projects.length === 0 ? (
          <p className="text-xs text-muted-foreground">There are no projects yet.</p>
        ) : (
          <div className="grid gap-2 sm:grid-cols-2 lg:grid-cols-3">
            {projects.map((p) => (
              <div key={p.id} className="flex items-center justify-between gap-2 rounded-lg border px-3 py-2">
                <span className="min-w-0 truncate text-sm">{p.name}</span>
                <Select
                  value={accessOf(p.id) ?? NONE}
                  onValueChange={(v) =>
                    void run(
                      () => api.setAccess(u.id, p.id, v === NONE ? null : (v as Access)),
                      v === NONE ? `${u.username} can no longer open ${p.name}` : `${u.username} can ${v === "write" ? "change" : "view"} ${p.name}`,
                    )
                  }
                >
                  <SelectTrigger size="sm" className="w-32" aria-label={`${u.username} on ${p.name}`}>
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value={NONE}>No access</SelectItem>
                    <SelectItem value="read">Can view</SelectItem>
                    <SelectItem value="write">Can change</SelectItem>
                  </SelectContent>
                </Select>
              </div>
            ))}
          </div>
        )}
      </CardContent>
    </Card>
  );
}

function NewUserDialog({ onClose, onCreated }: { onClose: () => void; onCreated: () => Promise<void> }) {
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [role, setRole] = useState<Role>("member");
  const [err, setErr] = useState<unknown>(null);

  async function submit() {
    try {
      await api.createUser({ username: username.trim(), password, role });
      toast.success(`Added ${username.trim()}${role === "member" ? ": now give them projects on their card" : ""}`);
      await onCreated();
    } catch (e) {
      setErr(e);
      if (!(e instanceof ApiError && e.fields.length)) toast.error(errorText(e));
    }
  }

  return (
    <StepDialog
      title="Add a user"
      description="Share the password with them; they can change it from their user menu."
      steps={[
        {
          title: "Account",
          fields: ["username", "password", "role"],
          content: (
            <>
              <div className="grid gap-1.5">
                <Label htmlFor="nu-name">Username</Label>
                <Input id="nu-name" value={username} onChange={(e) => setUsername(e.target.value)} autoComplete="off" aria-invalid={!!fieldError(err, "username")} />
                {fieldError(err, "username") && <p className="text-xs text-destructive">{fieldError(err, "username")}</p>}
              </div>
              <div className="grid gap-1.5">
                <Label htmlFor="nu-pw">Password</Label>
                <PasswordInput id="nu-pw" value={password} onChange={(e) => setPassword(e.target.value)} autoComplete="new-password" aria-invalid={!!fieldError(err, "password")} />
                {fieldError(err, "password") && <p className="text-xs text-destructive">{fieldError(err, "password")}</p>}
              </div>
              <div className="grid gap-1.5">
                <Label htmlFor="nu-role">Role</Label>
                <Select value={role} onValueChange={(v) => setRole(v as Role)}>
                  <SelectTrigger id="nu-role" className="w-full">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="member">Member</SelectItem>
                    <SelectItem value="admin">Admin</SelectItem>
                  </SelectContent>
                </Select>
              </div>
            </>
          ),
        },
      ]}
      errorFields={err instanceof ApiError ? err.fields.map((f) => f.field) : []}
      submitLabel="Add user"
      onSubmit={submit}
      onClose={onClose}
    />
  );
}
