import { cn } from "@/lib/utils";

export function Logo({ className }: { className?: string }) {
  return (
    <svg viewBox="0 0 32 32" className={cn("size-6 shrink-0", className)} aria-hidden="true">
      <rect width="32" height="32" rx="7" className="fill-primary" />
      <path d="M9 10h14M9 16h9M9 22h14" className="stroke-primary-foreground" strokeWidth="3" strokeLinecap="round" />
    </svg>
  );
}
