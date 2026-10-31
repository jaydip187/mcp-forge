/** Human label for an imported spec's format. */
export function specKindLabel(kind: string, openapi?: string): string {
  switch (kind) {
    case "swagger2":
      return "Swagger 2.0";
    case "postman":
      return "Postman collection";
    case "har":
      return "HAR capture";
    case "curl":
      return "curl commands";
    default:
      return openapi ? `OpenAPI ${openapi}` : "OpenAPI 3";
  }
}
