/**
 * Path filter for the import preview.
 * - With `*`, `**` or `?` it's a glob over the whole path: `*` and `?` stay
 *   within one segment, `**` crosses segments.
 * - Without wildcards it's a case-insensitive substring match.
 */
export function pathMatcher(pattern: string): (path: string) => boolean {
  const p = pattern.trim();
  if (!p) return () => true;
  if (!/[*?]/.test(p)) {
    const needle = p.toLowerCase();
    return (path) => path.toLowerCase().includes(needle);
  }
  let re = "";
  for (let i = 0; i < p.length; i++) {
    const c = p[i]!;
    if (c === "*") {
      if (p[i + 1] === "*") {
        re += ".*";
        i++;
      } else {
        re += "[^/]*";
      }
    } else if (c === "?") {
      re += "[^/]";
    } else {
      re += c.replace(/[.+^${}()|[\]\\]/g, "\\$&");
    }
  }
  const rx = new RegExp(`^${re}$`, "i");
  return (path) => rx.test(path);
}
