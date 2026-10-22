/** Light/dark preference, kept in this browser only. "system" follows the OS. */
export type ThemePref = "system" | "light" | "dark";

const KEY = "mcp-forge.theme";
const media = window.matchMedia("(prefers-color-scheme: dark)");
const listeners = new Set<() => void>();

export function getTheme(): ThemePref {
  try {
    const v = localStorage.getItem(KEY);
    return v === "light" || v === "dark" ? v : "system";
  } catch {
    return "system";
  }
}

export function setTheme(pref: ThemePref) {
  try {
    if (pref === "system") localStorage.removeItem(KEY);
    else localStorage.setItem(KEY, pref);
  } catch {
    /* storage unavailable: applies to this page only */
  }
  applyTheme(pref);
}

function applyTheme(pref = getTheme()) {
  const dark = pref === "dark" || (pref === "system" && media.matches);
  document.documentElement.classList.toggle("dark", dark);
  listeners.forEach((fn) => fn());
}

/** Applies the saved preference now and whenever the OS setting changes. */
export function initTheme() {
  applyTheme();
  media.addEventListener("change", () => applyTheme());
}

export function onThemeChange(fn: () => void): () => void {
  listeners.add(fn);
  return () => listeners.delete(fn);
}
