import { useState, type FormEvent } from "react";
import { Loader2Icon } from "lucide-react";

import { auth, ApiError } from "@/api";
import { AuthLayout } from "@/components/AuthLayout";
import { Field } from "@/components/FormSection";
import { PasswordInput } from "@/components/PasswordInput";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import type { User } from "@/types";

export function Login({ onSignedIn }: { onSignedIn: (user: User) => void }) {
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  async function submit(e: FormEvent) {
    e.preventDefault();
    setBusy(true);
    setError(null);
    try {
      const { user } = await auth.login({ username: username.trim(), password });
      onSignedIn(user);
    } catch (err) {
      setPassword("");
      setError(err instanceof ApiError || err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <AuthLayout title="Sign in" description="Use your mcp-forge account to manage projects, tools and connections.">
      <form onSubmit={submit} className="grid gap-5" noValidate>
        <Field id="username" label="Username">
          <Input
            id="username"
            autoComplete="username"
            autoCapitalize="none"
            spellCheck={false}
            value={username}
            onChange={(e) => setUsername(e.target.value)}
            autoFocus
            required
          />
        </Field>
        <Field id="password" label="Password">
          <PasswordInput
            id="password"
            autoComplete="current-password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            required
          />
        </Field>
        {error && (
          <Alert variant="destructive" role="alert">
            <AlertDescription>{error}</AlertDescription>
          </Alert>
        )}
        <Button type="submit" size="lg" disabled={busy || !username.trim() || !password}>
          {busy && <Loader2Icon className="animate-spin" />}
          Sign in
        </Button>
        <p className="text-xs text-muted-foreground">
          Forgot your password? On the server, run <code className="font-mono">mcp-forge user passwd &lt;name&gt;</code>.
        </p>
      </form>
    </AuthLayout>
  );
}
