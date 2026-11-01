import type {
  Access,
  ProjectPerson,
  Role,
  UserView,
  ApiKey,
  ConnectedApp,
  ConsentInfo,
  ApiKeyInput,
  AuditEntry,
  CallEntry,
  CallFilter,
  CallSummary,
  CallWindow,
  OAuthStatus,
  PendingApproval,
  AuthConfig,
  AuthStatus,
  AuthConfigInput,
  BundleReport,
  ConnectionReport,
  DbConnection,
  DbConnectionInput,
  ExplorationMode,
  FieldError,
  ImportPreview,
  JsonObject,
  McpResourceView,
  PromptDef,
  PromptInput,
  ResourceDef,
  ResourceInput,
  McpToolView,
  Project,
  ProjectSettings,
  ReimportPreview,
  ServerInfo,
  SpecRecord,
  TestRun,
  Tool,
  ToolInput,
  User,
} from "./types";

/**
 * Sent on every call. The server rejects state-changing requests without it,
 * so other sites can't forge them with the session cookie (custom headers
 * need a CORS preflight, which the server never grants).
 */
const CSRF_HEADER = "X-Mcp-Forge";

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

/** Called when the server answers 401, so the app can show the sign-in screen. */
let onUnauthorized: () => void = () => {};
export function setUnauthorizedHandler(fn: () => void) {
  onUnauthorized = fn;
}

async function request<T>(method: string, path: string, body?: unknown): Promise<T> {
  const headers: Record<string, string> = { Accept: "application/json", [CSRF_HEADER]: "1" };
  if (body !== undefined) headers["Content-Type"] = "application/json";

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
    if (resp.status === 401 && !path.startsWith("/auth/")) onUnauthorized();
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
  const resp = await fetch(`/api${path}`, { method, headers: { [CSRF_HEADER]: "1" } });
  if (resp.status === 401) onUnauthorized();
  if (!resp.ok) throw new ApiError(resp.status, "http_error", `The server answered ${resp.status}.`);
  return resp.text();
}

const p = (projectId: string) => `/projects/${encodeURIComponent(projectId)}`;

