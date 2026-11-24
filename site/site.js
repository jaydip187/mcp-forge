// Hero examples, the feature tour, install tabs, copy buttons, counters, scroll reveals and the docs
// contents highlight. Everything here is optional: without it the page is
// static and complete.
const reduceMotion = matchMedia("(prefers-reduced-motion: reduce)").matches;

// Hero and feature tour: replay one example's timeline. The pressed button's
// progress bar (a CSS animation) ends after a few seconds and moves on to the
// next one, so hover and focus pause both together.
for (const forge of document.querySelectorAll("[data-forge]")) {
  const buttons = [...forge.querySelectorAll("[data-show]")];
  const show = (button) => {
    for (const b of buttons) {
      const on = b === button;
      b.setAttribute("aria-pressed", on);
      const stage = forge.querySelector(`[data-example="${b.dataset.show}"]`);
      stage.hidden = !on;
      stage.classList.remove("is-playing");
    }
    const stage = forge.querySelector(`[data-example="${button.dataset.show}"]`);
    void stage.offsetWidth; // restart the CSS animations
    stage.classList.add("is-playing");
  };
  forge.addEventListener("click", (e) => {
    const button = e.target.closest("[data-show]");
    if (!button) return;
    forge.classList.add("chosen"); // the reader picked one; stop cycling
    show(button);
  });
  forge.addEventListener("animationend", (e) => {
    if (e.animationName !== "progress" || reduceMotion) return;
    const i = buttons.findIndex((b) => b.getAttribute("aria-pressed") === "true");
    show(buttons[(i + 1) % buttons.length]);
  });
}

// Install tabs; arrow keys move between them.
for (const tabs of document.querySelectorAll("[data-tabs]")) {
  const list = [...tabs.querySelectorAll('[role="tab"]')];
  const select = (tab) => {
    for (const t of list) {
      const on = t === tab;
      t.setAttribute("aria-selected", on);
      t.tabIndex = on ? 0 : -1;
      document.getElementById(t.getAttribute("aria-controls")).hidden = !on;
    }
    tab.focus();
  };
  tabs.addEventListener("click", (e) => {
    const tab = e.target.closest('[role="tab"]');
    if (tab) select(tab);
  });
  tabs.addEventListener("keydown", (e) => {
    const i = list.indexOf(document.activeElement);
    if (i < 0) return;
    if (e.key === "ArrowRight") select(list[(i + 1) % list.length]);
    if (e.key === "ArrowLeft") select(list[(i - 1 + list.length) % list.length]);
  });
}

// Guide code blocks get their own copy button.
for (const pre of document.querySelectorAll("pre.code")) {
  if (!pre.querySelector("code")) continue;
  const button = document.createElement("button");
  button.type = "button";
  button.className = "copy";
  button.textContent = "Copy";
  pre.classList.add("has-copy");
  pre.append(button);
}

// One handler for every copy button, on the home page and in the guide.
document.addEventListener("click", async (e) => {
  const button = e.target.closest(".copy");
  if (!button) return;
  {
    const code = button.parentElement.querySelector("code");
    try {
      await navigator.clipboard.writeText(code.textContent);
      button.textContent = "Copied";
      button.classList.add("done");
    } catch {
      button.textContent = "Press Ctrl+C";
      const range = document.createRange();
      range.selectNodeContents(code);
      getSelection().removeAllRanges();
      getSelection().addRange(range);
    }
    clearTimeout(button.resetTimer);
    button.resetTimer = setTimeout(() => {
      button.textContent = "Copy";
      button.classList.remove("done");
    }, 2000);
  }
});

// The hero's live example replays every few seconds while it's on screen.
for (const demo of document.querySelectorAll("[data-demo]")) {
  const play = () => {
    demo.classList.remove("play");
    void demo.offsetWidth; // restart the CSS animations
    demo.classList.add("play");
  };
  play();
  if (reduceMotion || !("IntersectionObserver" in window)) continue;
  let timer = null;
  new IntersectionObserver(([entry]) => {
    clearInterval(timer);
    if (entry.isIntersecting) timer = setInterval(play, 9000);
  }).observe(demo);
}

