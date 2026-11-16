/* kwant perch: the page. Built from Mira's reference renderer (autonomy/desktop-island/mockups/src/perch.js,
   spec design.md) and wired to Rust (src/perch.rs page_json / parse_perch, src/perch_window.rs).
   Every server string goes in through textContent (never innerHTML). Only our own generated SVG uses
   innerHTML. The page decides peek / expanded / celebrate and its timings; Rust decides what it may do. */
(function () {
  'use strict';
  var C = window.kwCreature;
  var HOLD = 5000;          // a peek shows this long (design.md §6)
  var AFTER_HOVER = 2000;   // and closes this long after the pointer leaves
  var CELEBRATE = 2400;
  var SENT_FOR = 3000;
  var CALL_TIMEOUT = 10000; // a call with no answer by then counts as failed
  var ASK_MAX = 8000;       // perch.rs ASK_MAX

  // ---------------------------------------------------------------------------------------------
  // Talking to Rust: one navigation at a time (two in the same moment may lose the first).
  var queue = [], pumping = false;
  function send(what) { queue.push(what); if (!pumping) pump(); }
  function pump() {
    var next = queue.shift();
    if (next === undefined) { pumping = false; return; }
    pumping = true;
    location.href = 'kwant-island://' + next;
    setTimeout(pump, 40);
  }
  function thing(p, id) { return 'p=' + Number(p) + '&id=' + encodeURIComponent(id); }

  // ---------------------------------------------------------------------------------------------
  // Building blocks (Mira's)
  function el(tag, attrs, kids) {
    var n = document.createElement(tag);
    for (var k in (attrs || {})) {
      var v = attrs[k]; if (v === false || v == null) continue;
      if (k === 'class') n.className = v; else if (k === 'text') n.textContent = v; else if (k === 'html') n.innerHTML = v; else n.setAttribute(k, v === true ? '' : v);
    }
    [].concat(kids || []).forEach(function (c) { if (c != null && c !== false) n.append(typeof c === 'string' ? document.createTextNode(c) : c); });
    return n;
  }
  var ICON = {
    open: '<path d="M7 7h10v10"/><path d="M7 17 17 7"/>',
    check: '<path d="M20 6 9 17l-5-5"/>',
    x: '<path d="M18 6 6 18"/><path d="m6 6 12 12"/>',
    send: '<path d="m5 12 7-7 7 7"/><path d="M12 19V5"/>',
    chev: '<path d="m6 9 6 6 6-6"/>',
  };
  function icon(name, cls) { return el('span', { class: 'ico', html: '<svg class="' + (cls || '') + '" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.25" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">' + ICON[name] + '</svg>' }).firstChild; }
  var GLYPH = {
    mac: ['.......', '.#####.', '.#...#.', '.#...#.', '.#####.', '#######', '.......'],
    server: ['.#####.', '.#..#..', '.#####.', '.......', '.#####.', '.#..#..', '.#####.'],
  };
  function glyph(kind) {
    var rows = GLYPH[kind] || GLYPH.server, r = '';
    rows.forEach(function (row, y) { Array.from(row).forEach(function (ch, x) { if (ch === '#') r += '<rect x="' + x + '" y="' + y + '" width="1" height="1"/>'; }); });
    return el('span', { html: '<svg class="glyph" viewBox="0 0 7 7" fill="currentColor" shape-rendering="crispEdges" aria-hidden="true">' + r + '</svg>' }).firstChild;
  }
  // the creature's SVG is ours: the name only picks a palette, it never goes into the markup
  function face(who, size, opts) { who = who || {}; return el('span', { class: 'face', html: C.creatureSvg(who.avatar || null, String(who.name || ''), size, opts) }); }
  function bdi(s, cls) { return el('bdi', { text: String(s || ''), class: cls || false, dir: 'auto' }); }
  function midcut(s, max) { var a = Array.from(String(s || '')); if (a.length <= max) return a.join(''); var keep = max - 1, h = Math.ceil(keep / 2); return a.slice(0, h).join('') + '…' + a.slice(a.length - (keep - h)).join(''); }
  function countLabel(n) { return n > 9 ? '9+' : String(n); }
  function key(p, id) { return p + '#' + id; }
  function ago(since, long) {
    var t = Date.parse(since || '');
    if (!isFinite(t)) return long ? '' : 'now';
    var m = Math.max(0, Math.floor((Date.now() - t) / 60000));
    if (m < 1) return long ? 'just now' : 'now';
    if (m < 60) return long ? m + ' min' : m + 'm';
    var h = Math.floor(m / 60);
    if (h < 24) return h + ' h';
    return Math.floor(h / 24) + ' d';
  }

  // ---------------------------------------------------------------------------------------------
  // State
  var D = null;              // the latest update from Rust
  var open = null;           // null | 'peek' | 'expanded'
  var openBy = null;         // 'key' | 'click' | 'hover' | 'event'
  var peekKey = null;        // the item a peek shows
  var lastPeek = null, lastCelebrate = null, celebrateUntil = 0, celebrateKey = null;
  var lastToggles = null;
  var hovering = false, focused = false;
  var holdTimer = null, leaveTimer = null, celebrateTimer = null;
  var busy = {};             // key → {op, timer}: a call on its way
  var failed = {};           // key → true: show "Didn't go through"
  var settled = {};          // key → true: went through, hidden until the next poll drops it
  var ask = { pick: null, text: '', sent: null, failed: false, cid: null };
  var lastSize = '';
  var lastState = 'hidden';
  var stage = document.getElementById('stage');

  function needs() { return (D && D.needs || []).filter(function (n) { return !settled[key(n.p, n.id)]; }); }
  function needsCount() { var all = D ? (D.counts && D.counts.needs) || 0 : 0; var hidden = (D && D.needs || []).length - needs().length; return Math.max(needs().length, all - hidden); }
  function working() { return D && D.working || []; }
  function workingCount() { return Math.max(working().length, D && D.counts ? D.counts.working || 0 : 0); }
  function finished() { return D && D.finished || []; }
  function findNeed(k) { return needs().filter(function (n) { return key(n.p, n.id) === k; })[0] || null; }

  function state() {
    if (!D) return 'hidden';
    if (open) return open;
    if (celebrateKey && Date.now() < celebrateUntil) return 'celebrate';
    if (needsCount() || workingCount()) return 'rest';
    if (D.mode === 'always') return 'rest';
    return 'hidden';
  }

  function clearTimers() { clearTimeout(holdTimer); clearTimeout(leaveTimer); holdTimer = leaveTimer = null; }

  function expand(by) {
    clearTimers();
    open = 'expanded'; openBy = by;
    render();
  }
  function peek(k, by) {
    if (open === 'expanded') return;
    var swap = open === 'peek' && peekKey !== k;
    open = 'peek'; openBy = by; peekKey = k;
    clearTimers();
    if (by !== 'hover') holdTimer = setTimeout(maybeClose, HOLD);
    render(swap);
  }
  function close(tellRust) {
    clearTimers();
    var was = open;
    open = null; openBy = null; peekKey = null;
    if (ask.text === '') ask.sent = null;
    if (was) render();
    if (tellRust) send('collapse');
  }
  // a peek or click-opened panel closes when the time is up, unless it is hovered, focused or typed in
  function maybeClose() {
    holdTimer = null;
    if (!open) return;
    if (open === 'expanded' && (openBy === 'key' || ask.text)) return;
    if (hovering || focused) return;
    close(true);
  }

  // ---------------------------------------------------------------------------------------------
  // The band (both ears)
  function band(st) {
    var left = el('div', { class: 'perch__ear perch__ear--l' });
    var right = el('div', { class: 'perch__ear perch__ear--r' });
    var faces = el('div', { class: 'perch__faces' });
    var who = working().map(function (w) { return w.who; });
    var celebrating = st === 'celebrate' ? finished().filter(function (f) { return key(f.p, f.id) === celebrateKey; })[0] : null;
    if (celebrating) who = [celebrating.who].concat(who.filter(function (w) { return w.name !== celebrating.who.name; }));
    var total = celebrating ? Math.max(who.length, workingCount()) : workingCount();
    var shown = total > 3 ? who.slice(0, 2) : who.slice(0, 3);
    var size = D.shape === 'notch' ? 20 : 18;
    shown.forEach(function (w, i) {
      var f = face(w, size);
      if (celebrating && i === 0) f.setAttribute('data-hop', ''); else f.setAttribute('data-working', '');
      faces.append(f);
    });
    if (total > 3) faces.append(el('span', { class: 'perch__more', text: '+' + (total - 2), 'aria-label': (total - 2) + ' more working' }));
    // Always, nothing going on: the floating tab shows kwant asleep; the notch shows two empty ears
    if (!shown.length && !celebrating && !needsCount() && D.shape !== 'notch' && st === 'rest') {
      faces.append(face({ name: 'kwant', avatar: 'brand' }, size, { asleep: true }));
    }
    left.append(faces);
    var n = needsCount();
    if (celebrating) {
      right.append(el('span', { class: 'perch__holo', role: 'img', 'aria-label': celebrating.who.name + ' finished: checked', html: '<svg viewBox="0 0 24 24" fill="none" stroke="#1b1033" stroke-width="3.5" stroke-linecap="square" aria-hidden="true"><path d="M20 6 9 17l-5-5"/></svg>' }));
    } else if (n) {
      var c = el('span', { class: 'perch__count perch__count--needs', text: countLabel(n), role: 'img', 'aria-label': n + ' need you' });
      if (n !== lastNeeds) c.setAttribute('data-pop', '');
      right.append(c);
    } else if (workingCount()) {
      right.append(el('span', { class: 'perch__count perch__count--work', text: countLabel(workingCount()), role: 'img', 'aria-label': workingCount() + ' working' }));
    }
    lastNeeds = n;
    var b = el('div', { class: 'perch__band', title: D.key ? 'Open the perch (' + D.key + ')' : 'Open the perch' }, [left, el('div', { class: 'perch__gap' }), right]);
    b.addEventListener('click', function () {
      if (open === 'expanded') close(true); else expand('click');
    });
    b.addEventListener('mouseenter', function () {
      if (!open && needs().length) peek(key(needs()[0].p, needs()[0].id), 'hover');
    });
    return b;
  }
  var lastNeeds = 0;

  // ---------------------------------------------------------------------------------------------
  // One needs-you item: the sentence and what you can do
  function verb(it) { return it.kind === 'question' ? ' asks: ' : it.kind === 'approval' ? ' wants your OK: ' : ' needs you: '; }
  function sentence(it) {
    var who = it.who.name;
    return el('div', { class: 'perch__line', title: who + verb(it) + it.title }, [el('b', {}, [bdi(who)]), verb(it), bdi(it.title)]);
  }
  function button(cls, label, onClick, attrs) {
    var b = el('button', Object.assign({ class: 'pb ' + cls, type: 'button' }, attrs || {}), label);
    b.addEventListener('click', function (e) { e.stopPropagation(); onClick(b); });
    return b;
  }
  function openButton(it, label, primary) {
    return button(primary ? 'pb--primary' : 'pb--ghost', [label, icon('open')], function () { send('open?' + thing(it.p, it.id)); }, { 'aria-label': label === 'Open' ? 'Open in kwant' : label });
  }
  function call(it, op, extra) {
    var k = key(it.p, it.id);
    if (busy[k]) return;
    delete failed[k];
    busy[k] = { op: op, timer: setTimeout(function () { finish({ op: op, id: it.id, ok: false, code: 'timeout' }, k); }, CALL_TIMEOUT) };
    send(op + '?' + thing(it.p, it.id) + (extra || ''));
    render();
  }
  function actions(it) {
    var k = key(it.p, it.id);
    var acts = el('div', { class: 'perch__acts' });
    var b = busy[k] ? busy[k].op : null;
    if (failed[k]) {
      acts.append(el('span', { class: 'perch__fail', role: 'status', text: "Didn't go through. Open it in kwant." }), openButton(it, 'Open'));
      return acts;
    }
    var multi = D.multiPlace || D.multiSpace;
    if (it.kind === 'approval' && it.inline === true) {
      var where = [it.space, D.multiPlace && it.place ? midcut(it.place, 24) : null].filter(Boolean).join(' · ');
      if (multi && where) acts.append(el('span', { class: 'perch__where', title: [it.space, it.place].filter(Boolean).join(' · '), text: where }));
      else acts.append(el('span', { style: 'flex:1' }));
      // Allow once is never focused for you: a stray Enter must never approve
      acts.append(
        button('pb--primary', 'Allow once', function () { call(it, 'allow'); }, { 'data-busy': b === 'allow', disabled: !!b }),
        button('', 'Deny', function () { call(it, 'deny'); }, { 'data-busy': b === 'deny', disabled: !!b }),
        openButton(it, 'Open'));
    } else if (it.kind === 'question') {
      (it.choices || []).slice(0, 3).forEach(function (c, i) {
        acts.append(button('pb--chip', [el('span', { text: String(c) })], function () { call(it, 'answer', '&c=' + i); }, { title: String(c), 'data-busy': b === 'answer', disabled: !!b }));
      });
      acts.append(el('span', { style: 'flex:1' }));
      acts.append(openButton(it, it.reply || !(it.choices || []).length ? 'Reply' : 'Open'));
    } else {
      acts.append(el('span', { class: 'perch__note', title: it.why || false, text: 'Decide in kwant: this one needs the full card.' }));
      acts.append(openButton(it, 'Open in kwant', true));
    }
    return acts;
  }
  function needItem(it, inPeek) {
    var more = needsCount() - 1;
    var aside = el('span', { class: 'perch__aside', text: inPeek && more > 0 ? '+' + more + ' more' : ago(it.since) });
    return el('div', { class: 'perch__item' }, [face(it.who, 20), sentence(it), aside, actions(it)]);
  }

  // ---------------------------------------------------------------------------------------------
  // Expanded
  function head(label, cls, extra) { return el('div', { class: 'perch__head ' + (cls || '') }, [label, extra ? el('span', { class: 'n', text: extra }) : null]); }
  function moreLine(n, label, what) {
    var a = el('a', { class: 'perch__more-line', role: 'link', tabindex: '0' }, ['and ' + n + ' more · ', el('u', { text: label })]);
    var go = function () { send('open?what=' + what); };
    a.addEventListener('click', go);
    a.addEventListener('keydown', function (e) { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); go(); } });
    return a;
  }
  function row(item, kids, attrs) {
    var r = el('div', Object.assign({ class: 'perch__row', role: 'button', tabindex: '0' }, attrs || {}), kids);
    var go = function () { send('open?' + thing(item.p, item.id)); };
    r.addEventListener('click', go);
    r.addEventListener('keydown', function (e) { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); go(); } });
    return r;
  }
  function expanded(compact) {
    var body = el('div', { class: 'perch__body' });
    var ns = needs(), ws = working(), fs = finished().slice().sort(function (a, b) { return (b.unseen ? 1 : 0) - (a.unseen ? 1 : 0); });
    var maxNeeds = compact ? 1 : 2, maxWork = compact ? 1 : 3, maxFin = compact ? 1 : 2;
    if (!ns.length && !ws.length && !fs.length) {
      body.append(el('div', { class: 'perch__quiet' }, [face({ name: 'kwant', avatar: 'brand' }, 32, { asleep: true }), el('span', { text: 'All quiet. Nobody needs you and nobody is working.' })]));
    }
    if (ns.length) {
      var nc = needsCount();
      var s = el('section', { class: 'perch__sec', 'aria-label': 'Needs you' }, [head('Needs you', 'perch__head--needs', '· ' + nc)]);
      ns.slice(0, maxNeeds).forEach(function (it) { s.append(el('div', { class: 'perch__card' }, [needItem(it, false)])); });
      if (nc > maxNeeds) s.append(moreLine(nc - maxNeeds, 'Open Inbox', 'inbox'));
      body.append(s);
    }
    if (ws.length) {
      var wc = workingCount();
      var s2 = el('section', { class: 'perch__sec', 'aria-label': 'Working now' }, [head('Working now', 'perch__head--work', '· ' + wc)]);
      ws.slice(0, maxWork).forEach(function (w) {
        s2.append(row(w, [face(w.who, 20), el('div', { class: 'perch__line', title: w.who.name + ' · ' + w.step }, [el('b', {}, [bdi(w.who.name)]), w.step ? ' · ' : '', bdi(w.step)]), el('span', { class: 'perch__aside', text: ago(w.since, true) })]));
      });
      if (wc > maxWork) s2.append(moreLine(wc - maxWork, 'Open Live now', 'live'));
      body.append(s2);
    }
    if (fs.length) {
      var s3 = el('section', { class: 'perch__sec', 'aria-label': 'Just finished' }, [head('Just finished')]);
      fs.slice(0, maxFin).forEach(function (f) {
        var chk = f.check === 'verified' ? el('span', { class: 'perch__check perch__check--ok' }, [icon('check'), 'Checked'])
          : f.check === 'failed' ? el('span', { class: 'perch__check perch__check--bad' }, [icon('x'), 'Check failed'])
          : el('span', { class: 'perch__check perch__check--none', text: 'Not checked' });
        s3.append(row(f, [face(f.who, 20), el('div', { class: 'perch__line', title: f.who.name + ' · ' + f.title }, [el('b', {}, [bdi(f.who.name)]), ' · ', bdi(f.title, 't')]), chk], { 'data-unseen': !!f.unseen }));
      });
      if (fs.length > maxFin) s3.append(moreLine(fs.length - maxFin, 'Open Activity', 'activity'));
      body.append(s3);
    }
    var targets = D.ask || [];
    if (targets.length) body.append(quickAsk(targets));
    var places = D.places || [];
    if (places.length > 1) {
      var list = el('div', { class: 'perch__places', role: 'group', 'aria-label': 'Places' });
      var show = places.length > 3 ? places.slice(0, 2) : places;
      show.forEach(function (p) {
        var label = p.kind === 'server' ? midcut(p.name, 22) : p.name;
        var tip = p.name + (p.off ? " · Can't reach it" : p.waiting ? ' · ' + p.waiting + ' waiting' : '');
        var b = el('button', { class: 'perch__place', type: 'button', 'aria-current': p.current ? 'true' : false, 'data-off': !!p.off, title: tip, 'aria-label': tip },
          [glyph(p.kind === 'mac' ? 'mac' : 'server'), el('span', { text: label }), p.waiting ? el('span', { class: 'n', text: countLabel(p.waiting) }) : null]);
        b.addEventListener('click', function () { send('place?i=' + Number(p.i)); });
        list.append(b);
      });
      if (places.length > 3) {
        var more = el('button', { class: 'perch__place', type: 'button', title: 'All places', text: '+' + (places.length - 2), 'aria-label': (places.length - 2) + ' more places, show all' });
        more.addEventListener('click', function () { send('places'); });
        list.append(more);
      }
      body.append(el('section', { class: 'perch__sec perch__sec--places', 'aria-label': 'Places' }, [head('Places'), list]));
    }
    var foot = el('div', { class: 'perch__foot' });
    foot.append(D.key ? el('span', { class: 'perch__keys' }, [el('kbd', { text: D.key }), ' opens and closes this']) : el('span'));
    foot.append(button('pb--ghost', ['Open kwant', icon('open')], function () { send('open?what=app'); }));
    body.append(foot);
    return body;
  }

  function quickAsk(targets) {
    // keep the person's pick across updates (by who it is, not by its index)
    var t = targets.filter(function (x) { return ask.pick && x.p === ask.pick.p && x.who.name === ask.pick.name; })[0] || targets[0];
    ask.pick = { p: t.p, name: t.who.name };
    var to = t.who;
    var s = el('section', { class: 'perch__sec', 'aria-label': 'Quick ask' }, [head('Quick ask')]);
    var pick = el('select', { 'aria-label': 'Choose who to ask' });
    targets.forEach(function (x) {
      var o = el('option', { value: String(x.t), text: x.who.name + (x.place ? ' · ' + midcut(x.place, 24) : '') });
      if (x === t) o.selected = true;
      pick.append(o);
    });
    pick.addEventListener('mousedown', function () { send('focus'); });
    pick.addEventListener('change', function () {
      var x = targets.filter(function (y) { return String(y.t) === pick.value; })[0];
      if (x) { ask.pick = { p: x.p, name: x.who.name }; ask.sent = null; render(); focusAsk(); }
    });
    var input = el('input', { type: 'text', id: 'ask', maxlength: String(ASK_MAX), placeholder: 'Ask ' + to.name + ' something…', 'aria-label': 'Message to ' + to.name, autocomplete: 'off', spellcheck: 'true', dir: 'auto' });
    input.value = ask.text;
    var go = button((ask.text.trim() ? 'pb--primary ' : '') + 'send', [icon('send')], function () { submit(t); }, { 'aria-label': 'Send', disabled: !ask.text.trim() || !!ask.cid, 'data-busy': !!ask.cid });
    input.addEventListener('mousedown', function () { send('focus'); });
    input.addEventListener('input', function () {
      ask.text = input.value; ask.failed = false;
      var on = !!ask.text.trim();
      go.disabled = !on || !!ask.cid; go.classList.toggle('pb--primary', on);
    });
    input.addEventListener('keydown', function (e) { if (e.key === 'Enter' && !e.isComposing) { e.preventDefault(); submit(t); } });
    s.append(el('div', { class: 'perch__ask' }, [
      el('label', { class: 'perch__ask-who', title: 'Choose who to ask' }, [face(to, 16), el('span', {}, [bdi(to.name)]), icon('chev', 'chev'), pick]),
      input, go,
    ]));
    if (ask.failed) {
      s.append(el('div', { class: 'perch__sent', role: 'status' }, [el('span', { class: 'perch__fail', text: "Didn't go through. Try again, or ask in kwant." })]));
    } else if (ask.sent && Date.now() < ask.sent.until) {
      var openChat = el('u', { role: 'link', tabindex: '0', text: 'Open chat' });
      var sent = ask.sent;
      var goChat = function () { send('open?' + thing(sent.p, sent.cid)); };
      openChat.addEventListener('click', goChat);
      openChat.addEventListener('keydown', function (e) { if (e.key === 'Enter') { e.preventDefault(); goChat(); } });
      s.append(el('div', { class: 'perch__sent', role: 'status' }, [icon('check'), el('span', {}, ['Sent to ', bdi(sent.name), ". It's in your chat."]), openChat]));
    }
    return s;
  }
  function focusAsk() { var i = document.getElementById('ask'); if (i) { send('focus'); i.focus(); } }
  function newCid() {
    var b = new Uint8Array(16); crypto.getRandomValues(b);
    return 'island-' + Array.from(b, function (x) { return (x < 16 ? '0' : '') + x.toString(16); }).join('');
  }
  function submit(t) {
    var text = ask.text.trim();
    if (!text || ask.cid) return;
    ask.cid = newCid(); ask.failed = false; ask.sent = null;
    ask.to = { p: t.p, name: t.who.name };
    clearTimeout(ask.timer);
    ask.timer = setTimeout(function () { finish({ op: 'ask', id: ask.cid, ok: false, code: 'timeout' }); }, CALL_TIMEOUT);
    send('ask?t=' + Number(t.t) + '&text=' + encodeURIComponent(text.slice(0, ASK_MAX)) + '&cid=' + ask.cid);
    render();
  }

  // ---------------------------------------------------------------------------------------------
  // Rendering and the size Rust needs
  var lastRendered = 'hidden';
  function render(swap) {
    var st = state();
    var wasHidden = lastRendered === 'hidden';
    var wasOpen = lastRendered === 'peek' || lastRendered === 'expanded';
    var nowOpen = st === 'peek' || st === 'expanded';
    // keep the Quick ask field's focus and caret through a re-render
    var active = document.activeElement && document.activeElement.id === 'ask' ? document.activeElement : null;
    var caret = active ? [active.selectionStart, active.selectionEnd] : null;
    if (st === 'hidden') {
      lastRendered = st;
      var old = stage.querySelector('.perch');
      if (old && !stage.hidden) {
        old.setAttribute('data-leave', '');
        setTimeout(function () { if (state() === 'hidden') { stage.hidden = true; stage.replaceChildren(); report('hidden', 0, 0); } }, reduced() ? 160 : 160);
      } else {
        stage.hidden = true; report('hidden', 0, 0);
      }
      return;
    }
    var root = el('div', { class: 'perch', 'data-shape': D.shape === 'notch' ? 'notch' : 'float', 'data-state': st, role: 'region', 'aria-label': 'kwant perch' });
    if (D.shape === 'notch' && D.notch) {
      root.style.setProperty('--notch-w', Math.max(80, Math.min(400, Number(D.notch.w) || 185)) + 'px');
      root.style.setProperty('--notch-h', Math.max(16, Math.min(64, Number(D.notch.h) || 37)) + 'px');
    }
    if (wasHidden) root.setAttribute('data-enter', '');
    else if (nowOpen && !wasOpen) root.setAttribute('data-grow', '');
    root.append(el('div', { class: 'perch__edge' }), el('div', { class: 'perch__fill' }));
    var inner = el('div', { class: 'perch__in' });
    inner.append(band(st));
    if (st === 'peek') {
      var it = findNeed(peekKey);
      var b = el('div', { class: 'perch__body', 'data-swap': swap ? '' : false });
      if (it) b.append(needItem(it, true));
      inner.append(b);
    }
    if (st === 'expanded') inner.append(expanded(compact));
    root.append(inner);
    root.addEventListener('mouseenter', function () { hovering = true; clearTimeout(leaveTimer); });
    root.addEventListener('mouseleave', function () {
      hovering = false;
      if (open && !(open === 'expanded' && openBy === 'key')) { clearTimeout(leaveTimer); leaveTimer = setTimeout(maybeClose, AFTER_HOVER); }
    });
    stage.hidden = false;
    stage.toggleAttribute('data-open', nowOpen);
    stage.replaceChildren(root);
    lastRendered = st;
    if (active) { var again = document.getElementById('ask'); if (again) { again.focus(); try { again.setSelectionRange(caret[0], caret[1]); } catch (e) { /* not a text field any more */ } } }
    // The window shows only once Rust has this size (perch_window.rs place), and WKWebView runs no
    // animation frames in a window that isn't on screen, so a frame alone never came while the
    // window was still hidden. Measure on the first frame or the first timer, whichever comes
    // first (offsetWidth lays out either way).
    var measured = false;
    function measureOnce() { if (measured) return; measured = true; measure(st, root); }
    requestAnimationFrame(measureOnce);
    setTimeout(measureOnce, 0);
  }
  var compact = false;
  function measure(st, root) {
    if (!root.isConnected) return;
    // layout size, not the animated one (transforms don't change offsetWidth)
    var w = Math.ceil(root.offsetWidth), h = Math.ceil(root.offsetHeight);
    // small screens: drop to one row per section when it would come within 80 px of the bottom
    if (st === 'expanded') {
      var room = (window.screen && window.screen.availHeight || 900) - 80;
      if (!compact && h > room) { compact = true; render(); return; }
      if (compact && h < room - 200) { compact = false; render(); return; }
    }
    report(st, w, h);
  }
  function report(st, w, h) {
    var s = 's=' + st + '&w=' + Math.min(9999, w) + '&h=' + Math.min(9999, h);
    if (s === lastSize) return;
    lastSize = s;
    lastState = st;
    send('size?' + s);
  }
  function reduced() { return window.matchMedia && matchMedia('(prefers-reduced-motion: reduce)').matches; }

  // ---------------------------------------------------------------------------------------------
  // What Rust calls
  function update(d) {
    if (!d || typeof d !== 'object') return;
    var first = D === null;
    D = d;
    // things that went through stay hidden until the poll drops them
    Object.keys(settled).forEach(function (k) { if (!(d.needs || []).some(function (n) { return key(n.p, n.id) === k; })) delete settled[k]; });
    // the shortcut: each press flips open / closed (Rust says which in byKey)
    if (first || d.toggles !== lastToggles) {
      var flipped = !first && d.toggles !== lastToggles;
      lastToggles = d.toggles;
      if (d.byKey) { expand('key'); }
      else if (flipped && open) { close(false); }
    }
    // a new need peeks (unless the panel is open)
    var pk = d.peek ? key(d.peek.p, d.peek.id) : null;
    if (pk && pk !== lastPeek) {
      lastPeek = pk;
      if (!open && findNeed(pk)) { peek(pk, 'event'); return; }
    }
    // a verified finish celebrates once, 2.4 s
    var ck = d.celebrate ? key(d.celebrate.p, d.celebrate.id) : null;
    if (ck && ck !== lastCelebrate) {
      lastCelebrate = ck; celebrateKey = ck; celebrateUntil = Date.now() + CELEBRATE;
      clearTimeout(celebrateTimer);
      celebrateTimer = setTimeout(function () { celebrateKey = null; render(); }, CELEBRATE + 20);
    }
    // the peeked item went away: show the next one, or close
    if (open === 'peek' && !findNeed(peekKey)) {
      var next = needs()[0];
      if (next) { peekKey = key(next.p, next.id); render(true); return; }
      close(true); render(); return;
    }
    render();
  }

  function finish(msg, k) {
    msg = msg || {};
    if (msg.op === 'ask') {
      if (msg.id !== ask.cid) return;
      clearTimeout(ask.timer);
      if (msg.ok) { ask.sent = { p: ask.to.p, name: ask.to.name, cid: ask.cid, until: Date.now() + SENT_FOR }; ask.text = ''; setTimeout(render, SENT_FOR + 20); }
      else ask.failed = true;
      ask.cid = null;
      render();
      return;
    }
    // find the item by the op's id (Rust only echoes an id the page sent)
    if (!k) Object.keys(busy).forEach(function (b) { if (b.slice(b.indexOf('#') + 1) === msg.id && busy[b].op === msg.op) k = b; });
    if (!k || !busy[k]) return;
    clearTimeout(busy[k].timer);
    delete busy[k];
    if (msg.ok) {
      settled[k] = true;
      if (open === 'peek' && peekKey === k) {
        var next = needs()[0];
        if (next) { peekKey = key(next.p, next.id); render(true); return; }
        close(true); return;
      }
    } else {
      failed[k] = true;
    }
    render();
  }

  document.addEventListener('keydown', function (e) {
    if (e.key === 'Escape') { e.preventDefault(); close(true); }
  });
  document.addEventListener('focusin', function () { focused = true; clearTimeout(leaveTimer); });
  document.addEventListener('focusout', function () { focused = false; });
  // a click on the shadow margin is a click outside
  stage.addEventListener('mousedown', function (e) { if (e.target === stage && open) close(true); });

  window.kwantPerch = { update: update, done: function (m) { finish(m); }, collapse: function () { close(false); } };
  send('ready');
})();
