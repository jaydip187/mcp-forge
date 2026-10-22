import { cn } from "@/lib/utils";
import { splitTemplate } from "@/lib/template";
import type { HttpMethod } from "@/types";

const METHOD_COLOR: Record<HttpMethod, string> = {
  GET: "bg-m-get/12 text-m-get",
  POST: "bg-m-post/12 text-m-post",
  PUT: "bg-m-put/12 text-m-put",
  PATCH: "bg-m-patch/12 text-m-patch",
  DELETE: "bg-m-delete/12 text-m-delete",
  HEAD: "bg-m-other/12 text-m-other",
  OPTIONS: "bg-m-other/12 text-m-other",
};

export function MethodLabel({ method, className }: { method: HttpMethod; className?: string }) {
  return (
    <span
      className={cn(
        "inline-block rounded-[4px] px-1.5 py-px text-center font-mono text-[0.85em] leading-snug font-semibold tracking-tight",
        METHOD_COLOR[method],
        className,
      )}
    >
      {method}
    </span>
  );
}

/**
 * A tool shown as the request it makes: `GET https://api.x.io/users/{id}`,
 * with path placeholders set as inset tokens.
 */
export function RequestLine({
  method,
  baseUrl,
  path,
  size = "md",
  showBase = true,
  className,
}: {
  method: HttpMethod;
  baseUrl: string;
  path: string;
  size?: "sm" | "md" | "lg";
  showBase?: boolean;
  className?: string;
}) {
  const text = { sm: "text-xs", md: "text-sm", lg: "text-base sm:text-lg" }[size];
  return (
    <span className={cn("inline-flex min-w-0 items-baseline gap-2 font-mono", text, className)}>
      <MethodLabel method={method} className="w-[4.5em] shrink-0" />
      <span className="min-w-0 break-all">
        {showBase && <span className="text-muted-foreground">{baseUrl.replace(/\/+$/, "")}</span>}
        {splitTemplate(path || "/").map((s, i) =>
          s.kind === "text" ? (
            <span key={i}>{s.text}</span>
          ) : (
            <span
              key={i}
              className="mx-px rounded-sm bg-accent px-1 py-px text-accent-foreground ring-1 ring-primary/25 ring-inset"
            >
              {`{${s.name}}`}
            </span>
          ),
        )}
      </span>
    </span>
  );
}
