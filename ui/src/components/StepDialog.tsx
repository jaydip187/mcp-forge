import { useEffect, useRef, useState, type FormEvent, type ReactNode } from "react";
import { CheckIcon } from "lucide-react";
import { toast } from "sonner";

import { ApiError } from "@/api";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { problemField } from "@/lib/fields";
import { cn } from "@/lib/utils";
import type { FieldError } from "@/types";

/** One page of a step-by-step form. */
export interface FormStep {
  title: string;
  description?: string;
  content: ReactNode;
  /** Field names (or prefixes) shown on this step, so errors can point to it. */
  fields?: string[];
}

/** Focuses the first text field or select in `root`; false if there is none. */
function focusFirstField(root: HTMLElement | null): boolean {
  const el = root?.querySelector<HTMLElement>("input:not([type=hidden]):not(:disabled), textarea:not(:disabled), button[role=combobox]:not(:disabled)");
  el?.focus();
  return !!el;
}

/** Errors a form found itself, as check problems. */
export function asProblems(errors: Record<string, string>): FieldError[] {
  return Object.entries(errors).map(([field, message]) => ({ field, message }));
}

/**
 * Check problems as the error a form keeps for the server, so its fields show
 * them. Code `check`: the dialog already lists them, so banners skip it.
 */
export function checkError(problems: FieldError[]): ApiError | null {
  return problems.length ? new ApiError(422, "check", "", problems) : null;
}

/** Whether `field` belongs to `step`. */
export function owns(step: FormStep, field: string): boolean {
  return (step.fields ?? []).some((f) => field === f || field.startsWith(`${f}.`) || field.startsWith(`${f}[`));
}

/**
 * A create (or edit) form in a dialog, one step at a time: numbered steps
 * on top, Back and Next below, the submit button on the last step. Errors
 * jump to the first step with a field in `errorFields`; steps with errors
 * are marked. `banner` (a server error, say) shows above every step.
 *
 * With `check`, Next validates first: it asks the server about the whole
 * form, keeps the problems with fields on this step or earlier ones (later
 * steps aren't filled in yet), and stays put while there are any, listing
 * them and passing them to `showErrors` for the fields to show.
 */
