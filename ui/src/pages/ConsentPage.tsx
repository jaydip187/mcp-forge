import { useEffect, useState } from "react";
import { ShieldCheckIcon } from "lucide-react";

import { api, ApiError } from "@/api";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardFooter, CardHeader, CardTitle } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import type { ConsentInfo, User } from "@/types";

/** An app (e.g. a claude.ai connector) asks to use one project's tools; the signed-in person decides. */
export function ConsentPage({ requestId, user }: { requestId: string; user: User | null }) {
  const [info, setInfo] = useState<ConsentInfo | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    api
      .oauthRequest(requestId)
      .then(setInfo)
      .catch((e) =>
        setError(
          e instanceof ApiError && e.status === 404
            ? "This sign-in request is unknown, already answered or older than 10 minutes. Start again from the app."
            : String(e),
        ),
      );
  }, [requestId]);

  async function answer(allow: boolean) {
    setBusy(true);
    try {
      const { redirect } = await (allow ? api.oauthApprove(requestId) : api.oauthDeny(requestId));
      // Back to the app, which finishes connecting.
      window.location.assign(redirect);
    } catch (e) {
      setBusy(false);
      setError(e instanceof ApiError ? e.message : String(e));
    }
  }

  return (
    <div className="mx-auto max-w-lg pt-6">
      {error ? (
        <Alert variant="destructive">
          <AlertDescription>{error}</AlertDescription>
        </Alert>
      ) : !info ? (
        <Skeleton className="h-64 w-full" />
      ) : (
        <Card>
          <CardHeader>
            <span className="mb-2 grid size-11 place-items-center rounded-lg border bg-card text-primary shadow-xs">
              <ShieldCheckIcon className="size-5" />
            </span>
            <CardTitle className="text-xl">Connect {info.client_name}?</CardTitle>
            <CardDescription>
              It asks to use the tools of the project <strong className="text-foreground">{info.project.name}</strong>{" "}
              (<code className="font-mono text-xs">/mcp/{info.project.slug}</code>) on your behalf.
            </CardDescription>
          </CardHeader>
          <CardContent className="space-y-3 text-sm">
            <ul className="list-disc space-y-1 pl-5 text-muted-foreground">
              <li>It can list and call every tool agents can see in this project, the same as a full access key.</li>
              <li>Tools that require approval still wait for one.</li>
              <li>You can disconnect it any time under the project's Connect tab.</li>
            </ul>
            <p className="text-xs text-muted-foreground">
              After you answer, you go back to <code className="font-mono">{info.redirect_host}</code>
              {user && (
                <>
                  . Signed in as <strong className="text-foreground">{user.username}</strong>
                </>
              )}
              .
            </p>
          </CardContent>
          <CardFooter className="flex justify-end gap-2">
            <Button variant="outline" disabled={busy} onClick={() => void answer(false)}>
              Deny
            </Button>
            <Button disabled={busy} onClick={() => void answer(true)}>
              Allow
            </Button>
          </CardFooter>
        </Card>
      )}
    </div>
  );
}
