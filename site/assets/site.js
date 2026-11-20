// Tidemap website. No framework: the hero canvas, the gallery viewer, the download pick
// and the guide's contents. Every part works without the others.

(() => {
  "use strict";

  const $ = (selector, root = document) => root.querySelector(selector);
  const $$ = (selector, root = document) => [...root.querySelectorAll(selector)];
  const reduced = matchMedia("(prefers-reduced-motion: reduce)");
  const SOFT = "linear(0, 0.068, 0.21, 0.37, 0.516, 0.638, 0.734, 0.808, 0.863, 0.903, 0.932, 0.952, 0.967, 0.977, 0.985, 0.99, 0.993, 0.995, 0.997, 0.998, 0.999, 1)";
  const OUT = "cubic-bezier(0.2, 0.7, 0.2, 1)";
  const typing = (target) => target instanceof Element && target.closest("input, textarea, select, [contenteditable]");

  // ------------------------------------------------------------------ platform

  const ua = navigator.userAgent;
  const touchMac = /Macintosh/.test(ua) && navigator.maxTouchPoints > 1; // iPadOS
  const mobile = /Android|iPhone|iPad|iPod/.test(ua) || touchMac;
  const platform = (navigator.userAgentData && navigator.userAgentData.platform) || navigator.platform || ua;
  const mac = !mobile && /Mac/i.test(platform);
  const os = mobile ? null : /Win/i.test(platform) ? "windows" : mac ? "mac" : /Linux|X11/i.test(platform) ? "linux" : null;

  // Shortcut labels: Ctrl on Windows and Linux, the Command key on a Mac.
  if (mac) {
    const symbols = { Ctrl: ["⌘", "Command"], Shift: ["⇧", "Shift"], Alt: ["⌥", "Option"] };
    for (const key of $$("kbd:not([data-literal])")) {
      const symbol = symbols[key.textContent];
      if (symbol) { key.textContent = symbol[0]; key.setAttribute("aria-label", symbol[1]); }
    }
  }

  // ------------------------------------------------------------------ download pick

  const RELEASE = "https://github.com/zokirjonkodirov/tidemap/releases/download/v0.4.0/";
  const BUILDS = {
    windows: { name: "Windows", file: "Tidemap-0.4.0-x86_64-pc-windows-msvc.zip", meta: "18.5 MB zip" },
    "mac-arm": { name: "macOS", file: "Tidemap-0.4.0-aarch64-apple-darwin.tar.gz", meta: "7.2 MB, for Apple silicon" },
    "mac-intel": { name: "macOS", file: "Tidemap-0.4.0-x86_64-apple-darwin.tar.gz", meta: "7.5 MB, for Intel Macs" },
    linux: { name: "Linux", file: "Tidemap-0.4.0-x86_64-unknown-linux-gnu.tar.gz", meta: "10.9 MB, x86_64" },
  };

  function pick(key, certain) {
    const build = BUILDS[key];
    const get = $("#get"), meta = $("#get-meta");
    if (!build || !get) return;
    get.href = RELEASE + build.file;
    get.textContent = `Download for ${build.name}`;
    get.removeAttribute("data-download");
    const other = key.startsWith("mac") && !certain
      ? ` <a href="#download">${key === "mac-arm" ? "Intel Mac?" : "Apple silicon?"}</a>`
      : "";
    meta.innerHTML = `Version 0.4.0 · ${build.meta}. Runs without installing.${other}`;
    for (const row of $$(".platform")) {
      const mine = row.dataset.os === key;
      row.classList.toggle("pick", mine);
      $(".yours", row).hidden = !mine;
    }
  }

  if (os === "windows" || os === "linux") pick(os, true);
  else if (os === "mac") {
    pick("mac-arm", false);
    const data = navigator.userAgentData;
    if (data && data.getHighEntropyValues) {
      data.getHighEntropyValues(["architecture"]).then((values) => {
        if (values.architecture === "x86") pick("mac-intel", true);
        else if (values.architecture === "arm") pick("mac-arm", true);
      }).catch(() => {});
    }
  } else if (mobile && $("#get-meta")) {
    $("#get-meta").textContent = "Tidemap is a desktop app for Windows, macOS and Linux.";
  }

  // ------------------------------------------------------------------ top bar edge

  const top = $(".top");
  if (top) {
    const onScroll = () => top.classList.toggle("scrolled", scrollY > 4);
    addEventListener("scroll", onScroll, { passive: true });
    onScroll();
  }

  // ------------------------------------------------------------------ hero canvas

  const win = $("#window");
  if (win) hero(win);

  function hero(win) {
    const canvas = $("#canvas", win);
    const stages = $$(".stage", canvas);
    const codes = $$(".code", win);
    const zoomLabel = $("#zoom", win);
    const sourceButton = $("#src-toggle", win);
    const exportLink = $("#export", win);
    const announce = $("#announce", win);
    const cams = stages.map(() => null);
    const touched = stages.map(() => false);
    let index = 0;
    let picked = null;

    win.addEventListener("animationend", () => win.classList.remove("enter"), { once: true });

    const svgOf = (i) => $("svg", stages[i]);
    const slug = (text) => text.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "");
    const label = (node) => node.textContent.replace(/\s+/g, " ").trim();
    const twin = (id) => (id.endsWith(" (end)") ? id.slice(0, -6) : `${id} (end)`);
    const nodesOf = (i) => $$(".node", svgOf(i)).filter((node) => !node.dataset.id.endsWith(" (end)"));

    // Camera ----------------------------------------------------------------

    function fit(i) {
      const width = +stages[i].dataset.w, height = +stages[i].dataset.h;
      const cw = canvas.clientWidth, ch = canvas.clientHeight;
      const padX = 24, padTop = 20, padBottom = 64;
      const room = Math.max(40, ch - padTop - padBottom);
      let s = Math.min((cw - 2 * padX) / width, room / height, 1.25);
      let x = (cw - width * s) / 2;
      const y = padTop + (room - height * s) / 2;
      // On a phone a wide diagram fitted to the width is too small to read: fit the height
      // instead, start at the left edge and let the reader swipe along it.
      if (s < 0.6 && cw < 600) {
        s = Math.min(0.75, room / height);
        x = padX;
      }
      return { x, y: Math.max(padTop, y), s };
    }

    function apply(i, animate) {
      const cam = cams[i];
      stages[i].classList.toggle("glide", !!animate && !reduced.matches);
      svgOf(i).style.transform = `translate(${cam.x}px, ${cam.y}px) scale(${cam.s})`;
      if (i === index) zoomLabel.textContent = `${Math.round(cam.s * 100)}%`;
    }

    function refit(i, animate) {
      cams[i] = fit(i);
      touched[i] = false;
      apply(i, animate);
    }

    function zoomBy(factor, cx = canvas.clientWidth / 2, cy = canvas.clientHeight / 2, animate = true) {
      const cam = cams[index];
      const s = Math.min(4, Math.max(0.2, cam.s * factor));
      cams[index] = { x: cx - (cx - cam.x) * (s / cam.s), y: cy - (cy - cam.y) * (s / cam.s), s };
      touched[index] = true;
      apply(index, animate);
    }

    const actualSize = () => zoomBy(1 / cams[index].s);

    new ResizeObserver(() => {
      stages.forEach((_, i) => {
        if (!cams[i] || !touched[i]) refit(i, false);
      });
    }).observe(canvas);

    // Pan with a drag; ctrl + wheel (and trackpad pinch) zooms at the pointer.
    let drag = null;
    canvas.addEventListener("pointerdown", (event) => {
      if (event.button !== 0 || event.target.closest(".pill")) return;
      drag = { id: event.pointerId, x: event.clientX, y: event.clientY, cam: { ...cams[index] }, moved: false };
    });
    canvas.addEventListener("pointermove", (event) => {
      if (!drag || drag.id !== event.pointerId) return;
      const dx = event.clientX - drag.x, dy = event.clientY - drag.y;
      if (!drag.moved && Math.hypot(dx, dy) < 4) return;
      if (!drag.moved) {
        drag.moved = true;
        canvas.setPointerCapture(event.pointerId);
        canvas.classList.add("dragging");
      }
      cams[index] = { x: drag.cam.x + dx, y: drag.cam.y + (event.pointerType === "touch" ? 0 : dy), s: drag.cam.s };
      touched[index] = true;
      apply(index, false);
    });
    const endDrag = () => {
      if (drag && drag.moved) canvas.classList.remove("dragging");
      setTimeout(() => (drag = null));
    };
    canvas.addEventListener("pointerup", endDrag);
    canvas.addEventListener("pointercancel", () => { canvas.classList.remove("dragging"); drag = null; });
    canvas.addEventListener("wheel", (event) => {
      if (!event.ctrlKey && !event.metaKey) return;
      event.preventDefault();
      const box = canvas.getBoundingClientRect();
      zoomBy(Math.exp(-Math.max(-60, Math.min(60, event.deltaY)) * 0.01), event.clientX - box.left, event.clientY - box.top, false);
    }, { passive: false });

    // Selection -------------------------------------------------------------

    function clear() {
      const svg = svgOf(index);
      svg.classList.remove("picking");
      $$(".on, .sel", svg).forEach((element) => element.classList.remove("on", "sel"));
      $$(".ring", svg).forEach((ring) => ring.remove());
      $$("li.hit", codes[index]).forEach((line) => line.classList.remove("hit"));
      picked = null;
    }

    function select(id, { scroll = true } = {}) {
      clear();
      const svg = svgOf(index);
      const node = $(`.node[data-id="${CSS.escape(id)}"]`, svg);
      if (!node) return;
      picked = id;
      svg.classList.add("picking");
      node.classList.add("on", "sel");
      const shape = node.firstElementChild;
      const ring = shape.cloneNode(false);
      for (const attribute of ["fill", "stroke", "stroke-width", "class", "stroke-opacity"]) ring.removeAttribute(attribute);
      ring.setAttribute("class", "ring");
      node.insertBefore(ring, shape);
      const other = $(`.node[data-id="${CSS.escape(twin(id))}"]`, svg);
      if (other) other.classList.add("on");

      const neighbours = new Set();
      for (const edge of $$(".edge, .edge-label", svg)) {
        const { from, to } = edge.dataset;
        if (from === id || to === id || (other && (from === other.dataset.id || to === other.dataset.id))) {
          edge.classList.add("on");
          for (const end of [from, to]) if (end !== id && end !== twin(id)) neighbours.add(end);
        }
      }
      for (const end of neighbours) {
        const neighbour = $(`.node[data-id="${CSS.escape(end)}"]`, svg);
        if (neighbour) neighbour.classList.add("on");
      }

      const lines = (node.dataset.lines || "").split(" ").filter(Boolean);
      const rows = lines.map((n) => $(`li[data-line="${n}"]`, codes[index])).filter(Boolean);
      rows.forEach((row) => row.classList.add("hit"));
      if (scroll && rows[0] && win.classList.contains("src-open")) {
        const list = codes[index];
        const target = rows[0].offsetTop - list.clientHeight / 3;
        list.scrollTo({ top: Math.max(0, target), behavior: reduced.matches ? "auto" : "smooth" });
      }

      const names = [...neighbours].map((end) => $(`.node[data-id="${CSS.escape(end)}"]`, svg)).filter(Boolean).map(label);
      const unique = [...new Set(names)];
      announce.textContent = `${label(node)} selected.` +
        (unique.length ? ` Connected to ${unique.join(", ")}.` : "") +
        (lines.length ? ` Source line${lines.length > 1 ? "s" : ""} ${lines.join(", ")}.` : "");
    }

    canvas.addEventListener("click", (event) => {
      if (drag && drag.moved) return;
      const node = event.target.closest(".node");
      if (node && !event.target.closest(".pill")) {
        const id = node.dataset.id.endsWith(" (end)") ? twin(node.dataset.id) : node.dataset.id;
        if (picked === id) { clear(); announce.textContent = "Selection cleared."; }
        else select(id);
      } else if (!event.target.closest(".pill")) {
        if (picked) { clear(); announce.textContent = "Selection cleared."; }
      }
    });

    // A source line finds the first node it names.
    win.addEventListener("click", (event) => {
      const row = event.target.closest(".code li");
      if (!row) return;
      const n = row.dataset.line;
      const text = row.textContent;
      const candidates = nodesOf(index).filter((node) => (node.dataset.lines || "").split(" ").includes(n));
      if (!candidates.length) { clear(); return; }
      const position = (node) => {
        const id = node.dataset.id.startsWith("[*]") ? "[*]" : node.dataset.id;
        const at = text.indexOf(id);
        return at < 0 ? Infinity : at;
      };
      candidates.sort((a, b) => position(a) - position(b));
      select(candidates[0].dataset.id, { scroll: false });
    });

    canvas.addEventListener("keydown", (event) => {
      const nodes = nodesOf(index);
      if (!nodes.length) return;
      const current = nodes.findIndex((node) => node.dataset.id === picked);
      if (["ArrowRight", "ArrowDown", "ArrowLeft", "ArrowUp"].includes(event.key)) {
        event.preventDefault();
        const step = event.key === "ArrowRight" || event.key === "ArrowDown" ? 1 : -1;
        const next = current < 0 ? (step > 0 ? 0 : nodes.length - 1) : (current + step + nodes.length) % nodes.length;
        select(nodes[next].dataset.id);
      } else if (event.key === "Escape" && picked) {
        event.preventDefault();
        event.stopPropagation();
        clear();
        announce.textContent = "Selection cleared.";
      } else if (event.key === "Enter" && picked && !win.classList.contains("src-open")) {
        event.preventDefault();
        setSource(true);
        select(picked);
      }
    });

    // Diagrams ----------------------------------------------------------------

    function go(next, direction) {
      next = (next + stages.length) % stages.length;
      if (next === index) return;
      clear();
      const from = stages[index], to = stages[next];
      codes[index].hidden = true;
      codes[next].hidden = false;
      index = next;
      to.hidden = false;
      if (!cams[next] || !touched[next]) refit(next, false); else apply(next, false);
      const shift = reduced.matches ? 0 : 28 * direction;
      from.animate([{ opacity: 1, transform: "none" }, { opacity: 0, transform: `translateX(${-shift}px)` }],
        { duration: reduced.matches ? 150 : 180, easing: OUT }).finished.then(() => { if (stages[index] !== from) from.hidden = true; });
      to.animate([{ opacity: 0, transform: `translateX(${shift}px)` }, { opacity: 1, transform: "none" }],
        { duration: reduced.matches ? 150 : 340, easing: reduced.matches ? "linear" : SOFT });
      const title = to.dataset.title;
      $("#win-title", win).textContent = title;
      $("#win-count", win).textContent = `${index + 1} of ${stages.length}`;
      exportLink.href = `diagrams/hero-${index}.svg`;
      exportLink.download = `${slug(title)}.svg`;
      zoomLabel.textContent = `${Math.round(cams[index].s * 100)}%`;
    }

    $("#prev", win).addEventListener("click", () => go(index - 1, -1));
    $("#next", win).addEventListener("click", () => go(index + 1, 1));
    $("#zoom-in", win).addEventListener("click", () => zoomBy(1.25));
    $("#zoom-out", win).addEventListener("click", () => zoomBy(0.8));
    $("#fit", win).addEventListener("click", () => refit(index, true));

    win.addEventListener("keydown", (event) => {
      if (typing(event.target) || event.ctrlKey || event.metaKey || event.altKey) return;
      const actions = {
        "[": () => go(index - 1, -1),
        "]": () => go(index + 1, 1),
        f: () => refit(index, true),
        F: () => refit(index, true),
        0: () => refit(index, true),
        1: actualSize,
        "=": () => zoomBy(1.25),
        "+": () => zoomBy(1.25),
        "-": () => zoomBy(0.8),
      };
      const action = actions[event.key];
      if (action) { event.preventDefault(); action(); }
    });

    // Source card ---------------------------------------------------------------

    function setSource(open) {
      win.classList.toggle("src-open", open);
      sourceButton.setAttribute("aria-pressed", String(open));
      sourceButton.setAttribute("aria-label", open ? "Hide source" : "Show source");
      requestAnimationFrame(() => stages.forEach((_, i) => { if (!touched[i]) refit(i, i === index); }));
    }
    sourceButton.addEventListener("click", () => setSource(!win.classList.contains("src-open")));
    $("#src-close", win).addEventListener("click", () => { setSource(false); sourceButton.focus(); });

    // Start with the source open where there is room for both, as a desk would.
    if (matchMedia("(min-width: 1000px)").matches) setSource(true);
    stages.forEach((_, i) => refit(i, false));
  }

  // ------------------------------------------------------------------ gallery viewer

  const viewer = $("#viewer");
  const tiles = $$(".tile");
  if (viewer && tiles.length) gallery(viewer, tiles);

  function gallery(viewer, tiles) {
    const image = $("#viewer-img", viewer);
    const source = $("#viewer-src", viewer);
    const copyLabel = $("#copy-label", viewer);
    let current = 0;
    let closing = false;

    function fill(i) {
      current = (i + tiles.length) % tiles.length;
      const tile = tiles[current];
      const thumb = $("img", tile);
      $("#viewer-title", viewer).textContent = $(".tile-name", tile).textContent;
      $("#viewer-key", viewer).textContent = $(".tile-key", tile).textContent;
      image.src = thumb.getAttribute("src");
      image.alt = thumb.alt;
      image.width = thumb.width || +thumb.getAttribute("width");
      image.height = +thumb.getAttribute("height");
      source.replaceChildren($("template", tile).content.cloneNode(true));
      $("#viewer-pos", viewer).textContent = `${current + 1} of ${tiles.length}`;
      copyLabel.textContent = "Copy";
      copyLabel.classList.remove("copy-done");
    }

    // The dialog grows out of the tile and shrinks back into it.
    function originFrom(element) {
      const box = viewer.getBoundingClientRect();
      const from = element.getBoundingClientRect();
      const visible = from.bottom > 0 && from.top < innerHeight;
      if (!visible || !box.width) { viewer.style.setProperty("--dx", "0px"); viewer.style.setProperty("--dy", "12px"); viewer.style.setProperty("--ds", "0.96"); return; }
      viewer.style.setProperty("--dx", `${from.left + from.width / 2 - (box.left + box.width / 2)}px`);
      viewer.style.setProperty("--dy", `${from.top + from.height / 2 - (box.top + box.height / 2)}px`);
      viewer.style.setProperty("--ds", String(Math.max(0.2, Math.min(0.9, from.width / box.width))));
    }

    function open(i) {
      fill(i);
      viewer.classList.remove("closing");
      viewer.showModal();
      originFrom($(".tile-hit", tiles[current]));
      viewer.classList.add("opening");
      viewer.addEventListener("animationend", () => viewer.classList.remove("opening"), { once: true });
    }

    function close() {
      if (closing || !viewer.open) return;
      closing = true;
      viewer.classList.remove("opening");
      originFrom($(".tile-hit", tiles[current]));
      viewer.classList.add("closing");
      const done = () => { viewer.classList.remove("closing"); viewer.close(); closing = false; };
      const style = getComputedStyle(viewer);
      if (style.animationName === "none" || parseFloat(style.animationDuration) === 0) done();
      else viewer.addEventListener("animationend", done, { once: true });
    }

    function step(direction) {
      fill(current + direction);
      if (!reduced.matches) image.animate([{ opacity: 0, transform: `translateX(${16 * direction}px)` }, { opacity: 1, transform: "none" }], { duration: 260, easing: SOFT });
      const hit = $(".tile-hit", tiles[current]);
      hit.scrollIntoView({ block: "nearest", behavior: "auto" });
    }

    tiles.forEach((tile, i) => $(".tile-hit", tile).addEventListener("click", () => open(i)));
    $("#viewer-close", viewer).addEventListener("click", close);
    $("#viewer-prev", viewer).addEventListener("click", () => step(-1));
    $("#viewer-next", viewer).addEventListener("click", () => step(1));
    viewer.addEventListener("cancel", (event) => { event.preventDefault(); close(); });
    viewer.addEventListener("click", (event) => { if (event.target === viewer) close(); });
    viewer.addEventListener("close", () => {
      const hit = $(".tile-hit", tiles[current]);
      if (document.activeElement === document.body || !document.activeElement) hit.focus({ preventScroll: true });
    });
    viewer.addEventListener("keydown", (event) => {
      if (event.ctrlKey || event.metaKey || event.altKey) return;
      if (event.key === "[") { event.preventDefault(); step(-1); }
      if (event.key === "]") { event.preventDefault(); step(1); }
    });

    $("#copy", viewer).addEventListener("click", async () => {
      const text = source.textContent;
      try {
        await navigator.clipboard.writeText(text);
        copyLabel.textContent = "Copied";
        copyLabel.classList.add("copy-done");
      } catch {
        const range = document.createRange();
        range.selectNodeContents($("pre", source));
        getSelection().removeAllRanges();
        getSelection().addRange(range);
        copyLabel.textContent = mac ? "Press ⌘C" : "Press Ctrl+C";
      }
      setTimeout(() => { copyLabel.textContent = "Copy"; copyLabel.classList.remove("copy-done"); }, 1800);
    });
  }

  // ------------------------------------------------------------------ guide contents

  const toc = $(".toc");
  if (toc) contents(toc);

  function contents(toc) {
    const details = $("details", toc);
    const wide = matchMedia("(min-width: 1040px)");
    const sync = () => { details.open = wide.matches; };
    sync();
    wide.addEventListener("change", sync);

    const links = new Map($$("a[href^='#']", toc).map((link) => [link.getAttribute("href").slice(1), link]));
    toc.addEventListener("click", (event) => { if (event.target.closest("a") && !wide.matches) details.open = false; });
    const visible = new Set();
    const observer = new IntersectionObserver((entries) => {
      for (const entry of entries) entry.isIntersecting ? visible.add(entry.target.id) : visible.delete(entry.target.id);
      const first = [...links.keys()].find((id) => visible.has(id));
      if (!first) return;
      for (const [id, link] of links) {
        if (id === first) link.setAttribute("aria-current", "true");
        else link.removeAttribute("aria-current");
      }
    }, { rootMargin: "-72px 0px -55% 0px" });
    for (const id of links.keys()) {
      const section = document.getElementById(id);
      if (section) observer.observe(section);
    }
  }
})();
