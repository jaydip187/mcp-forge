# Guide animation

## Intent
Make the guide (`site/docs/index.html`) feel alive while reading, and help
readers picture the three hardest ideas, in the friendly style of the site
restyle (`2026-11-24-site-friendly-restyle-design.md`).

## Approaches
- A. Polish only (scroll reveals, progress bar, copy buttons, TOC motion).
- B. Teaching diagrams only, in a few sections.
- **A everywhere + B in three sections (picked).** Polish is cheap; the three
  diagrams cover the ideas text explains worst.

## Goals
1. **Polish (whole guide):**
   - Reading-progress bar under the sticky header (gradient, scales with scroll).
   - Each section's content eases up into view once as it scrolls in.
   - Copy button on every `pre.code` block (reuses the home page's copy
     behaviour and its "Copied" pop).
   - TOC: the current-section marker slides between entries instead of jumping.
   - Headings: a `#` anchor link appears on hover with a small wiggle.
2. **Teaching diagrams (3 sections)**, each a figure that plays once when
   scrolled into view and has a Replay button:
   - **GitHub tool** (after "Shape the response"): the agent's call → GET to
     GitHub → a tall ~100-field response → the jq shape squeezes it → 4 fields
     reach the agent.
   - **Safe SQL** (after "3. What the agent gets"): the agent sends
     `customer_id = 42` → the value travels in its own lane, separate from the
     SQL text → rows come back. Second beat: `"42; DROP TABLE orders"` is
     stopped at the integer type check and never reaches the database.
   - **Approvals**: the call waits in a queue with a countdown → a person
     clicks Approve → the request goes out; a ghost second run shows a timeout
     turning into a refusal.

## Non-goals / anti-goals
- No copy rewrites; diagrams sit beside existing text, not in place of it.
- No libraries or build step; plain HTML/CSS/JS in the shared files.
- Must not slow reading: nothing loops forever, nothing moves the text you
  are reading (reveals happen only below the fold, once).

## Constraints
- Under `prefers-reduced-motion: reduce`: no movement; diagrams show their
  final state, progress bar still works (it's position, not motion).
- Without JS: all content visible, diagrams shown in final state.
- Works in both themes and at 360px with no horizontal scroll.
- Diagrams have a text alternative (`role="img"` + `aria-label`, or a caption).

## Left to the builder
Exact timings, diagram markup, which shared CSS to reuse from the home page
(forge/tour panes, `line-in`, `draw`, `press` keyframes).