// Numbers count up once when they scroll into view.
const counters = document.querySelectorAll("[data-count]");
if (!reduceMotion && "IntersectionObserver" in window) {
  const count = new IntersectionObserver(
    (entries) => {
      for (const entry of entries) {
        if (!entry.isIntersecting) continue;
        count.unobserve(entry.target);
        const el = entry.target;
        const to = Number(el.dataset.count);
        const start = performance.now();
        const step = (now) => {
          const t = Math.min(1, (now - start) / 900);
          el.textContent = String(Math.round(to * (1 - (1 - t) ** 3)));
          if (t < 1) requestAnimationFrame(step);
        };
        requestAnimationFrame(step);
      }
    },
    { threshold: 0.6 },
  );
  for (const el of counters) count.observe(el);
}

// Sequences build once when they scroll into view.
const reveals = document.querySelectorAll("[data-reveal]");
if ("IntersectionObserver" in window) {
  const seen = new IntersectionObserver(
    (entries) => {
      for (const entry of entries) {
        if (!entry.isIntersecting) continue;
        entry.target.classList.add("in-view");
        seen.unobserve(entry.target);
      }
    },
    { threshold: 0.25 },
  );
  for (const list of reveals) {
    [...list.children].forEach((li, i) => li.style.setProperty("--i", i));
    seen.observe(list);
  }
} else {
  for (const list of reveals) list.classList.add("in-view");
}

// Docs: mark the contents link for the section being read.
const tocLinks = [...document.querySelectorAll('.toc a[href^="#"]')];
if (tocLinks.length && "IntersectionObserver" in window) {
  const byId = new Map(tocLinks.map((a) => [a.hash.slice(1), a]));
  const visible = new Set();
  // A marker that slides to the current entry instead of jumping.
  const list = document.querySelector(".toc ol");
  const marker = document.createElement("span");
  marker.className = "toc-marker";
  marker.setAttribute("aria-hidden", "true");
  list?.prepend(marker);
  let current = null;
  const place = () => {
    const a = current && byId.get(current);
    marker.classList.toggle("on", Boolean(a && a.offsetParent));
    if (!a || !a.offsetParent) return;
    marker.style.setProperty("--top", `${a.offsetTop}px`);
    marker.style.setProperty("--h", `${a.offsetHeight}px`);
  };
  const mark = () => {
    current = [...byId.keys()].find((id) => visible.has(id)) ?? null;
    for (const [id, a] of byId) a.setAttribute("aria-current", id === current);
    place();
  };
  addEventListener("resize", place);
  document.querySelector(".toc details")?.addEventListener("toggle", place);
  const spy = new IntersectionObserver(
    (entries) => {
      for (const entry of entries) {
        if (entry.isIntersecting) visible.add(entry.target.id);
        else visible.delete(entry.target.id);
      }
      mark();
    },
    { rootMargin: "-80px 0px -55% 0px" },
  );
  for (const id of byId.keys()) {
    const section = document.getElementById(id);
    if (section) spy.observe(section);
  }
}

// Theme: the head script already set data-theme before paint; the toggle
// flips it and remembers the choice. Storage can be blocked, so the choice
// then lasts only for this page view.
const root = document.documentElement;
const stored = () => { try { return localStorage.getItem("theme"); } catch { return null; } };
const applyTheme = (theme) => {
  root.dataset.theme = theme;
  const bg = getComputedStyle(root).getPropertyValue("--bg").trim();
  for (const meta of document.querySelectorAll('meta[name="theme-color"]')) meta.content = bg;
  for (const button of document.querySelectorAll(".theme-toggle")) {
    const next = theme === "dark" ? "light" : "dark";
    button.setAttribute("aria-label", `Switch to ${next} theme`);
    button.title = `Switch to ${next} theme`;
  }
};
applyTheme(root.dataset.theme === "dark" ? "dark" : "light");
for (const button of document.querySelectorAll(".theme-toggle")) {
  button.addEventListener("click", () => {
    const theme = root.dataset.theme === "dark" ? "light" : "dark";
    try { localStorage.setItem("theme", theme); } catch {}
    applyTheme(theme);
  });
}
// Follow the system while the reader hasn't picked a theme.
matchMedia("(prefers-color-scheme: dark)").addEventListener("change", (e) => {
  if (!stored()) applyTheme(e.matches ? "dark" : "light");
});

