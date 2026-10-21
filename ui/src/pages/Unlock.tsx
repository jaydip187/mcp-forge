import { useState, type FormEvent } from "react";
import { KeyRoundIcon } from "lucide-react";

import { api, ApiError, token } from "@/api";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";

export function Unlock({ onUnlocked }: { onUnlocked: () => void }) {
  const [value, setValue] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  async function submit(e: FormEvent) {
    e.preventDefault();
    setBusy(true);
    token.set(value.trim());
    try {
      await api.info();
      onUnlocked();
    } catch (err) {
      token.set(null);
      setError(
        err instanceof ApiError && err.status === 401
          ? "That token doesn't match the one this server was started with."
          : err instanceof Error
            ? err.message
            : String(err),
      );
    } finally {
      setBusy(false);
    }
  }

  return (
    <main className="grid min-h-svh place-items-center p-4">
      <Card className="w-full max-w-sm">
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            <KeyRoundIcon className="size-4 text-primary" /> Enter the admin token
          </CardTitle>
          <CardDescription>
            This server was started with <code>--admin-token</code>. The token stays in this browser tab only.
          </CardDescription>
        </CardHeader>
        <CardContent>
          <form onSubmit={submit} className="grid gap-3">
            <Label htmlFor="token">Admin token</Label>
            <Input
              id="token"
              type="password"
              autoComplete="current-password"
              value={value}
              onChange={(e) => setValue(e.target.value)}
              autoFocus
              required
            />
            {error && (
              <Alert variant="destructive">
                <AlertDescription>{error}</AlertDescription>
              </Alert>
            )}
            <Button type="submit" disabled={busy || !value.trim()}>
              Unlock
            </Button>
          </form>
        </CardContent>
      </Card>
    </main>
  );
}
