import type {
  AuthConfig,
  AuthConfigInput,
  BundleReport,
  FieldError,
  ImportPreview,
  JsonObject,
  McpToolView,
  Project,
  ProjectSettings,
  ReimportPreview,
  ServerInfo,
  SpecRecord,
  TestRun,
  Tool,
  ToolInput,
} from "./types";

const TOKEN_KEY = "mcp-forge.admin-token";

/** Admin token kept per browser tab; never persisted beyond the session. */
export const token = {
  get(): string | null {
    try {
      return sessionStorage.getItem(TOKEN_KEY);
    } catch {
      return memoryToken;
    }
  },
  set(value: string | null) {
    memoryToken = value;
    try {
      if (value) sessionStorage.setItem(TOKEN_KEY, value);
      else sessionStorage.removeItem(TOKEN_KEY);
    } catch {
      /* storage unavailable: memory only */
    }
  },
};
let memoryToken: string | null = null;

export class ApiError extends Error {
  constructor(
    readonly status: number,
    readonly code: string,
    message: string,
    readonly fields: FieldError[] = [],
  ) {
    super(message);
  }
}

/** Called when the server answers 401, so the app can ask for a token. */
let onUnauthorized: () => void = () => {};
export function setUnauthorizedHandler(fn: () => void) {
  onUnauthorized = fn;
}

async function request<T>(method: string, path: string, body?: unknown): Promise<T> {
  const headers: Record<string, string> = { Accept: "application/json" };
  if (body !== undefined) headers["Content-Type"] = "application/json";
  const t = token.get();
  if (t) headers.Authorization = `Bearer ${t}`;

  let resp: Response;
  try {
    resp = await fetch(`/api${path}`, {
      method,
      headers,
      body: body === undefined ? undefined : JSON.stringify(body),
    });
  } catch {
    throw new ApiError(0, "network", "Can't reach the mcp-forge server. Is it still running?");
  }
  if (resp.status === 204) return undefined as T;
  const text = await resp.text();
  let data: unknown = null;
  try {
    data = text ? JSON.parse(text) : null;
  } catch {
    data = null;
  }
  if (!resp.ok) {
    const err = (data as { error?: { code?: string; message?: string; fields?: FieldError[] } } | null)?.error;
    if (resp.status === 401) onUnauthorized();
    throw new ApiError(
      resp.status,
      err?.code ?? "http_error",
      err?.message ?? `The server answered ${resp.status}.`,
      err?.fields ?? [],
    );
  }
  return data as T;
}

/** Like `request`, for endpoints that answer with plain text (YAML export). */
async function requestText(method: string, path: string): Promise<string> {
  const headers: Record<string, string> = {};
  const t = token.get();
  if (t) headers.Authorization = `Bearer ${t}`;
  const resp = await fetch(`/api${path}`, { method, headers });
  if (resp.status === 401) onUnauthorized();
  if (!resp.ok) throw new ApiError(resp.status, "http_error", `The server answered ${resp.status}.`);
  return resp.text();
}

const p = (projectId: string) => `/projects/${encodeURIComponent(projectId)}`;

export const api = {
  info: () => request<ServerInfo>("GET", "/info"),

  listProjects: () => request<Project[]>("GET", "/projects"),
  getProject: (id: string) => request<Project>("GET", p(id)),
  createProject: (input: { name: string; slug?: string }) => request<Project>("POST", "/projects", input),
  updateProject: (id: string, input: { name: string; slug?: string }) => request<Project>("PUT", p(id), input),
  deleteProject: (id: string) => request<void>("DELETE", p(id)),
  updateSettings: (id: string, settings: ProjectSettings) => request<Project>("PUT", `${p(id)}/settings`, settings),

  listTools: (pid: string) => request<Tool[]>("GET", `${p(pid)}/tools`),
  getTool: (pid: string, id: string) => request<Tool>("GET", `${p(pid)}/tools/${id}`),
  createTool: (pid: string, input: ToolInput) => request<Tool>("POST", `${p(pid)}/tools`, input),
  updateTool: (pid: string, id: string, input: ToolInput) => request<Tool>("PUT", `${p(pid)}/tools/${id}`, input),
  deleteTool: (pid: string, id: string) => request<void>("DELETE", `${p(pid)}/tools/${id}`),
  toolMcpView: (pid: string, id: string) => request<McpToolView>("GET", `${p(pid)}/tools/${id}/mcp`),
  testTool: (pid: string, id: string, args: JsonObject) =>
    request<TestRun>("POST", `${p(pid)}/tools/${id}/test`, { arguments: args }),

  previewOpenApi: (pid: string, body: { content?: string; url?: string; base_url?: string }) =>
    request<ImportPreview>("POST", `${p(pid)}/import/openapi/preview`, body),
  importOpenApi: (
    pid: string,
    body: {
      content: string;
      source_url?: string | null;
      filename?: string | null;
      base_url?: string;
      auth_ref?: string | null;
      operations: { key: string; name?: string }[];
    },
  ) => request<{ spec: SpecRecord; created: Tool[] }>("POST", `${p(pid)}/import/openapi`, body),
  listSpecs: (pid: string) => request<SpecRecord[]>("GET", `${p(pid)}/specs`),
  previewReimport: (
    pid: string,
    sid: string,
    body: { content?: string; url?: string; base_url?: string; auth_ref?: string },
  ) => request<ReimportPreview>("POST", `${p(pid)}/specs/${sid}/reimport/preview`, body),
  reimport: (
    pid: string,
    sid: string,
    body: {
      content?: string;
      source_url?: string | null;
      filename?: string | null;
      base_url?: string;
      auth_ref?: string;
      add: string[];
      remove: string[];
    },
  ) =>
    request<{ spec: SpecRecord; result: { updated: Tool[]; created: Tool[]; deleted: string[] } }>(
      "POST",
      `${p(pid)}/specs/${sid}/reimport`,
      body,
    ),
  resetOverrides: (pid: string, tid: string, fields?: string[]) =>
    request<Tool>("POST", `${p(pid)}/tools/${tid}/overrides/reset`, fields ? { fields } : {}),
  exportBundle: (pid: string) => requestText("GET", `${p(pid)}/export`),
  importBundle: (pid: string, content: string, onConflict: "fail" | "skip" | "replace") =>
    request<BundleReport>("POST", `${p(pid)}/import/bundle`, { content, on_conflict: onConflict }),

  listAuth: (pid: string) => request<AuthConfig[]>("GET", `${p(pid)}/auth-configs`),
  createAuth: (pid: string, input: AuthConfigInput) => request<AuthConfig>("POST", `${p(pid)}/auth-configs`, input),
  updateAuth: (pid: string, id: string, input: AuthConfigInput) =>
    request<AuthConfig>("PUT", `${p(pid)}/auth-configs/${id}`, input),
  deleteAuth: (pid: string, id: string) => request<void>("DELETE", `${p(pid)}/auth-configs/${id}`),
};