// Response trimmer: each ticked field becomes part of the jq shape, and the
// agent's answer and its size update. Sizes are of this sample only.
for (const trimmer of document.querySelectorAll("[data-trimmer]")) {
  const boxes = [...trimmer.querySelectorAll("input[data-path]")];
  const defaults = boxes.map((b) => b.defaultChecked);
  const jq = trimmer.querySelector("[data-trim-jq]");
  const json = trimmer.querySelector("[data-trim-json]");
  const size = trimmer.querySelector("[data-trim-size]");
  const bar = trimmer.querySelector(".trim-bar i");
  const bytes = (value) => new Blob([JSON.stringify(value)]).size;
  const field = (b) => ({ path: b.dataset.path, out: b.dataset.out || b.dataset.path, value: JSON.parse(b.dataset.value) });
  const full = Object.fromEntries(boxes.map((b) => [b.dataset.path, field(b).value]));
  const fullBytes = bytes(full);
  const render = () => {
    const picked = boxes.filter((b) => b.checked).map(field);
    jq.textContent = `{${picked.map((f) => (f.out === f.path ? f.path : `${f.out}: .${f.path}`)).join(", ")}}`;
    const answer = Object.fromEntries(picked.map((f) => [f.out, f.value]));
    json.textContent = JSON.stringify(answer, null, 2);
    const kept = bytes(answer);
    const saved = Math.round((1 - kept / fullBytes) * 100);
    size.innerHTML = `<strong>${saved}% smaller</strong> · ${kept} of ${fullBytes} bytes, about ${Math.ceil(kept / 4)} tokens instead of ${Math.ceil(fullBytes / 4)}`;
    bar.style.setProperty("--w", `${Math.max(1.5, (kept / fullBytes) * 100)}%`);
  };
  trimmer.addEventListener("change", render);
  trimmer.addEventListener("click", (e) => {
    const action = e.target.closest("[data-trim]")?.dataset.trim;
    if (!action) return;
    boxes.forEach((b, i) => { b.checked = action === "all" ? true : action === "none" ? false : defaults[i]; });
    render();
  });
  render();
}

// Guide: a reading-progress bar under the header.
const progress = document.querySelector(".progress");
if (progress) {
  let queued = false;
  const update = () => {
    queued = false;
    const max = document.documentElement.scrollHeight - innerHeight;
    progress.style.setProperty("--p", max > 0 ? Math.min(1, scrollY / max) : 0);
  };
  addEventListener("scroll", () => { if (!queued) { queued = true; requestAnimationFrame(update); } }, { passive: true });
  addEventListener("resize", update);
  update();
}

// Guide: sections below the fold ease in once as they arrive. Sections the
// reader can already see (or has deep-linked past) are never hidden.
const docSections = [...document.querySelectorAll(".doc section")];
if (docSections.length && !reduceMotion && "IntersectionObserver" in window) {
  const arrive = new IntersectionObserver(
    (entries) => {
      for (const entry of entries) {
        if (!entry.isIntersecting) continue;
        entry.target.classList.replace("pending", "in-view");
        arrive.unobserve(entry.target);
      }
    },
    { rootMargin: "0px 0px -8% 0px" },
  );
  // A deep link jumps after this runs, so the target and everything above
  // it stay visible; anything else on screen once the jump is done is shown
  // at once, without the ease-in.
  let target = null;
  try { target = location.hash && document.querySelector(location.hash)?.closest(".doc section"); } catch {}
  const last = target ? docSections.indexOf(target) : -1;
  docSections.forEach((section, i) => {
    if (i <= last || section.getBoundingClientRect().top < innerHeight) return;
    section.classList.add("pending");
    arrive.observe(section);
  });
  const settle = () => {
    for (const section of document.querySelectorAll(".doc section.pending")) {
      if (section.getBoundingClientRect().top >= innerHeight) continue;
      section.classList.remove("pending");
      arrive.unobserve(section);
    }
  };
  addEventListener("load", () => requestAnimationFrame(settle), { once: true });
  addEventListener("hashchange", settle);
}

// Guide: a "#" link beside each section heading.
for (const h2 of document.querySelectorAll(".doc section[id] > h2")) {
  const a = document.createElement("a");
  a.className = "anchor";
  a.href = `#${h2.parentElement.id}`;
  a.setAttribute("aria-label", `Link to the section ${h2.textContent}`);
  a.textContent = "#";
  h2.append(a);
}

