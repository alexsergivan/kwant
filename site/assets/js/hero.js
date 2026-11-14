// kwant.si hero: the dithered pixel nebula and the "kwant moments" toolbar.
// Mira's design (Nov 13 2026), built by Pixel. Runs after first paint, needs no library. Home page only.
// Nebula: after load; 8 fps only on a desktop with a mouse, one still frame on phones and touch screens and under reduced motion;
// paused off screen or in a hidden tab; each frame drawn in slices of at most 8 ms.
// Moments: an ARIA tablist; it never switches by itself.
(function () {
  "use strict";
  function afterPaint(fn) {
    function go() { requestAnimationFrame(function () { setTimeout(fn, 0); }); }
    if (document.readyState === "complete") go(); else window.addEventListener("load", go, { once: true });
  }
  var reduce = window.matchMedia("(prefers-reduced-motion: reduce)");

  /* ---------- Nebula ----------
     One canvas cell = 4 CSS px (PX). A warped value-noise field picks a level 0..4 on one of three ladders
     (violet, teal, pink), and a 4x4 Bayer matrix dithers both the level and the ladder choice.
     Teal = --v4 mixed 45% with --ok; pink = --v5 mixed 50% with the avatar coral #ff8a7a, then 80% toward --scan. Nothing else. */
  var PX = 4;
  var LADDERS = {
    violet: ["#0d0a1f", "#160c3a", "#22125c", "#2f1a80", "#4024a8"],
    teal:   ["#0d0a1f", "#19243b", "#263e56", "#325772", "#3e718e"],
    pink:   ["#0d0a1f", "#2d1c3a", "#4e2e54", "#6e406f", "#8e528a"]
  };
  var BAYER = [0, 8, 2, 10, 12, 4, 14, 6, 3, 11, 1, 9, 15, 7, 13, 5];
  function rgb(h) { return [parseInt(h.slice(1, 3), 16), parseInt(h.slice(3, 5), 16), parseInt(h.slice(5, 7), 16)]; }
  var LAD = {}; Object.keys(LADDERS).forEach(function (k) { LAD[k] = LADDERS[k].map(rgb); });

  function hash(x, y) { var h = x * 374761393 + y * 668265263; h = (h ^ (h >>> 13)) * 1274126177; return ((h ^ (h >>> 16)) >>> 0) / 4294967295; }
  function smooth(t) { return t * t * (3 - 2 * t); }
  function noise(x, y) {
    var xi = Math.floor(x), yi = Math.floor(y), xf = smooth(x - xi), yf = smooth(y - yi);
    var a = hash(xi, yi), b = hash(xi + 1, yi), c = hash(xi, yi + 1), d = hash(xi + 1, yi + 1);
    return a + (b - a) * xf + (c - a) * yf + (a - b - c + d) * xf * yf;
  }
  function fbm(x, y) { var s = 0, amp = 0.5; for (var i = 0; i < 4; i++) { s += amp * noise(x, y); x *= 2.03; y *= 2.03; amp *= 0.5; } return s / 0.9375; }
  function clamp(v) { return v < 0 ? 0 : v > 1 ? 1 : v; }
  function sstep(a, b, v) { return smooth(clamp((v - a) / (b - a))); }

  // One frame, drawn a few rows at a time: each call works for at most `budget` ms and returns true once the frame
  // is on the canvas. So a frame never blocks the page for long, even on a slow phone.
  function frame(cv, t) {
    var w = cv.width, h = cv.height, ctx = cv.getContext("2d"), img = ctx.createImageData(w, h), p = img.data, y = 0;
    var cx = parseFloat(cv.dataset.cx || 0.5), cy = parseFloat(cv.dataset.cy || 0.3);
    var rx = parseFloat(cv.dataset.rx || 0.34), ry = parseFloat(cv.dataset.ry || 0.30);
    var aspect = (w / h);
    function row(y) {
      for (var x = 0; x < w; x++) {
        var u = x / w, v = y / h;
        var sx = u * 3.2 * aspect, sy = v * 3.2;
        // domain warp: smoke, not blobs
        var wx = fbm(sx + t * 0.05, sy - t * 0.03), wy = fbm(sx - 4.1, sy + 2.7 + t * 0.04);
        var n = fbm(sx + 1.8 * wx, sy + 1.8 * wy);
        // quiet middle behind the headline, smoke at the edges and top
        var dx = (u - cx) / rx, dy = (v - cy) / ry, r = Math.sqrt(dx * dx + dy * dy);
        var edge = sstep(0.55, 1.35, r);
        var floor = 1 - sstep(0.55, 0.95, v); // fade out toward the window
        var d = clamp(sstep(0.30, 0.82, n) * edge * (0.35 + 0.65 * floor) * 1.15);
        var th = (BAYER[(y & 3) * 4 + (x & 3)] + 0.5) / 16;
        // hue: teal on the left, pink on the right, violet over the top; dithered between ladders
        var side = (u - 0.5) * 2.4 + (wx - 0.5) * 0.9;
        var lad = LAD.violet;
        if (Math.abs(side) > 0.35 + th * 0.9) lad = side < 0 ? LAD.teal : LAD.pink;
        var lv = Math.floor(d * 4 + th);
        if (lv > 4) lv = 4;
        var c = lad[lv], i = (y * w + x) * 4;
        p[i] = c[0]; p[i + 1] = c[1]; p[i + 2] = c[2]; p[i + 3] = 255;
      }
    }
    return function (budget) {
      var stop = performance.now() + budget;
      while (y < h) { row(y++); if (performance.now() > stop) return false; }
      ctx.putImageData(img, 0, 0);
      return true;
    };
  }

  // Phones and touch screens get one still frame (drawn after the page has loaded, in small slices); only a desktop with
  // a mouse animates it, at 8 fps, paused off screen and in a hidden tab. Reduced motion: one still frame everywhere.
  function startNebula(cv) {
    var box = cv.parentNode, animate = window.matchMedia("(hover: hover) and (min-width: 860px)");
    var t = parseFloat(cv.dataset.t || 7), gen = 0, timer = 0, onScreen = true;
    function size() { cv.width = Math.ceil(box.clientWidth / PX); cv.height = Math.ceil(box.clientHeight / PX); }
    function moving() { return animate.matches && !reduce.matches && cv.dataset.still === undefined && onScreen && !document.hidden; }
    // Draws one frame in slices of at most 8 ms, then calls done (unless a newer paint has started).
    function paint(done) {
      var my = ++gen, f = frame(cv, t);
      (function slice() { if (my !== gen) return; if (f(8)) { timer = 0; cv.classList.add("live"); if (done) done(); } else timer = setTimeout(slice, 0); })();
    }
    function tick() {
      timer = 0;
      if (!moving()) return;
      t += 0.125 * 0.35;
      paint(function () { timer = setTimeout(tick, 125); });
    }
    function run() { if (moving() && !timer) timer = setTimeout(tick, 125); }
    size();
    paint(run);
    if ("IntersectionObserver" in window) new IntersectionObserver(function (e) { onScreen = e[0].isIntersecting; run(); }).observe(box);
    document.addEventListener("visibilitychange", run);
    if (animate.addEventListener) animate.addEventListener("change", run);
    var rz = 0;
    window.addEventListener("resize", function () { clearTimeout(rz); rz = setTimeout(function () { clearTimeout(timer); timer = 0; size(); paint(run); }, 150); });
  }

  /* ---------- Moments toolbar (ARIA tablist) ---------- */
  function startMoments(root) {
    var tabs = [].slice.call(root.querySelectorAll('[role="tab"]'));
    var cap = root.querySelector("[data-caption]");
    var video = root.querySelector("video");
    var poster = root.querySelector(".win-poster");
    var panel = root.querySelector('[role="tabpanel"]');
    var pause = root.querySelector("[data-pause]");
    function paused() { return pause.getAttribute("aria-pressed") === "true"; }
    // The poster image stays under the video; the video shows once it has a frame to show.
    video.addEventListener("playing", function () { video.classList.add("on"); });
    video.addEventListener("loadeddata", function () { video.classList.add("on"); });
    var chars = [].slice.call(root.querySelectorAll("[data-char]"));
    function select(tab, focus) {
      tabs.forEach(function (t) {
        var on = t === tab; t.setAttribute("aria-selected", on); t.tabIndex = on ? 0 : -1;
      });
      cap.textContent = tab.dataset.text;
      poster.srcset = tab.dataset.srcset; poster.src = tab.dataset.poster; poster.alt = tab.dataset.text;
      video.classList.remove("on");
      if (video.dataset.ready) { video.src = tab.dataset.clip; if (!paused()) video.play().catch(function () {}); }
      panel.setAttribute("aria-labelledby", tab.id);
      chars.forEach(function (c) { c.classList.toggle("on", c.dataset.char === tab.dataset.bot); });
      if (focus) tab.focus();
    }
    tabs.forEach(function (t, i) {
      t.addEventListener("click", function () { select(t); });
      t.addEventListener("keydown", function (e) {
        var j = { ArrowRight: i + 1, ArrowLeft: i - 1, Home: 0, End: tabs.length - 1 }[e.key];
        if (j === undefined) return;
        e.preventDefault(); select(tabs[(j + tabs.length) % tabs.length], true);
      });
    });
    pause.addEventListener("click", function () {
      if (paused()) {
        pause.setAttribute("aria-pressed", "false"); pause.setAttribute("aria-label", "Pause clip");
        if (!video.dataset.ready) arm(true); else video.play().catch(function () {});
      } else {
        pause.setAttribute("aria-pressed", "true"); pause.setAttribute("aria-label", "Play clip");
        video.pause();
      }
    });
    if (reduce.matches) { pause.setAttribute("aria-pressed", "true"); pause.setAttribute("aria-label", "Play clip"); }
    // Load the clip only after first paint and only when the window is on screen.
    // Under reduced motion nothing loads until the visitor presses play.
    function arm(force) {
      if (video.dataset.ready || (paused() && !force)) return;
      video.dataset.ready = "1";
      var cur = tabs.filter(function (t) { return t.getAttribute("aria-selected") === "true"; })[0];
      video.src = cur.dataset.clip;
      if (!paused()) video.play().catch(function () {});
    }
    // Wait until the page has loaded and painted so the clip never competes with the poster (the LCP image).
    function watch() {
      if ("IntersectionObserver" in window) new IntersectionObserver(function (e) { if (e[0].isIntersecting) arm(); }, { rootMargin: "200px" }).observe(video);
      else arm();
    }
    afterPaint(watch);
    var start = new URLSearchParams(location.search).get("tab");
    if (start && tabs[+start]) select(tabs[+start]);
  }

  // The "Watch it work" reel (it has a voiceover; the hero clips stay muted). Without JS (or <dialog>) the pill is a plain link to the mp4. The video's src and
  // poster are set only when the dialog opens, so the page loads none of it. Esc, Close and a click on the backdrop
  // close it; closing pauses and rewinds; the native dialog returns focus to the pill.
  function startReel(pill) {
    var dlg = document.getElementById("reel-dialog");
    if (!dlg || typeof dlg.showModal !== "function") return;
    var video = dlg.querySelector("video"), play = dlg.querySelector("[data-reel-play]");
    function arm() {
      if (video.dataset.ready) return;
      video.dataset.ready = "1";
      var big = (window.innerWidth * (window.devicePixelRatio || 1)) > 1280;
      var poster = (big && video.dataset.posterLg) || video.dataset.posterSm || video.dataset.posterLg;
      if (poster) video.poster = poster;
      var track = video.querySelector("track[data-src]");
      if (track) track.src = track.dataset.src;
      video.src = video.dataset.src;
    }
    pill.addEventListener("click", function (e) {
      if (e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) return;
      e.preventDefault();
      arm();
      document.documentElement.classList.add("reel-open");
      dlg.showModal();
      if (reduce.matches) { play.hidden = false; play.focus(); return; }
      // Unmuted, inside the click: the click is the gesture that allows sound. If the browser still refuses, the
      // native controls and the poster stay up; it is never retried muted.
      play.hidden = true;
      video.muted = false;
      video.focus();
      video.play().catch(function () {});
    });
    play.addEventListener("click", function () { play.hidden = true; video.focus(); video.play().catch(function () {}); });
    video.addEventListener("play", function () { play.hidden = true; });
    dlg.querySelector("[data-reel-close]").addEventListener("click", function () { dlg.close(); });
    // A click on the dialog element itself (not its contents) is a click on the backdrop.
    dlg.addEventListener("click", function (e) { if (e.target === dlg) dlg.close(); });
    dlg.addEventListener("close", function () {
      video.pause();
      try { video.currentTime = 0; } catch (err) {}
      document.documentElement.classList.remove("reel-open");
      pill.focus();
    });
  }
  var reelPill = document.querySelector("[data-reel]"); if (reelPill) startReel(reelPill);

  // Extra work waits for the load event and then for the frame after it: on a fast load the load event comes before
  // the first paint, and anything started then would hold that paint up.
  function idle(fn, ms) { if ("requestIdleCallback" in window) requestIdleCallback(fn, { timeout: ms }); else setTimeout(fn, 50); }
  afterPaint(function () { idle(function () { var m = document.querySelector("[data-moments]"); if (m) startMoments(m); }, 600); });
  // Until the nebula starts, the still image (nebula-still.png) is the hero's background.
  afterPaint(function () { idle(function () { var cv = document.querySelector("canvas[data-nebula]"); if (cv) startNebula(cv); }, 1000); });
})();
