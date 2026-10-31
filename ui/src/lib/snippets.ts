import type { ServerInfo } from "../types";

export interface Snippet {
  id: string;
  title: string;
  hint: string;
  code: string;
}

const TOKEN_PLACEHOLDER = "<your-admin-token>";
const KEY_PLACEHOLDER = "<your-access-key>";

/** Single quotes are literal in both POSIX shells and PowerShell, so Windows paths survive as-is. */
function shellQuote(s: string): string {
  return /^[A-Za-z0-9_./:@=-]+$/.test(s) ? s : `'${s.replace(/'/g, "'\\''")}'`;
}

/**
 * Client configuration snippets for one project. Never embeds a real token
 * or key. With `useKey` (the project has access keys or requires one) the
 * HTTP snippets carry an access-key placeholder.
 */
export function buildSnippets(slug: string, mcpUrl: string, info: ServerInfo, useKey = false): Snippet[] {
  const placeholder = useKey ? KEY_PLACEHOLDER : TOKEN_PLACEHOLDER;
  const headers = info.auth_required || useKey ? { Authorization: `Bearer ${placeholder}` } : undefined;
  const exe = info.exe ?? "mcp-forge";
  const stdioArgs = ["stdio", "--project", slug, ...(info.data_dir ? ["--data-dir", info.data_dir] : [])];

  const httpJson = {
    mcpServers: { [slug]: { type: "http", url: mcpUrl, ...(headers ? { headers } : {}) } },
  };
  const stdioJson = { mcpServers: { [slug]: { command: exe, args: stdioArgs } } };

  const claudeHttp = [
    "claude mcp add --transport http",
    shellQuote(slug),
    shellQuote(mcpUrl),
    ...(headers ? ["--header", shellQuote(`Authorization: Bearer ${placeholder}`)] : []),
  ].join(" ");
  const claudeStdio = ["claude mcp add", shellQuote(slug), "--", shellQuote(exe), ...stdioArgs.map(shellQuote)].join(" ");

  return [
    {
      id: "claude-http",
      title: "Claude Code (HTTP)",
      hint: "Run in your terminal. Needs this server running.",
      code: claudeHttp,
    },
    {
      id: "json-http",
      title: "JSON config (HTTP)",
      hint: "For clients that read an mcpServers file, such as .mcp.json.",
      code: JSON.stringify(httpJson, null, 2),
    },
    {
      id: "claude-stdio",
      title: "Claude Code (stdio)",
      hint: "The client starts mcp-forge itself. No server needs to be running.",
      code: claudeStdio,
    },
    {
      id: "json-stdio",
      title: "JSON config (stdio)",
      hint: "Same as above, for mcpServers files.",
      code: JSON.stringify(stdioJson, null, 2),
    },
  ];
}