// Guide lessons: each plays once when it is mostly on screen; Replay
// restarts it. Without motion the finished picture simply stays.
for (const lesson of document.querySelectorAll("[data-lesson]")) {
  const play = () => {
    lesson.classList.remove("is-playing");
    void lesson.offsetWidth; // restart the CSS animations
    lesson.classList.add("is-playing");
  };
  lesson.querySelector(".lesson-replay")?.addEventListener("click", play);
  if (reduceMotion || !("IntersectionObserver" in window)) continue;
  lesson.classList.add("armed");
  const watch = new IntersectionObserver(
    ([entry]) => {
      if (!entry.isIntersecting) return;
      watch.disconnect();
      play();
    },
    { threshold: 0.4 },
  );
  watch.observe(lesson);
}

// Guide search: filters the contents list by what each section says, not
// only its title. "/" focuses the box, Enter opens the first match, Escape
// clears. Built here so readers without scripts never see a dead box.
const tocList = document.querySelector(".toc ol");
if (tocList) {
  const entries = [...tocList.querySelectorAll('a[href^="#"]')]
    .map((a) => {
      const section = document.getElementById(a.hash.slice(1));
      if (!section) return null;
      const subs = [...section.querySelectorAll("h3")].map((h) => ({
        text: h.textContent.trim(),
        lower: h.textContent.toLowerCase(),
        el: h,
      }));
      return { a, li: a.closest("li"), section, text: section.textContent.toLowerCase(), subs };
    })
    .filter(Boolean);

  const box = document.createElement("div");
  box.className = "toc-search";
  box.innerHTML =
    '<input id="toc-q" type="search" placeholder="Search the guide" aria-label="Search the guide" autocomplete="off" spellcheck="false" aria-describedby="toc-count">' +
    '<kbd aria-hidden="true">/</kbd>' +
    '<p id="toc-count" class="toc-count" aria-live="polite"></p>';
  tocList.before(box);
  const input = box.querySelector("input");
  const count = box.querySelector(".toc-count");
  let first = null;

  const run = () => {
    const words = input.value.toLowerCase().split(/\s+/).filter(Boolean);
    box.classList.toggle("has-q", words.length > 0);
    first = null;
    let shown = 0;
    for (const e of entries) {
      e.li.querySelector(".toc-hits")?.remove();
      const hit = words.every((w) => e.text.includes(w));
      e.li.hidden = !hit;
      if (!hit) continue;
      shown++;
      first ??= e.section;
      if (!words.length) continue;
      // Point at the subsections that match, so a hit deep in a long section is easy to find.
      const subs = e.subs.filter((s) => words.some((w) => s.lower.includes(w)));
      if (subs.length) {
        const ul = document.createElement("ul");
        ul.className = "toc-hits";
        for (const s of subs.slice(0, 4)) {
          const li = document.createElement("li");
          const link = document.createElement("a");
          if (!s.el.id) s.el.id = `${e.section.id}-${s.text.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "")}`;
          link.href = `#${s.el.id}`;
          link.textContent = s.text;
          li.append(link);
          ul.append(li);
        }
        e.li.append(ul);
      }
    }
    count.textContent = !words.length
      ? ""
      : shown
        ? `${shown} ${shown === 1 ? "section" : "sections"}`
        : "Nothing found. Try fewer or shorter words.";
    dispatchEvent(new Event("resize")); // re-place the contents marker
  };

  input.addEventListener("input", run);
  input.addEventListener("keydown", (ev) => {
    if (ev.key === "Enter" && first) {
      ev.preventDefault();
      first.scrollIntoView({ behavior: reduceMotion ? "auto" : "smooth" });
      history.replaceState(null, "", `#${first.id}`);
    } else if (ev.key === "Escape") {
      input.value = "";
      run();
    }
  });
  addEventListener("keydown", (ev) => {
    if (ev.key !== "/" || ev.ctrlKey || ev.metaKey || ev.altKey) return;
    const t = ev.target;
    if (t instanceof HTMLElement && (t.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(t.tagName))) return;
    ev.preventDefault();
    input.closest("details")?.setAttribute("open", "");
    input.focus();
  });
}
