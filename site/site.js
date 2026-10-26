// Hero examples, install tabs, copy buttons, scroll reveals and the docs
// contents highlight. Everything here is optional: without it the page is
// static and complete.
const reduceMotion = matchMedia("(prefers-reduced-motion: reduce)").matches;

// Hero: replay one example's timeline. The pressed button's progress bar
// (a CSS animation) ends after a few seconds and moves on to the next one,
// so hover and focus pause both together.
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

for (const button of document.querySelectorAll(".copy")) {
  button.addEventListener("click", async () => {
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
    setTimeout(() => {
      button.textContent = "Copy";
      button.classList.remove("done");
    }, 2000);
  });
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
  const mark = () => {
    const current = [...byId.keys()].find((id) => visible.has(id));
    for (const [id, a] of byId) a.setAttribute("aria-current", id === current);
  };
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
