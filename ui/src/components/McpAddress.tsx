import { useState } from "react";
import { CheckIcon, CopyIcon } from "lucide-react";

import { Button } from "@/components/ui/button";

/** The address agents connect to, set like a request line, with a copy button. */
export function McpAddress({ url }: { url: string }) {
  const [copied, setCopied] = useState(false);
  const u = new URL(url);

  async function copy() {
    try {
      await navigator.clipboard.writeText(url);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch {
      window.getSelection()?.selectAllChildren(document.getElementById("mcp-address")!);
    }
  }

  return (
    <div className="flex min-w-0 items-center gap-3 rounded-xl border bg-card py-2 pr-2 pl-3 shadow-xs">
      <span className="shrink-0 rounded-[4px] bg-primary/12 px-1.5 py-px font-mono text-xs font-semibold text-primary">MCP</span>
      <code id="mcp-address" className="min-w-0 flex-1 truncate text-sm sm:text-[0.95rem]" title={url}>
        <span className="text-muted-foreground">{u.origin}</span>
        <span className="font-medium">{u.pathname}</span>
      </code>
      <Button type="button" variant="outline" size="sm" onClick={() => void copy()} aria-label={copied ? "MCP URL copied" : "Copy MCP URL"}>
        {copied ? <CheckIcon className="text-success" /> : <CopyIcon />}
        <span className="hidden sm:inline">{copied ? "Copied" : "Copy"}</span>
      </Button>
      <span className="sr-only" aria-live="polite">
        {copied ? "Copied" : ""}
      </span>
    </div>
  );
}
