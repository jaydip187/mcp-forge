import { useState, type FormEvent } from "react";
import { ChevronsUpDownIcon, KeyRoundIcon, Loader2Icon, LogOutIcon } from "lucide-react";
import { toast } from "sonner";

import { auth, ApiError } from "@/api";
import { Field } from "@/components/FormSection";
import { PasswordInput } from "@/components/PasswordInput";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { SidebarMenu, SidebarMenuButton, SidebarMenuItem } from "@/components/ui/sidebar";
import type { User } from "@/types";

export function UserMenu({ user, version, onSignedOut }: { user: User; version: string; onSignedOut: () => void }) {
  const [changing, setChanging] = useState(false);

  async function signOut() {
    try {
      await auth.logout();
    } finally {
      onSignedOut();
    }
  }

  return (
    <SidebarMenu>
      <SidebarMenuItem>
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <SidebarMenuButton size="lg" className="data-[state=open]:bg-sidebar-accent">
              <span className="grid size-8 shrink-0 place-items-center rounded-lg bg-primary/10 text-sm font-semibold text-primary uppercase">
                {user.username.slice(0, 1)}
              </span>
              <span className="grid min-w-0 flex-1 text-left leading-tight">
                <span className="truncate text-sm font-medium">{user.username}</span>
                <span className="truncate text-xs text-muted-foreground">mcp-forge {version}</span>
              </span>
              <ChevronsUpDownIcon className="ml-auto size-4 text-muted-foreground" />
            </SidebarMenuButton>
          </DropdownMenuTrigger>
          <DropdownMenuContent side="top" align="start" className="w-(--radix-dropdown-menu-trigger-width) min-w-56">
            <DropdownMenuLabel className="font-normal text-muted-foreground">
              Signed in as <span className="font-medium text-foreground">{user.username}</span>
            </DropdownMenuLabel>
            <DropdownMenuSeparator />
            <DropdownMenuItem onSelect={() => setChanging(true)}>
              <KeyRoundIcon /> Change password
            </DropdownMenuItem>
            <DropdownMenuItem onSelect={() => void signOut()}>
              <LogOutIcon /> Sign out
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
        <ChangePasswordDialog open={changing} onOpenChange={setChanging} />
      </SidebarMenuItem>
    </SidebarMenu>
  );
}

function ChangePasswordDialog({ open, onOpenChange }: { open: boolean; onOpenChange: (open: boolean) => void }) {
  const [current, setCurrent] = useState("");
  const [next, setNext] = useState("");
  const [confirm, setConfirm] = useState("");
  const [fields, setFields] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState(false);
  const mismatch = confirm.length > 0 && confirm !== next;

  function reset(open: boolean) {
    if (!open) {
      setCurrent("");
      setNext("");
      setConfirm("");
      setFields({});
    }
    onOpenChange(open);
  }

  async function submit(e: FormEvent) {
    e.preventDefault();
    if (mismatch) return;
    setBusy(true);
    setFields({});
    try {
      await auth.changePassword({ current_password: current, new_password: next });
      toast.success("Password changed. Other devices were signed out.");
      reset(false);
    } catch (err) {
      if (err instanceof ApiError && err.fields.length) {
        setFields(Object.fromEntries(err.fields.map((f) => [f.field, f.message])));
      } else {
        toast.error(err instanceof Error ? err.message : String(err));
      }
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog open={open} onOpenChange={reset}>
      <DialogContent className="sm:max-w-md">
        <form onSubmit={submit} className="grid gap-5">
          <DialogHeader>
            <DialogTitle>Change password</DialogTitle>
            <DialogDescription>Other sessions are signed out; this one stays signed in.</DialogDescription>
          </DialogHeader>
          <Field id="current-password" label="Current password" error={fields.current_password}>
            <PasswordInput
              id="current-password"
              autoComplete="current-password"
              aria-invalid={!!fields.current_password}
              value={current}
              onChange={(e) => setCurrent(e.target.value)}
              autoFocus
              required
            />
          </Field>
          <Field id="next-password" label="New password" error={fields.new_password} hint="At least 8 characters.">
            <PasswordInput
              id="next-password"
              autoComplete="new-password"
              aria-invalid={!!fields.new_password}
              value={next}
              onChange={(e) => setNext(e.target.value)}
              required
            />
          </Field>
          <Field id="confirm-next" label="Confirm new password" error={mismatch ? "doesn't match" : undefined}>
            <PasswordInput
              id="confirm-next"
              autoComplete="new-password"
              aria-invalid={mismatch}
              value={confirm}
              onChange={(e) => setConfirm(e.target.value)}
              required
            />
          </Field>
          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => reset(false)}>
              Cancel
            </Button>
            <Button type="submit" disabled={busy || !current || !next || mismatch}>
              {busy && <Loader2Icon className="animate-spin" />}
              Change password
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
