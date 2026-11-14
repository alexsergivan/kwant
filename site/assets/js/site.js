// kwant site: small progressive enhancements. Everything works without this file.
(function () {
  "use strict";
  var reduce = window.matchMedia("(prefers-reduced-motion: reduce)");

  // Demo video posters, and the home page's lazy images (data-src), load only when they come within 600 px of the
  // screen, so they never compete with the page's first image. Without JS the videos show no poster, just the play
  // control, and the images come from their <noscript> copy.
  var lazy = document.querySelectorAll("video[data-poster], img[data-src]");
  function show(el) {
    if (el.tagName === "VIDEO") { if (!el.getAttribute("poster")) el.poster = el.dataset.poster; return; }
    if (el.dataset.srcset) el.srcset = el.dataset.srcset;
    el.src = el.dataset.src;
    el.removeAttribute("data-src");
    el.removeAttribute("data-srcset");
  }
  if ("IntersectionObserver" in window) {
    var near = new IntersectionObserver(function (entries) {
      entries.forEach(function (e) { if (e.isIntersecting) { show(e.target); near.unobserve(e.target); } });
    }, { rootMargin: "600px 0px" });
    lazy.forEach(function (el) { near.observe(el); });
  } else {
    lazy.forEach(show);
  }

  // Cloudflare Web Analytics (see CF_ANALYTICS in build_site.py): added after the page has loaded and painted.
  var beacon = document.getElementById("cf-beacon");
  function analytics() {
    var s = document.createElement("script");
    s.src = "https://static.cloudflareinsights.com/beacon.min.js";
    s.defer = true;
    s.setAttribute("data-cf-beacon", beacon.textContent);
    document.body.appendChild(s);
  }
  function later() { requestAnimationFrame(function () { setTimeout(analytics, 0); }); }
  if (beacon) { if (document.readyState === "complete") later(); else window.addEventListener("load", later, { once: true }); }

  // Demo videos play while on screen, unless the visitor prefers reduced motion.
  var videos = document.querySelectorAll("video[data-autoplay]");
  if (videos.length && "IntersectionObserver" in window) {
    var io = new IntersectionObserver(function (entries) {
      entries.forEach(function (e) {
        var v = e.target;
        if (e.isIntersecting && !reduce.matches) {
          if (v.preload === "none") v.preload = "auto";
          var p = v.play();
          if (p && p.catch) p.catch(function () { v.controls = true; });
        } else {
          v.pause();
        }
      });
    }, { threshold: 0.2 });
    videos.forEach(function (v) {
      v.controls = reduce.matches;
      io.observe(v);
    });
  }

  // On small screens the docs nav is a sideways strip: keep the current page's tab in view.
  var current = document.querySelector(".docs-nav a[aria-current]");
  if (current) {
    var strip = current.closest(".docs-nav");
    if (strip.scrollWidth > strip.clientWidth) strip.scrollLeft = current.offsetLeft - strip.offsetLeft - 18;
  }

  // Features "On this page" bar: mark the group being read and keep its chip in view.
  var jumpBar = document.querySelector(".jump-bar");
  if (jumpBar) {
    var row = jumpBar.querySelector(".jump-scroll");
    var chips = [].slice.call(jumpBar.querySelectorAll(".jump a"));
    var groups = chips.map(function (a) { return document.getElementById(a.getAttribute("href").slice(1)); });
    var shown = null;
    var edge = function () {
      jumpBar.classList.toggle("more", row.scrollLeft + row.clientWidth < row.scrollWidth - 2);
    };
    // Scroll only the row sideways; scrollIntoView would also move the page.
    var reveal = function (a) {
      var pad = parseFloat(getComputedStyle(row).paddingLeft) || 0;
      var left = a.offsetLeft - row.offsetLeft;
      if (left < row.scrollLeft + pad || left + a.offsetWidth > row.scrollLeft + row.clientWidth - pad) {
        row.scrollTo({ left: left - pad - 40, behavior: reduce.matches ? "auto" : "smooth" });
      }
    };
    var mark = function (a) {
      if (a === shown) return;
      shown = a;
      chips.forEach(function (o) {
        if (o === a) o.setAttribute("aria-current", "true"); else o.removeAttribute("aria-current");
      });
      if (a) reveal(a);
    };
    var spy = function () {
      var line = jumpBar.getBoundingClientRect().bottom + window.innerHeight * 0.25;
      var hit = null;
      groups.forEach(function (g, i) { if (g && g.getBoundingClientRect().top <= line) hit = chips[i]; });
      var last = groups[groups.length - 1];
      if (last && last.getBoundingClientRect().bottom < jumpBar.getBoundingClientRect().bottom) hit = null;
      mark(hit);
    };
    var queued = false;
    window.addEventListener("scroll", function () {
      if (queued) return;
      queued = true;
      requestAnimationFrame(function () { queued = false; spy(); });
    }, { passive: true });
    window.addEventListener("resize", function () { edge(); spy(); });
    row.addEventListener("scroll", edge, { passive: true });
    row.addEventListener("focusin", function (e) { if (e.target.tagName === "A") reveal(e.target); });
    edge();
    spy();
  }

  // Scrollable tables can be reached and scrolled with the keyboard.
  document.querySelectorAll(".table-wrap").forEach(function (w) {
    if (w.scrollWidth > w.clientWidth) {
      w.tabIndex = 0;
      w.setAttribute("role", "region");
      w.setAttribute("aria-label", "Table, scrolls sideways");
    }
  });

  // Install box: tabs (arrow keys move between them) and a copy button per command.
  document.querySelectorAll("[data-tabs]").forEach(function (box) {
    var tabs = [].slice.call(box.querySelectorAll("[role=tab]"));
    function pick(t, focus) {
      tabs.forEach(function (o) {
        var on = o === t;
        o.setAttribute("aria-selected", on ? "true" : "false");
        o.tabIndex = on ? 0 : -1;
        document.getElementById(o.getAttribute("aria-controls")).hidden = !on;
      });
      if (focus) t.focus();
    }
    tabs.forEach(function (t, i) {
      t.addEventListener("click", function () { pick(t); });
      t.addEventListener("keydown", function (e) {
        var j = e.key === "ArrowRight" ? i + 1 : e.key === "ArrowLeft" ? i - 1 : e.key === "Home" ? 0 : e.key === "End" ? tabs.length - 1 : null;
        if (j === null) return;
        e.preventDefault();
        pick(tabs[(j + tabs.length) % tabs.length], true);
      });
    });
  });
  document.querySelectorAll(".copy-cmd").forEach(function (btn) {
    if (!navigator.clipboard) { btn.hidden = true; return; }
    btn.addEventListener("click", function () {
      navigator.clipboard.writeText(btn.parentNode.querySelector("code").textContent).then(function () {
        btn.textContent = "copied";
        setTimeout(function () { btn.textContent = "copy"; }, 1600);
      });
    });
  });

  // Copy buttons on code blocks.
  document.querySelectorAll("pre > code").forEach(function (code) {
    if (!navigator.clipboard) return;
    var btn = document.createElement("button");
    btn.type = "button";
    btn.className = "copy";
    btn.textContent = "Copy";
    btn.addEventListener("click", function () {
      var text = code.innerText.replace(/\n$/, "");
      navigator.clipboard.writeText(text).then(function () {
        btn.textContent = "Copied";
        setTimeout(function () { btn.textContent = "Copy"; }, 1600);
      });
    });
    code.parentNode.appendChild(btn);
  });

  // Screenshot lightbox. Links still open the full-size image without JS.
  var dlg = document.createElement("dialog");
  if (typeof dlg.showModal !== "function") return;
  dlg.className = "lightbox";
  dlg.innerHTML = '<img alt=""><div class="lb-bar"><span></span><button type="button">Close</button></div>';
  document.body.appendChild(dlg);
  var img = dlg.querySelector("img");
  var cap = dlg.querySelector("span");
  dlg.querySelector("button").addEventListener("click", function () { dlg.close(); });
  dlg.addEventListener("click", function (e) { if (e.target === dlg) dlg.close(); });
  dlg.addEventListener("close", function () { img.removeAttribute("src"); });
  document.querySelectorAll("a.zoom").forEach(function (a) {
    a.addEventListener("click", function (e) {
      if (e.metaKey || e.ctrlKey || e.shiftKey) return;
      e.preventDefault();
      var inner = a.querySelector("img");
      img.src = a.getAttribute("href");
      img.alt = inner ? inner.alt : "";
      cap.textContent = a.getAttribute("data-caption") || img.alt;
      dlg.showModal();
    });
  });
})();

