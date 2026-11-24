# Guide animation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Reading polish across the guide plus three teaching animations.

**Architecture:** Static page. Markup for the three figures goes in `site/docs/index.html`; behaviour in shared `site/site.js` (only runs where its hooks exist); styles in `site/style.css`. Every animated element's *base* style is its final state; keyframes animate *from* the start, so no-JS and reduced-motion readers see the finished picture.

**Tech Stack:** HTML, CSS keyframes, vanilla JS (IntersectionObserver, rAF).

**Spec:** `docs/superpowers/specs/2026-11-24-docs-animation-design.md`

## Global Constraints
- No libraries, no copy rewrites, nothing loops forever, nothing above the fold moves on load.
- `prefers-reduced-motion: reduce` → no movement, final states; progress bar still tracks.
- Both themes, 360px without horizontal scroll, figures have `role="img"` + `aria-label`.

## Review Focus
- Deep link (`/docs/#sql`): sections at/above the landing point must not be hidden waiting for a reveal.
- Copy button must copy only the code, not the word "Copy".
- Replay pressed mid-animation restarts cleanly.
- TOC marker position after window resize / mobile `<details>` toggle.
- Home page must keep working: copy buttons there use the same handler.

---

### Task 1: Polish (`site.js`, `style.css`, docs header)
- [ ] Header: `<div class="progress" aria-hidden="true"><i></i></div>` in docs `.top`; JS sets `--p` (0–1) on scroll via rAF; CSS `transform: scaleX(var(--p))`.
- [ ] Reveals: JS adds `.pending` to `.doc section` whose top is below the viewport at load; IntersectionObserver swaps it for `.in-view` once. CSS only under no-preference.
- [ ] Copy: switch the copy handler to one delegated `document` click on `.copy`; JS appends `<button class="copy">` to each `pre.code` (adds `.has-copy`).
- [ ] TOC: JS inserts `.toc-marker` into `.toc ol`; `mark()` sets its `top`/`height` to the current link; spring transition; recompute on resize and `toggle`.
- [ ] Anchors: JS adds `<a class="anchor" href="#<section id>" aria-label="Link to this section">#</a>` to each `.doc section > h2`; hover wiggle.
- [ ] Verify in browser: scroll bar moves, sections ease in, copy copies code only, marker slides, home copy still works.

### Task 2: Three lessons (`docs/index.html`, `site.js`, `style.css`)
Shared: `figure.lesson[data-lesson]` with `.lesson-replay` button. JS adds `.armed` (motion allowed), plays (`.is-playing`) once at 40% visibility; Replay = remove class, reflow, add. Beats use `--d` delays.
- [ ] **GitHub tool** after the shaped-response `pre` in `#http-tool`: request card → response list (12 field names + "+ 88 more"; 4 kept fields highlight, others fade) → jq funnel → result card.
- [ ] **Safe SQL** after "3. What the agent gets" `pre` in `#sql`: stations Agent | Type check | Database; SQL-text lane fixed; value chip `42` travels to Database and rows return; then chip `"42; DROP TABLE orders"` stops at Type check, shakes, turns red with "refused: must be an integer".
- [ ] **Approvals** after the list in `#approvals`: run 1 call → waiting bar shrinks → Approve press → "✓ approved · request sent"; run 2 bar runs out → "✗ no answer in 300 s · refused, never sent".
- [ ] Verify: each plays on scroll, Replay works, reduced-motion shows final states, 360px no overflow, both themes.
