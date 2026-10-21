import { useState } from "react";
import { CheckIcon, CopyIcon } from "lucide-react";

import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

async function copyText(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    return false;
  }
}

/** Code (or a URL) with a copy button. Falls back to selecting the text. */
export function CopyBlock({ code, label, inline = false }: { code: string; label: string; inline?: boolean }) {
  const [copied, setCopied] = useState(false);
  const [preId] = useState(() => `copy-${Math.random().toString(36).slice(2)}`);

  async function onCopy() {
    if (await copyText(code)) {
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } else {
      const el = document.getElementById(preId);
      if (el) window.getSelection()?.selectAllChildren(el);
    }
  }

  return (
    <div className={cn("group relative rounded-lg border bg-muted/60", inline && "flex items-center")}>
      <pre
        id={preId}
        className={cn(
          "p-3 pr-12 font-mono text-[0.8rem] leading-relaxed break-all whitespace-pre-wrap",
          inline && "flex-1 py-2",
        )}
      >
        {code}
      </pre>
      <Button
        type="button"
        size="icon-sm"
        variant="ghost"
        onClick={onCopy}
        className="absolute top-1.5 right-1.5"
        aria-label={copied ? `${label} copied` : `Copy ${label}`}
      >
        {copied ? <CheckIcon className="text-success" /> : <CopyIcon />}
      </Button>
      <span className="sr-only" aria-live="polite">
        {copied ? "Copied" : ""}
      </span>
    </div>
  );
}
