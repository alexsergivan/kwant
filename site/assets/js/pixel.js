// kwant site: the pixel scenes. The bots are the same 8-bit creatures as the kwant app's avatars,
// drawn in one violet ramp on a small CRT screen. Every scene has a still image behind it, so the screen is never empty.
(function () {
  "use strict";

  var V = ["#0b0620", "#160c3a", "#22125c", "#2f1a80", "#4024a8", "#5c3fd0", "#8a72ec", "#b8a9fb", "#ddd5ff", "#f4f1ff"];
  var AMBER = "#ffcc33";
  var reduce = window.matchMedia("(prefers-reduced-motion: reduce)");

  // Bots on dark backgrounds are light; on the light cards they're mid violet.
  var THEMES = {
    dark: { body: V[7], shade: V[6], light: V[9], accent: V[9], ink: V[0], blush: V[6], white: V[9], second: V[6], shadow: V[1] },
    light: { body: V[6], shade: V[5], light: V[8], accent: V[3], ink: V[0], blush: V[7], white: V[9], second: V[4], shadow: V[7] }
  };

  // ---- Creatures, ported from frontend/src/components/ModernAvatar.tsx (26-grid, cropped to x 5..21, y 2..21) ----
  function accessory(kind) {
    switch (kind) {
      case "antenna": return [[15, 6, 1, 1, "ink"], [15, 3, 2, 2, "accent"], [15, 3, 1, 1, "white"]];
      case "crown": return [[10, 5, 6, 2, "accent"], [10, 4, 1, 1, "accent"], [12, 3, 2, 2, "accent"], [15, 4, 1, 1, "accent"], [12, 5, 2, 1, "blush"]];
      case "hardhat": return [[10, 3, 6, 1, "accent"], [9, 4, 8, 2, "accent"], [7, 6, 12, 1, "second"], [13, 3, 1, 3, "second"]];
      case "beret": return [[9, 5, 8, 2, "accent"], [8, 6, 1, 1, "accent"], [13, 4, 1, 1, "accent"]];
      case "visor": return [[9, 5, 8, 1, "accent"], [8, 6, 1, 3, "accent"], [17, 6, 1, 3, "accent"], [6, 10, 2, 3, "accent"], [18, 10, 2, 3, "accent"]];
      case "ears": return [[8, 5, 2, 3, "body"], [16, 5, 2, 3, "shade"], [8, 6, 1, 1, "blush"], [17, 6, 1, 1, "blush"]];
      case "spikes": return [[10, 5, 1, 2, "body"], [13, 4, 1, 3, "body"], [16, 5, 1, 2, "body"], [13, 4, 1, 1, "light"]];
      case "bulb": return [[12, 2, 2, 1, "accent"], [11, 3, 4, 2, "accent"], [12, 5, 2, 1, "second"], [12, 6, 2, 1, "ink"]];
      default: return [];
    }
  }
  function overlay(kind) {
    switch (kind) {
      case "goggles": return [[7, 8, 12, 1, "ink"], [9, 7, 3, 2, "accent"], [14, 7, 3, 2, "accent"]];
      case "glasses": return [[9, 9, 4, 1, "accent"], [13, 9, 4, 1, "accent"], [9, 10, 1, 2, "accent"], [16, 10, 1, 2, "accent"], [12, 10, 2, 1, "accent"]];
      default: return [];
    }
  }
  function creature(kind, blink) {
    var eyes = blink
      ? [[10, 12, 2, 1, "ink"], [14, 12, 2, 1, "ink"]]
      : [[10, 10, 2, 3, "ink"], [14, 10, 2, 3, "ink"], [10, 10, 1, 1, "white"], [14, 10, 1, 1, "white"]];
    return [[7, 20, 12, 1, "shadow"]].concat(accessory(kind), [
      [9, 7, 8, 1, "body"], [8, 8, 10, 1, "body"], [7, 9, 12, 7, "body"], [8, 16, 10, 1, "body"], [9, 17, 8, 1, "shade"],
      [18, 9, 1, 7, "shade"], [17, 16, 1, 1, "shade"], [7, 15, 1, 1, "shade"], [8, 16, 1, 1, "shade"],
      [10, 8, 2, 1, "light"], [9, 9, 1, 1, "light"]
    ], eyes, [
      [8, 13, 2, 1, "blush"], [16, 13, 2, 1, "blush"], [11, 14, 1, 1, "ink"], [14, 14, 1, 1, "ink"], [12, 15, 2, 1, "ink"],
      [9, 18, 2, 1, "ink"], [15, 18, 2, 1, "ink"]
    ], overlay(kind));
  }
  var SPRITE_W = 16, SPRITE_H = 19;
  var spriteCache = {};
  function sprite(kind, theme, blink) {
    var key = kind + theme + (blink ? "b" : "");
    if (spriteCache[key]) return spriteCache[key];
    var c = document.createElement("canvas");
    c.width = SPRITE_W; c.height = SPRITE_H;
    var g = c.getContext("2d"), pal = THEMES[theme];
    creature(kind, blink).forEach(function (r) {
      g.fillStyle = pal[r[4]];
      g.fillRect(r[0] - 5, r[1] - 2, r[2], r[3]);
    });
    return (spriteCache[key] = c);
  }

  // ---- Small helpers ----
  function rng(seed) {
    return function () {
      seed |= 0; seed = (seed + 0x6d2b79f5) | 0;
      var t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
      t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
      return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
    };
  }
  var BAYER = [0, 8, 2, 10, 12, 4, 14, 6, 3, 11, 1, 9, 15, 7, 13, 5];
  // Ordered dithering between ramp steps: v is a fractional ramp index.
  function ditherColor(v, x, y) {
    var i = Math.floor(v), f = v - i;
    return V[Math.max(0, Math.min(9, f * 16 > BAYER[(y & 3) * 4 + (x & 3)] ? i + 1 : i))];
  }
  function rect(g, x, y, w, h, c) { g.fillStyle = c; g.fillRect(Math.round(x), Math.round(y), w, h); }
  function arcPoint(a, b, h, t) {
    return { x: a.x + (b.x - a.x) * t, y: a.y + (b.y - a.y) * t - 4 * h * t * (1 - t) };
  }
  function dottedArc(g, a, b, h, phase, color, upto) {
    var len = Math.abs(b.x - a.x) + h * 2, n = Math.max(6, Math.floor(len / 3));
    for (var i = 0; i <= n; i++) {
      var t = (i + phase) / n;
      if (t > (upto == null ? 1 : upto) || t > 1) break;
      var p = arcPoint(a, b, h, t);
      rect(g, p.x, p.y, 1, 1, color);
    }
  }
  function packet(g, x, y, theme) {
    var o = theme === "dark" ? V[1] : V[2], f = theme === "dark" ? V[9] : V[4];
    rect(g, x - 2, y - 2, 5, 5, o);
    rect(g, x - 1, y - 1, 3, 3, f);
  }
  function flag(g, x, y, t) {
    rect(g, x, y, 1, 9, V[9]);
    var wave = Math.floor(t * 4) % 2;
    rect(g, x + 1, y, 5, 3, AMBER);
    rect(g, x + 1 + 4, y + wave, 1, 3 - wave, AMBER);
    rect(g, x + 2, y + 3, 3, wave, AMBER);
  }
  function laptop(g, x, y, theme, glow) {
    rect(g, x, y, 9, 6, theme === "dark" ? V[1] : V[3]);
    rect(g, x + 1, y + 1, 7, 4, glow ? V[8] : V[6]);
    rect(g, x - 1, y + 6, 11, 1, theme === "dark" ? V[2] : V[4]);
  }

  // ---- A scene: a low-res canvas scaled up with crisp pixels ----
  function Scene(canvas, opts) {
    this.c = canvas;
    this.g = canvas.getContext("2d");
    this.o = opts;
    this.theme = opts.theme || "dark";
    this.bots = [];
    this.fx = [];
    this.t = 0;
    this.running = false;
    this.visible = true;
    var self = this;
    this.resize();
    if (window.ResizeObserver) new ResizeObserver(function () { self.resize(); self.draw(); }).observe(canvas.parentNode);
    canvas.parentNode.addEventListener("click", function (e) {
      var r = canvas.getBoundingClientRect();
      self.click((e.clientX - r.left) / r.width * self.W, (e.clientY - r.top) / r.height * self.H, e.detail === 0);
    });
    if ("IntersectionObserver" in window) {
      new IntersectionObserver(function (es) {
        self.visible = es[0].isIntersecting;
        if (self.visible) self.start();
      }).observe(canvas);
    }
    document.addEventListener("visibilitychange", function () { if (!document.hidden) self.start(); });
    reduce.addEventListener && reduce.addEventListener("change", function () { self.start(); self.draw(); });
  }
  Scene.prototype.resize = function () {
    var box = this.c.parentNode.getBoundingClientRect();
    var s = this.o.scale ? this.o.scale(box.width) : 3;
    var W = Math.max(40, Math.ceil(box.width / s)), H = Math.max(30, Math.ceil(box.height / s));
    if (W === this.W && H === this.H) return;
    this.W = this.c.width = W;
    this.H = this.c.height = H;
    this.g.imageSmoothingEnabled = false;
    this.bg = null;
    this.o.layout(this);
  };
  Scene.prototype.addBot = function (kind, x, y, state) {
    var b = { kind: kind, x: x, y: y, home: x, state: state || "idle", jump: 0, blinkAt: Math.random() * 4, seed: Math.random() * 10 };
    this.bots.push(b);
    return b;
  };
  Scene.prototype.handoff = function (from, to, h, done) {
    var self = this;
    this.fx.push({ kind: "hand", a: { x: from.x + 8, y: from.y + 4 }, b: { x: to.x + 8, y: to.y + 4 }, h: h || 14, t: 0, dur: 1.4,
      end: function () { to.state = "work"; to.jump = 0.35; if (done) done(to); self.changed(); } });
  };
  Scene.prototype.ripple = function (x, y) { this.fx.push({ kind: "ripple", x: x, y: y, t: 0, dur: 0.5 }); };
  Scene.prototype.changed = function () { if (this.o.onChange) this.o.onChange(this); };
  Scene.prototype.drawBot = function (b) {
    var g = this.g, t = this.t, bob = 0;
    if (b.state === "work") bob = Math.floor((t + b.seed) * 3) % 2;
    if (b.state === "walk") bob = Math.floor((t + b.seed) * 6) % 2;
    var jy = b.jump > 0 ? Math.round(Math.sin((0.35 - b.jump) / 0.35 * Math.PI) * 5) : 0;
    var blink = !this.still && ((t + b.blinkAt) % 4) < 0.14;
    g.drawImage(sprite(b.kind, this.theme, blink), Math.round(b.x), Math.round(b.y - bob - jy));
    if (b.state === "work") {
      laptop(g, b.x + 14, b.y + 10, this.theme, Math.floor((t + b.seed) * 2) % 2 === 0);
      if (Math.floor((t + b.seed) * 1.5) % 3 === 0) {
        for (var i = 0; i < 3; i++) rect(g, b.x + 6 + i * 2, b.y - 3 - ((Math.floor(t * 6) + i) % 3 === 0 ? 1 : 0), 1, 1, this.theme === "dark" ? V[8] : V[4]);
      }
    }
    if (b.state === "wait") flag(g, b.x + 13, b.y - 9 - jy, t);
  };
  Scene.prototype.draw = function () {
    var g = this.g;
    if (!this.bg) {
      this.bg = document.createElement("canvas");
      this.bg.width = this.W; this.bg.height = this.H;
      this.o.background(this, this.bg.getContext("2d"));
    }
    g.drawImage(this.bg, 0, 0);
    if (this.o.ambient) this.o.ambient(this, g);
    var self = this;
    this.bots.forEach(function (b) { self.drawBot(b); });
    this.fx.forEach(function (f) {
      if (f.kind === "hand") {
        var p = Math.min(1, f.t / f.dur);
        dottedArc(g, f.a, f.b, f.h, (self.t * 4) % 1, self.theme === "dark" ? V[8] : V[4], p);
        var q = arcPoint(f.a, f.b, f.h, p);
        packet(g, q.x, q.y, self.theme);
      } else if (f.kind === "ripple") {
        var r = Math.round(1 + f.t / f.dur * 6);
        g.fillStyle = self.theme === "dark" ? V[8] : V[4];
        for (var a = 0; a < 12; a++) {
          var ang = a / 12 * Math.PI * 2;
          g.fillRect(Math.round(f.x + Math.cos(ang) * r), Math.round(f.y + Math.sin(ang) * r * 0.6), 1, 1);
        }
      }
    });
    if (this.o.foreground) this.o.foreground(this, g);
    // Reveal the canvas only once a whole frame has drawn; until then the still image behind it shows.
    if (!this.shown && this.o.host) { this.shown = true; this.o.host.classList.add("live"); }
  };
  Scene.prototype.step = function (dt) {
    this.t += dt;
    this.bots.forEach(function (b) {
      if (b.jump > 0) b.jump = Math.max(0, b.jump - dt);
      if (b.state === "walk") {
        var d = b.target - b.x;
        if (Math.abs(d) < 0.6) { b.x = b.target; b.state = b.after || "idle"; }
        else b.x += Math.sign(d) * Math.min(Math.abs(d), dt * 14);
      }
    });
    var keep = [];
    this.fx.forEach(function (f) {
      f.t += dt;
      if (f.t >= f.dur) { if (f.end) f.end(); } else keep.push(f);
    });
    this.fx = keep;
    if (this.o.tick) this.o.tick(this, dt);
  };
  Scene.prototype.start = function () {
    if (reduce.matches) {
      this.still = true;
      this.fx = [];
      if (this.o.still) this.o.still(this);
      this.draw();
      return;
    }
    this.still = false;
    if (this.running || !this.visible || document.hidden) return;
    this.running = true;
    var self = this, last = performance.now();
    function frame(now) {
      if (!self.visible || document.hidden || reduce.matches) { self.running = false; return; }
      self.step(Math.min(0.1, (now - last) / 1000));
      last = now;
      self.draw();
      requestAnimationFrame(frame);
    }
    requestAnimationFrame(frame);
  };
  Scene.prototype.click = function (x, y, keyboard) {
    if (this.o.click) this.o.click(this, x, y, keyboard);
    if (this.still) {
      // Reduced motion: apply the result at once and show it as a still.
      this.fx.forEach(function (f) { if (f.end) f.end(); });
      this.fx = [];
      this.bots.forEach(function (b) { b.jump = 0; if (b.state === "walk") { b.x = b.target; b.state = b.after || "idle"; } });
      this.draw();
    }
  };

  function pick(list, r) { return list[Math.floor((r || Math.random)() * list.length)]; }
  function bots(s, state) { return s.bots.filter(function (b) { return b.state === state; }); }
  var KINDS = ["antenna", "visor", "beret", "goggles", "crown", "hardhat", "glasses", "ears", "spikes", "bulb"];

  // ---- The night town: hero and the closing band ----
  function town(canvas, opts) {
    opts = opts || {};
    var stars = [], windows = [];
    return new Scene(canvas, {
      theme: "dark",
      scale: function (w) { return w < 600 ? 3 : 4; },
      layout: function (s) {
        var r = rng(7), W = s.W, H = s.H;
        s.ground = H - Math.max(14, Math.round(H * 0.13));
        stars = []; windows = [];
        for (var i = 0; i < W * H / 300; i++) {
          var sx = Math.floor(r() * W), sy = Math.floor(r() * s.ground * 0.75);
          // Keep the sky behind the headline quiet.
          if (!opts.low && !opts.framed && sx < W * 0.6 && sy > s.ground * 0.1 && r() < 0.8) continue;
          stars.push({ x: sx, y: sy, p: r() * 6 });
        }
        s.buildings = [];
        var x = -4;
        while (x < W) {
          var w = 10 + Math.floor(r() * 16);
          // Lower roofs on the left of the hero, where the headline sits.
          var maxH = opts.low || !opts.framed && x < W * 0.55 && W > 200 ? 0.26 : 0.5;
          var h = Math.floor((0.12 + r() * maxH) * s.ground);
          s.buildings.push({ x: x, w: w, h: h, far: r() < 0.45 });
          x += w + (r() < 0.3 ? 2 : 0);
        }
        s.buildings.forEach(function (b) {
          if (b.far) return;
          for (var wy = s.ground - b.h + 3; wy < s.ground - 4; wy += 4)
            for (var wx = b.x + 2; wx < b.x + b.w - 2; wx += 3)
              if (r() < 0.32) windows.push({ x: wx, y: wy, on: r() < 0.7, flip: r() * 20 });
        });
        var n = Math.max(3, Math.min(opts.count || 7, Math.floor(W / 34)));
        s.bots = [];
        var gap = W / n, start = gap / 2 - 12;
        for (var k = 0; k < n; k++) {
          var b = s.addBot(KINDS[k % KINDS.length], Math.round(start + k * gap), s.ground - 15, k % 3 === 1 ? "idle" : "work");
          b.home = b.x;
        }
        if (s.bots[3]) s.bots[3].state = "wait";
        s.next = 1.2;
        s.changed();
      },
      background: function (s, g) {
        var W = s.W, H = s.H, gy = s.ground, mx = Math.round(W * 0.78), my = Math.round(gy * 0.32);
        for (var y = 0; y < gy; y++) {
          for (var x = 0; x < W; x++) {
            var d = Math.hypot(x - mx, (y - my) * 1.3);
            var v = 1.4 + (y / gy) * 2.6 + Math.max(0, 3.4 - d / 14);
            g.fillStyle = ditherColor(Math.min(8.6, v), x, y);
            g.fillRect(x, y, 1, 1);
          }
        }
        // The moon
        for (var yy = -6; yy <= 6; yy++) for (var xx = -6; xx <= 6; xx++)
          if (xx * xx + yy * yy <= 34) rect(g, mx + xx, my + yy, 1, 1, xx * xx + yy * yy > 22 ? V[8] : V[9]);
        rect(g, mx - 2, my - 1, 2, 2, V[8]); rect(g, mx + 2, my + 2, 1, 1, V[8]);
        s.buildings.forEach(function (b) {
          rect(g, b.x, gy - b.h, b.w, b.h, b.far ? V[2] : V[1]);
          if (!b.far && b.w > 12) rect(g, b.x + 3, gy - b.h - 3, 2, 3, V[1]);
        });
        // Street and its reflections
        for (var sy = gy; sy < H; sy++) for (var sx = 0; sx < W; sx++) {
          var refl = Math.abs(sx - mx) < 7 - (sy - gy) * 0.2 && (sx + sy * 3) % 5 < 2;
          g.fillStyle = refl ? V[4] : ditherColor(1.6 - (sy - gy) / (H - gy) * 0.8, sx, sy);
          g.fillRect(sx, sy, 1, 1);
        }
        rect(g, 0, gy, W, 1, V[3]);
      },
      ambient: function (s, g) {
        stars.forEach(function (st) {
          var on = s.still ? st.p > 1.5 : ((s.t + st.p) % 6) > 0.4;
          if (on) rect(g, st.x, st.y, 1, 1, st.p > 5 ? V[9] : V[6]);
        });
        windows.forEach(function (w) {
          var on = s.still ? w.on : ((s.t + w.flip) % 20 < 17) === w.on;
          rect(g, w.x, w.y, 1, 2, on ? V[6] : V[2]);
        });
      },
      tick: function (s, dt) {
        s.next -= dt;
        if (s.next > 0) return;
        s.next = 1.8 + Math.random() * 1.6;
        var work = bots(s, "work"), idle = bots(s, "idle");
        if (work.length && idle.length && Math.random() < 0.75) {
          var a = pick(work), b = pick(idle);
          a.state = "idle";
          s.handoff(a, b, 10 + Math.abs(a.x - b.x) / 5);
        } else if (!bots(s, "wait").length && work.length > 1) {
          pick(work).state = "wait";
        } else if (idle.length > work.length) {
          pick(idle).state = "work";
        }
        s.changed();
      },
      still: function (s) {
        s.bots.forEach(function (b, i) { b.state = i === 3 ? "wait" : i % 3 === 1 ? "idle" : "work"; b.x = b.home; b.jump = 0; });
        s.changed();
      },
      foreground: function (s, g) {
        if (!s.still || s.bots.length < 3) return;
        // A frozen handoff, so the still frame shows one bot passing work to another.
        var a = s.bots[0], b = s.bots[1];
        dottedArc(s.g, { x: a.x + 8, y: a.y + 4 }, { x: b.x + 8, y: b.y + 4 }, 12, 0, V[8], 1);
        var q = arcPoint({ x: a.x + 8, y: a.y + 4 }, { x: b.x + 8, y: b.y + 4 }, 12, 0.55);
        packet(g, q.x, q.y, "dark");
      },
      click: function (s, x, y, keyboard) {
        var waiting = bots(s, "wait");
        var hit = waiting.filter(function (b) { return x > b.x - 4 && x < b.x + 24 && y > b.y - 14 && y < b.y + 20; })[0];
        if (keyboard && waiting.length) hit = waiting[0];
        if (hit) { approve(s, hit); return; }
        var idle = bots(s, "idle"), pool = idle.length ? idle : s.bots;
        var near = keyboard ? pick(pool) : pool.reduce(function (m, b) { return Math.abs(b.x - x) < Math.abs(m.x - x) ? b : m; });
        if (keyboard) { x = near.x + 8; y = Math.max(4, near.y - 30); }
        s.ripple(x, y);
        s.fx.push({ kind: "hand", a: { x: x, y: y }, b: { x: near.x + 8, y: near.y + 4 }, h: 6, t: 0, dur: 0.9,
          end: function () { near.state = "work"; near.jump = 0.35; s.changed(); } });
        s.changed();
      },
      host: opts.host,
      onChange: opts.onChange
    });
  }
  function approve(s, b) { b.state = "work"; b.jump = 0.35; s.changed(); }

  // Each scene starts on its own, so one that fails leaves only its own still image, not a blank page section.
  function guard(fn) {
    try { fn(); } catch (e) { if (window.console) console.warn("kwant scene skipped:", e); }
  }

  // ---- Hero ----
  var hero = document.querySelector("[data-scene=hero]");
  if (hero) guard(function () {
    var chip = document.querySelector("[data-scene-status]");
    var okBtn = document.querySelector("[data-scene-approve]");
    var heroScene = town(hero.querySelector("canvas"), {
      framed: true,
      host: hero,
      onChange: function (s) {
        if (!chip) return;
        var w = bots(s, "work").length + s.fx.filter(function (f) { return f.kind === "hand"; }).length;
        var wait = bots(s, "wait").length;
        chip.querySelector("b").textContent = Math.min(w, s.bots.length);
        chip.querySelector("span").textContent = w === 1 ? " bot working" : " bots working";
        if (okBtn) okBtn.hidden = !wait;
      }
    });
    if (okBtn) okBtn.addEventListener("click", function () {
      var w = bots(heroScene, "wait")[0];
      if (w) { approve(heroScene, w); if (heroScene.still) heroScene.draw(); }
    });
    heroScene.start();
  });

  // ---- Closing panel and 404: the same town, quieter ----
  Array.prototype.forEach.call(document.querySelectorAll("[data-scene=street]"), function (el) {
    guard(function () {
      town(el.querySelector("canvas"), { low: true, host: el, count: el.hasAttribute("data-lost") ? 1 : 6 }).start();
    });
  });
})();
