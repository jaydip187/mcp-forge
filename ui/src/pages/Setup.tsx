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

const MIN_PASSWORD = 8;

/** First run: create the first account with the setup code from the server log. */
export function Setup({ onSignedIn }: { onSignedIn: (user: User) => void }) {
  const [code, setCode] = useState("");
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [confirm, setConfirm] = useState("");
  const [fields, setFields] = useState<Record<string, string>>({});
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const mismatch = confirm.length > 0 && confirm !== password;

  async function submit(e: FormEvent) {
    e.preventDefault();
    if (password.length < MIN_PASSWORD) {
      setFields({ password: `must be at least ${MIN_PASSWORD} characters` });
      return;
    }
    if (mismatch) return;
    setBusy(true);
    setError(null);
    setFields({});
    try {
      const { user } = await auth.setup({ code: code.trim(), username: username.trim(), password });
      onSignedIn(user);
    } catch (err) {
      if (err instanceof ApiError && err.fields.length) {
        setFields(Object.fromEntries(err.fields.map((f) => [f.field, f.message])));
      } else {
        setError(err instanceof Error ? err.message : String(err));
      }
    } finally {
      setBusy(false);
    }
  }

  return (
    <AuthLayout
      title="Create your account"
      description={
        <>
          This is the first run. Copy the setup code from the terminal where you started <code className="font-mono">mcp-forge serve</code>.
        </>
      }
    >
      <form onSubmit={submit} className="grid gap-5" noValidate>
        <Field id="code" label="Setup code" hint="Looks like ABCD-EFGH-JKLM. It works once.">
          <Input
            id="code"
            autoComplete="one-time-code"
            autoCapitalize="characters"
            spellCheck={false}
            className="font-mono tracking-widest uppercase"
            value={code}
            onChange={(e) => setCode(e.target.value)}
            autoFocus
            required
          />
        </Field>
        <Field id="new-username" label="Username" error={fields.username} hint="Letters, digits and . _ - @">
          <Input
            id="new-username"
            autoComplete="username"
            autoCapitalize="none"
            spellCheck={false}
            aria-invalid={!!fields.username}
            value={username}
            onChange={(e) => setUsername(e.target.value)}
            required
          />
        </Field>
        <Field id="new-password" label="Password" error={fields.password} hint={`At least ${MIN_PASSWORD} characters.`}>
          <PasswordInput
            id="new-password"
            autoComplete="new-password"
            aria-invalid={!!fields.password}
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            required
          />
        </Field>
        <Field id="confirm-password" label="Confirm password" error={mismatch ? "doesn't match" : undefined}>
          <PasswordInput
            id="confirm-password"
            autoComplete="new-password"
            aria-invalid={mismatch}
            value={confirm}
            onChange={(e) => setConfirm(e.target.value)}
            required
          />
        </Field>
        {error && (
          <Alert variant="destructive" role="alert">
            <AlertDescription>{error}</AlertDescription>
          </Alert>
        )}
        <Button type="submit" size="lg" disabled={busy || !code.trim() || !username.trim() || !password || mismatch}>
          {busy && <Loader2Icon className="animate-spin" />}
          Create account
        </Button>
      </form>
    </AuthLayout>
  );
}