// Holo stickers: the sheen follows a mouse or pen (ported from the app's Sticker.tsx). It tracks across the
// sticker's card ([data-holo-area]), or the sticker itself when it has none. Touch and reduced motion keep 30% / 20%.
(() => {
  const stickers = document.querySelectorAll(".kw-sticker-holo");
  if (!stickers.length) return;
  const still = window.matchMedia("(prefers-reduced-motion: reduce)");
  const clamp = (v) => Math.max(0, Math.min(100, Math.round(v)));
  stickers.forEach((el) => {
    const area = el.closest("[data-holo-area]") || el;
    area.addEventListener("pointermove", (e) => {
      if (e.pointerType === "touch" || still.matches) return;
      const r = area.getBoundingClientRect();
      if (!r.width || !r.height) return;
      el.style.setProperty("--mx", clamp(((e.clientX - r.left) / r.width) * 100) + "%");
      el.style.setProperty("--my", clamp(((e.clientY - r.top) / r.height) * 100) + "%");
    });
    area.addEventListener("pointerleave", () => {
      el.style.removeProperty("--mx");
      el.style.removeProperty("--my");
    });
  });
})();

// Downloads: lead with the visitor's own platform. The HTML shows Mac (also on phones and without this file);
// Windows and Linux visitors get their button and first-run hint. Picking another platform's link switches the
// box to it, so its first-run help is the one on screen.
(function () {
  var boxes = document.querySelectorAll("[data-dl]");
  if (!boxes.length) return;
  var ua = navigator.userAgent || "";
  var plat = (navigator.userAgentData && navigator.userAgentData.platform) || navigator.platform || "";
  var os = null;
  if (!/Android|iPhone|iPad|iPod|CrOS/i.test(ua)) {
    if (/Win/i.test(plat) || /Windows NT/i.test(ua)) os = "windows";
    else if (/Linux/i.test(plat) || /X11|Linux/i.test(ua)) os = "linux";
  }
  boxes.forEach(function (box) {
    if (os) box.setAttribute("data-os", os);
    box.querySelectorAll("a[data-os-link]").forEach(function (a) {
      a.addEventListener("click", function () { box.setAttribute("data-os", a.getAttribute("data-os-link")); });
    });
  });
})();

// O7a: opening the dmg download also opens the Mac first-open steps, so they're there when macOS asks.
(function () {
  var d = document.getElementById("mac-first");
  if (!d) return;
  document.querySelectorAll('a[href$="kwant_universal.dmg"]').forEach(function (a) {
    a.addEventListener("click", function () { d.open = true; });
  });
})();