export const auth = {
  status: () => request<AuthStatus>("GET", "/auth/status"),
  setup: (input: { code: string; username: string; password: string }) =>
    request<{ user: User }>("POST", "/auth/setup", input),
  login: (input: { username: string; password: string }) => request<{ user: User }>("POST", "/auth/login", input),
  logout: () => request<void>("POST", "/auth/logout", {}),
  changePassword: (input: { current_password: string; new_password: string }) =>
    request<void>("POST", "/auth/password", input),
};

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

  previewOpenApi: (
    pid: string,
    body: { content?: string; url?: string; base_url?: string; graphql?: boolean; auth_ref?: string | null },
  ) =>
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
  importBundle: (pid: string, content: string, onConflict: "fail" | "skip" | "replace", confirmFullAccess = false) =>
    request<BundleReport>("POST", `${p(pid)}/import/bundle`, {
      content,
      on_conflict: onConflict,
      confirm_full_access: confirmFullAccess,
    }),

  listAuth: (pid: string) => request<AuthConfig[]>("GET", `${p(pid)}/auth-configs`),
  createAuth: (pid: string, input: AuthConfigInput) => request<AuthConfig>("POST", `${p(pid)}/auth-configs`, input),
  updateAuth: (pid: string, id: string, input: AuthConfigInput) =>
    request<AuthConfig>("PUT", `${p(pid)}/auth-configs/${id}`, input),
  deleteAuth: (pid: string, id: string) => request<void>("DELETE", `${p(pid)}/auth-configs/${id}`),

  oauthRequest: (id: string) => request<ConsentInfo>("GET", `/oauth/requests/${encodeURIComponent(id)}`),
  oauthApprove: (id: string) => request<{ redirect: string }>("POST", `/oauth/requests/${encodeURIComponent(id)}/approve`, {}),
  oauthDeny: (id: string) => request<{ redirect: string }>("POST", `/oauth/requests/${encodeURIComponent(id)}/deny`, {}),
  listConnectedApps: (pid: string) => request<ConnectedApp[]>("GET", `${p(pid)}/connected-apps`),
  disconnectApp: (pid: string, id: string) => request<void>("DELETE", `${p(pid)}/connected-apps/${id}`),
  listUsers: () => request<UserView[]>("GET", "/users"),
  createUser: (input: { username: string; password: string; role: Role }) => request<UserView>("POST", "/users", input),
  setRole: (id: string, role: Role) => request<UserView>("PUT", `/users/${id}`, { role }),
  resetPassword: (id: string, password: string) => request<void>("PUT", `/users/${id}/password`, { password }),
  deleteUser: (id: string) => request<void>("DELETE", `/users/${id}`),
  setAccess: (userId: string, pid: string, access: Access | null) =>
    request<void>("PUT", `/users/${userId}/access/${pid}`, { access }),
  projectPeople: (pid: string) => request<ProjectPerson[]>("GET", `${p(pid)}/people`),
  listKeys: (pid: string) => request<ApiKey[]>("GET", `${p(pid)}/api-keys`),
  createKey: (pid: string, input: ApiKeyInput) =>
    request<{ key: ApiKey; secret: string }>("POST", `${p(pid)}/api-keys`, input),
  deleteKey: (pid: string, id: string) => request<void>("DELETE", `${p(pid)}/api-keys/${id}`),
  oauthStatus: (pid: string) => request<OAuthStatus[]>("GET", `${p(pid)}/oauth`),
  oauthStart: (pid: string, id: string) =>
    request<{ authorize_url: string; redirect_uri: string }>("POST", `${p(pid)}/auth-configs/${id}/oauth/start`, {}),
  oauthDisconnect: (pid: string, id: string) => request<void>("DELETE", `${p(pid)}/auth-configs/${id}/oauth`),

  listConnections: (pid: string) => request<DbConnection[]>("GET", `${p(pid)}/db-connections`),
  listResources: (pid: string) => request<ResourceDef[]>("GET", `${p(pid)}/resources`),
  createResource: (pid: string, input: ResourceInput) => request<ResourceDef>("POST", `${p(pid)}/resources`, input),
  updateResource: (pid: string, id: string, input: ResourceInput) =>
    request<ResourceDef>("PUT", `${p(pid)}/resources/${encodeURIComponent(id)}`, input),
  deleteResource: (pid: string, id: string) => request<void>("DELETE", `${p(pid)}/resources/${encodeURIComponent(id)}`),
  listPrompts: (pid: string) => request<PromptDef[]>("GET", `${p(pid)}/prompts`),
  createPrompt: (pid: string, input: PromptInput) => request<PromptDef>("POST", `${p(pid)}/prompts`, input),
  updatePrompt: (pid: string, id: string, input: PromptInput) =>
    request<PromptDef>("PUT", `${p(pid)}/prompts/${encodeURIComponent(id)}`, input),
  deletePrompt: (pid: string, id: string) => request<void>("DELETE", `${p(pid)}/prompts/${encodeURIComponent(id)}`),
  mcpResources: (pid: string) => request<McpResourceView[]>("GET", `${p(pid)}/mcp-resources`),
  readMcpResource: (pid: string, uri: string) =>
    request<{ uri: string; mime_type: string; text: string }>("GET", `${p(pid)}/mcp-resources/read?uri=${encodeURIComponent(uri)}`),
  createConnection: (pid: string, input: DbConnectionInput, confirmFullAccess = false) =>
    request<DbConnection>("POST", `${p(pid)}/db-connections`, { ...input, confirm_full_access: confirmFullAccess }),
  updateConnection: (pid: string, id: string, input: DbConnectionInput, confirmFullAccess = false) =>
    request<DbConnection>("PUT", `${p(pid)}/db-connections/${id}`, { ...input, confirm_full_access: confirmFullAccess }),
  listAudit: (pid: string, before?: string) =>
    request<AuditEntry[]>("GET", `${p(pid)}/audit?limit=100${before ? `&before=${encodeURIComponent(before)}` : ""}`),
  listCalls: (pid: string, filter: CallFilter, before?: string) => {
    const q = new URLSearchParams({ limit: "100" });
    for (const [k, v] of Object.entries(filter)) if (v) q.set(k, v);
    if (before) q.set("before", before);
    return request<CallEntry[]>("GET", `${p(pid)}/calls?${q}`);
  },
  listApprovals: () => request<PendingApproval[]>("GET", "/approvals"),
  approve: (id: string) => request<void>("POST", `/approvals/${id}/approve`, {}),
  deny: (id: string) => request<void>("POST", `/approvals/${id}/deny`, {}),
  callSummary: (pid: string, window: CallWindow) =>
    request<CallSummary>("GET", `${p(pid)}/calls/summary?window=${window}`),
  deleteConnection: (pid: string, id: string) => request<void>("DELETE", `${p(pid)}/db-connections/${id}`),
  testConnection: (pid: string, id: string) =>
    request<ConnectionReport>("POST", `${p(pid)}/db-connections/${id}/test`, {}),
  addExplorationTools: (pid: string, id: string, modes: ExplorationMode[]) =>
    request<{ created: Tool[]; existing: string[] }>("POST", `${p(pid)}/db-connections/${id}/exploration-tools`, { modes }),
};
