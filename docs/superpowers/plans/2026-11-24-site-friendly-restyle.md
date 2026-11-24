# Site friendly restyle Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Light + dark, friendly, playful restyle of `site/` plus a live response-trimmer widget.

**Architecture:** Static site. All three pages share `site/style.css` and `site/site.js`. Theme is a `data-theme` attribute on `<html>` set by a tiny inline head script (localStorage, else system); CSS tokens switch on it.

**Tech Stack:** HTML, CSS (custom properties, color-mix), vanilla JS. Google Fonts.

**Spec:** `docs/superpowers/specs/2026-11-24-site-friendly-restyle-design.md`

## Global Constraints
- No build step, no JS libraries. Pages work fully without JS (widget shows its default state).
- WCAG AA text contrast in both themes; visible focus rings.
- No horizontal scroll at 360px. All motion behind `prefers-reduced-motion: no-preference`.
- Do not change copy or section order (new widget + callout wrappers are the only markup additions).

## Review Focus
- localStorage throws (private mode): toggle still works for the session, no console error.
- No flash of wrong theme on load (inline script runs before CSS paints).
- Trimmer with zero fields checked: shows `{}` and a sensible size, not NaN.
- Hard-coded old teal/lime rgb() values left in CSS would look wrong on light: grep must find none.
- Brand SVG stroke uses `var(--bg)`; must stay visible in both themes.

---

### Task 1: Tokens, themes, typography (`site/style.css`)
- [ ] Replace `:root` with light tokens; `[data-theme="dark"]` + `@media (prefers-color-scheme: dark) :root:not([data-theme="light"])` with dark tokens. Names: `--bg --paper --paper-2 --card --ink --muted --rule --primary --on-primary --accent --grad --mist --focus`, code tokens fixed dark in both.
- [ ] Rename uses of `--teal`→`--primary`, `--teal-ink`→`--on-primary`, `--lime`→`--accent`; replace every literal `rgb(79 224 200 …)`, `rgb(184 240 138 …)`, `rgb(255 255 255 …)` with `color-mix` on tokens.
- [ ] Fonts: Nunito (display, 700/800), Atkinson Hyperlegible Next (body), JetBrains Mono. Update font `<link>` in all 3 pages.
- [ ] Verify: `grep -E "79 224 200|184 240 138|--teal|--lime" site/style.css` → no output.

### Task 2: Friendly components (`site/style.css`)
- [ ] `.callout` (+ `.info`, `.warn`) with a round icon badge on the top-left corner; `.aside` in docs styled as `.callout.info`.
- [ ] Springy buttons/cards (hover lift, overshoot easing), wavy underline on link hover, sparkles beside hero `.grad`, softer bands.
- [ ] Wrap index notes: install note → `callout info`, DB write note → `callout warn`.

### Task 3: Theme toggle (`site/site.js`, 3 HTML heads/headers)
- [ ] Inline head script: `try{t=localStorage.getItem('theme')}catch{}`; set `data-theme` to t or system.
- [ ] Header button `.theme-toggle` (sun/moon SVG, `aria-label="Switch to dark theme"` updated on toggle); click flips, saves in try/catch, updates `theme-color` meta.
- [ ] Verify in browser: toggle flips, reload keeps choice.

### Task 4: Response trimmer (`site/index.html` curate band, `site/site.js`, `site/style.css`)
- [ ] Markup `figure.trimmer[data-trimmer]`: checkbox per field (`data-path`, `data-out`, `data-value` JSON); `output` for jq, JSON, size meter. Defaults checked: full_name→name, stargazers_count→stars, language.
- [ ] JS `renderTrimmer(root)`: jq = `{a, b: .c}` (shorthand when out==path), JSON.stringify of picked, bytes via `new Blob([s]).size` vs full sample, ~tokens = ceil(bytes/4), bar width %. Zero picked → `{}`.
- [ ] Verify: toggle fields, numbers update; keyboard works.

### Task 5: Assets, verification, commit
- [ ] favicon.svg to new primary; theme-color metas.
- [ ] Browser check (browser-mcp): light, dark, 360px width, docs, 404.
- [ ] Commit.
