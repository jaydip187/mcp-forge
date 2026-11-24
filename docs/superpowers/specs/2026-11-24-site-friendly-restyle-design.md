# Site restyle: friendly, playful, light + dark

## Intent
Restyle the public site (`site/`) in the visual spirit of Josh Comeau's
"A Friendly Introduction to SVG": warm, approachable, teach-by-playing.
Keep it a product landing page. Style only: no copying of that site's
fonts, logo, illustrations or wording.

## Approaches
- **A. Restyle the landing page (picked).** Same sections and copy, new look,
  plus a couple of interactive widgets. Smaller, keeps the product pitch.
- B. Rewrite as a long-form article with TOC and a demo per section. Bigger;
  copy rewrite. Not picked.

## Goals
- Light theme by default (warm off-white, deep ink, blue/indigo primary,
  pink secondary); dark theme (deep navy, gold primary, pink secondary).
  Follows `prefers-color-scheme`, with a header toggle remembered in
  localStorage (try/catch).
- Friendly type: rounded/humanist sans for body and headings, bold
  headings, generous line-height, ~68ch reading width for prose.
- Callout boxes (info / warning / success) with a round icon badge sitting
  on the corner; used for existing notes (read-only DB, fail-closed, etc.).
- Code blocks: soft-dark panels, rounded, gentle syntax colors per theme.
- Playful micro-interactions: hover lift/wiggle on cards and icons,
  springy buttons, underline-wave links. All disabled under
  `prefers-reduced-motion`.
- One or two interactive widgets in "play with it" style:
  1. **Response trimmer** (in the "One API answer is a hundred fields"
     section): a JSON response with checkboxes/slider choosing fields; the
     trimmed output and a token/byte counter update live.
  2. Existing feature tour / hero loop kept, restyled to the new look.
- Same style reaches `docs/index.html` and `404.html` via the shared
  `style.css` / `site.js`.

## Non-goals / anti-goals
- No copy rewrite, no section restructuring, no new pages.
- No build step, frameworks, or new JS dependencies; stays static HTML/CSS/JS.
- Must not look like a clone of joshwcomeau.com.
- Must not regress accessibility (contrast AA in both themes, focus rings,
  keyboard use of widgets) or mobile layout (no horizontal scroll at 360px).

## Constraints
- GitHub Pages, served under `/mcp-forge/`.
- Google Fonts only for webfonts (builder's call on families, e.g. a rounded
  sans like Nunito/Quicksand-class for headings + readable body sans, and a
  mono).
- `og.png` and `favicon.svg` may be refreshed to match, builder's call.

## Left to the builder
Exact palette values, font choice, widget markup, which notes become
callouts, verification in browser (light, dark, mobile, reduced motion).