export function StepDialog({
  open = true,
  onClose,
  title,
  description,
  steps,
  errorFields = [],
  banner,
  submitLabel,
  busy = false,
  onSubmit,
  wide = false,
  extraActions,
  check,
  showErrors,
}: {
  open?: boolean;
  onClose: () => void;
  title: ReactNode;
  description?: ReactNode;
  steps: FormStep[];
  errorFields?: string[];
  banner?: ReactNode;
  submitLabel: string;
  busy?: boolean;
  onSubmit: () => void | Promise<void>;
  /** Room for tables, such as parameter lists. */
  wide?: boolean;
  /** Shown at the left of the footer, such as a delete button. */
  extraActions?: ReactNode;
  /** Every problem the server would refuse the form with now; none when it would save. */
  check?: () => Promise<FieldError[]>;
  /** Shows a check's problems (only those of the steps so far) on the fields. */
  showErrors?: (errors: FieldError[]) => void;
}) {
  const [at, setAt] = useState(0);
  const [checking, setChecking] = useState(false);
  const [problems, setProblems] = useState<FieldError[]>([]);
  const bodyRef = useRef<HTMLDivElement>(null);
  const invalid = steps.map((s) => errorFields.some((f) => owns(s, f)));
  const invalidKey = invalid.map(Number).join("");

  // New errors: show the first step that has one. Keyed on which steps
  // have errors, so fixing a field doesn't move you around.
  useEffect(() => {
    const first = invalidKey.indexOf("1");
    if (first >= 0) setAt(first);
  }, [invalidKey]);

  // A new step starts at its top, with the cursor in its first field.
  const shown = useRef(at);
  useEffect(() => {
    if (shown.current === at) return;
    shown.current = at;
    bodyRef.current?.scrollTo({ top: 0 });
    focusFirstField(bodyRef.current);
  }, [at]);

  const step = Math.min(at, steps.length - 1);
  const last = step === steps.length - 1;
  const current = steps[step]!;

  /** Moves to step `to`, checking every step before it first when going forward. */
  async function go(to: number) {
    if (to <= step || !check) {
      setProblems([]);
      setAt(to);
      return;
    }
    setChecking(true);
    try {
      const before = steps.slice(0, to);
      const found = (await check()).filter((f) => before.some((s) => owns(s, f.field)));
      setProblems(found);
      showErrors?.(found);
      const stuck = before.findIndex((s) => found.some((f) => owns(s, f.field)));
      setAt(stuck >= 0 ? stuck : to);
    } catch (e) {
      toast.error(e instanceof Error ? e.message : String(e));
    } finally {
      setChecking(false);
    }
  }

  function submit(e: FormEvent) {
    e.preventDefault();
    // Enter on an earlier step moves on instead of submitting.
    if (!last) void go(step + 1);
    else void onSubmit();
  }

  const here = problems.filter((f) => owns(current, f.field));

  return (
    <Dialog open={open} onOpenChange={(o) => !o && onClose()}>
      <DialogContent
        className={cn("flex max-h-[calc(100dvh-2rem)] flex-col gap-0 p-0 sm:max-w-2xl", wide && "sm:max-w-4xl")}
        // A stray click outside shouldn't throw away a half-filled form.
        onInteractOutside={(e) => e.preventDefault()}
        onOpenAutoFocus={(e) => {
          if (focusFirstField(bodyRef.current)) e.preventDefault();
        }}
      >
        <DialogHeader className="gap-1.5 border-b px-5 pt-5 pb-4">
          <DialogTitle className="text-lg">{title}</DialogTitle>
          {description && <DialogDescription className="leading-relaxed">{description}</DialogDescription>}
          {steps.length > 1 && (
            <ol className="mt-3 flex items-center gap-1.5" aria-label="Steps">
              {steps.map((s, i) => {
                const done = i < step;
                const here = i === step;
                return (
                  <li key={s.title} className={cn("flex min-w-0 items-center gap-1.5", i < steps.length - 1 && "flex-1")}>
                    <button
                      type="button"
                      onClick={() => void go(i)}
                      disabled={checking}
                      aria-current={here ? "step" : undefined}
                      className="group flex min-w-0 items-center gap-2 rounded-md text-left outline-none focus-visible:ring-2 focus-visible:ring-ring"
                    >
                      <span
                        className={cn(
                          "flex size-6 shrink-0 items-center justify-center rounded-full border text-xs font-semibold transition-colors",
                          invalid[i]
                            ? "border-destructive bg-destructive text-white"
                            : here
                              ? "border-primary bg-primary text-primary-foreground"
                              : done
                                ? "border-primary/40 bg-primary/10 text-primary"
                                : "text-muted-foreground group-hover:border-foreground/40",
                        )}
                      >
                        {done && !invalid[i] ? <CheckIcon className="size-3.5" /> : i + 1}
                      </span>
                      <span
                        className={cn(
                          "hidden truncate text-xs font-medium md:inline",
                          here ? "text-foreground" : invalid[i] ? "text-destructive" : "text-muted-foreground group-hover:text-foreground",
                        )}
                      >
                        {s.title}
                      </span>
                    </button>
                    {i < steps.length - 1 && <span className={cn("h-px min-w-3 flex-1", done ? "bg-primary/40" : "bg-border")} aria-hidden />}
                  </li>
                );
              })}
            </ol>
          )}
        </DialogHeader>

        <form onSubmit={submit} noValidate className="flex min-h-0 flex-1 flex-col">
          <div ref={bodyRef} className="min-h-0 flex-1 space-y-5 overflow-y-auto px-5 py-5">
            {banner}
            {here.length > 0 && (
              <Alert variant="destructive">
                <AlertTitle>Fix this to continue</AlertTitle>
                <AlertDescription>
                  <ul className="list-disc pl-4">
                    {here.map((f) => (
                      <li key={`${f.field}:${f.message}`}>
                        <span className="font-medium">{problemField(f.field)}:</span> {f.message}
                      </li>
                    ))}
                  </ul>
                </AlertDescription>
              </Alert>
            )}
            {steps.length > 1 && (
              <div className="space-y-1">
                <p className="text-xs font-medium text-muted-foreground">
                  Step {step + 1} of {steps.length}
                </p>
                <h3 className="font-heading text-base font-medium">{current.title}</h3>
                {current.description && <p className="text-sm leading-relaxed text-muted-foreground">{current.description}</p>}
              </div>
            )}
            <div className="grid gap-5">{current.content}</div>
          </div>

          <div className="flex flex-wrap items-center gap-2 rounded-b-xl border-t bg-muted/50 px-5 py-3">
            {extraActions}
            <span className="flex-1" />
            {step > 0 ? (
              <Button type="button" variant="outline" onClick={() => void go(step - 1)}>
                Back
              </Button>
            ) : (
              <Button type="button" variant="ghost" onClick={onClose}>
                Cancel
              </Button>
            )}
            {last ? (
              <Button type="submit" disabled={busy}>
                {submitLabel}
              </Button>
            ) : (
              <Button type="submit" disabled={checking}>
                {checking ? "Checking…" : "Next"}
              </Button>
            )}
          </div>
        </form>
      </DialogContent>
    </Dialog>
  );
}
