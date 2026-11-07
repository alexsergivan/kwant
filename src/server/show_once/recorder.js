// Show it once: the recorder (0.7.9). Runs in kwnt's own isolated world, so the page's scripts
// can't see it, call it or change it. It sends one line per action the person takes: never a
// keystroke, never a picture, and never the value of a masked field (decisions.md Q-S1). The
// server applies the same mask again to whatever arrives. Events a script dispatched
// (isTrusted false) are never recorded; a field whose value isn't all the person's own typing
// (a script, the vault or the page set any of it) is masked as a saved login.
(function () {
  if (globalThis.__kwntRecorder) return;
  globalThis.__kwntRecorder = true;
  var send = globalThis.__BINDING__;
  if (typeof send !== 'function') return;
  var WORDS = __MASK_WORDS__;
  var TEXT_TYPES = { '': 1, text: 1, email: 1, search: 1, tel: 1, url: 1, number: 1, password: 1,
    date: 1, 'datetime-local': 1, month: 1, week: 1, time: 1 };
  var everPassword = new WeakSet();
  var scripted = new WeakSet();
  var lastTrusted = new WeakMap();
  var committed = new WeakMap();
  var dirty = new Set();

  function cut(s, n) { s = String(s == null ? '' : s).replace(/\s+/g, ' ').trim(); return s.length > n ? s.slice(0, n) : s; }
  function post(o) { o.url = location.href; o.trusted = true; try { send(JSON.stringify(o)); } catch (e) {} }

  function isTextField(el) {
    if (!el || el.nodeType !== 1) return false;
    if (el.tagName === 'TEXTAREA') return true;
    if (el.tagName === 'INPUT') return !!TEXT_TYPES[(el.getAttribute('type') || '').toLowerCase()];
    return !!el.isContentEditable;
  }
  function textOf(id) { var n = document.getElementById(id); return n ? n.textContent : ''; }
  function labels(el) {
    var out = [];
    if (el.labels) for (var i = 0; i < el.labels.length; i++) out.push(cut(el.labels[i].textContent, 200));
    if (el.getAttribute('aria-label')) out.push(cut(el.getAttribute('aria-label'), 200));
    var by = el.getAttribute('aria-labelledby');
    if (by) out.push(cut(by.split(/\s+/).map(textOf).join(' '), 200));
    if (el.getAttribute('placeholder')) out.push(cut(el.getAttribute('placeholder'), 200));
    return out.filter(Boolean).slice(0, 6);
  }
  function facts(el) {
    if (String(el.type).toLowerCase() === 'password') everPassword.add(el);
    return {
      type: cut(el.getAttribute && el.getAttribute('type'), 40),
      ever_password: everPassword.has(el),
      autocomplete: cut(el.getAttribute && el.getAttribute('autocomplete'), 200),
      name: cut(el.getAttribute && el.getAttribute('name'), 200),
      id: cut(el.id, 200),
      labels: labels(el),
      script_filled: scripted.has(el)
    };
  }
  // The same rule as the server's mask.rs, so a masked value never leaves the page.
  function mask(f) {
    var tokens = f.autocomplete.toLowerCase().split(/\s+/);
    var hay = [f.name, f.id].concat(f.labels).map(function (s) { return s.toLowerCase(); });
    function word(kind) { return WORDS.some(function (w) { return w[1] === kind && hay.some(function (h) { return h.indexOf(w[0]) >= 0; }); }); }
    if (f.type.toLowerCase() === 'password' || f.ever_password || tokens.indexOf('current-password') >= 0 || tokens.indexOf('new-password') >= 0 || word('password')) return 'password';
    if (tokens.indexOf('one-time-code') >= 0 || word('code')) return 'code';
    if (tokens.some(function (t) { return t.indexOf('cc-') === 0; }) || word('payment')) return 'payment';
    if (word('secret')) return 'secret';
    if (f.script_filled) return 'saved_login';
    return null;
  }
  function valueOf(el) { return el.isContentEditable && el.tagName !== 'INPUT' && el.tagName !== 'TEXTAREA' ? el.innerText : el.value; }
  function label(f) { return f.labels[0] || f.name || f.id || ''; }

  // the `type` step for a field the person edited since its last commit, or null
  function take(el) {
    if (!dirty.has(el)) return null;
    dirty.delete(el);
    var value = String(valueOf(el) || '');
    // a script changed the value after the person's last keystroke: the vault, or the page
    if (lastTrusted.has(el) && lastTrusted.get(el) !== value) scripted.add(el);
    if (committed.get(el) === value) return null;
    committed.set(el, value);
    var f = facts(el), m = mask(f);
    var step = { kind: 'type', field: f, label: label(f), masked: m };
    if (!m) step.value = value.slice(0, 600);
    return step;
  }
  function commit(el) { var step = take(el); if (step) post(step); }
  function commitAll() { Array.from(dirty).forEach(commit); }
  // The recording is stopping (api.md §3): kwnt takes the edits still pending (the focused
  // field's, typed but not committed yet) as their payloads, from this world only.
  Object.defineProperty(globalThis, '__kwntRecorderFlush', { value: function () {
    var out = [];
    Array.from(dirty).forEach(function (el) {
      var step = take(el);
      if (step) { step.url = location.href; step.trusted = true; out.push(JSON.stringify(step)); }
    });
    return out;
  } });

  // a show-password toggle keeps the field masked for good
  new MutationObserver(function (list) {
    list.forEach(function (r) { if (String(r.oldValue).toLowerCase() === 'password') everPassword.add(r.target); });
  }).observe(document, { subtree: true, attributes: true, attributeFilter: ['type'], attributeOldValue: true });

  // Before each edit of the person's: the field must hold exactly what their own edits left,
  // empty before the first. Anything else (a value a script set with no event, a prefilled
  // value, the vault) means some of the text isn't theirs, so the field is masked (S-F2).
  document.addEventListener('beforeinput', function (e) {
    var el = e.composedPath ? e.composedPath()[0] : e.target;
    if (!e.isTrusted || !isTextField(el)) return;
    var now = String(valueOf(el) || '');
    var theirs = lastTrusted.has(el) ? lastTrusted.get(el) : '';
    if (now !== theirs) scripted.add(el);
  }, true);

  document.addEventListener('input', function (e) {
    var el = e.composedPath ? e.composedPath()[0] : e.target;
    if (!isTextField(el)) return;
    if (String(el.type).toLowerCase() === 'password') everPassword.add(el);
    if (!e.isTrusted) { scripted.add(el); return; }
    dirty.add(el);
    lastTrusted.set(el, String(valueOf(el) || ''));
    // again after the page's own handlers (an input mask reformatting is the page, not a script fill)
    setTimeout(function () { lastTrusted.set(el, String(valueOf(el) || '')); }, 0);
  }, true);

  document.addEventListener('change', function (e) {
    var el = e.target;
    if (!e.isTrusted) { if (isTextField(el)) scripted.add(el); return; }
    if (el.tagName === 'SELECT') {
      var f = facts(el), m = mask(f), opt = el.selectedOptions && el.selectedOptions[0];
      var step = { kind: 'select', field: f, label: label(f), masked: m };
      if (!m) step.value = cut(opt ? opt.label || opt.textContent : '', 600);
      post(step);
    } else if (el.tagName === 'INPUT' && String(el.type).toLowerCase() === 'file') {
      var ff = facts(el);
      post({ kind: 'upload', field: ff, label: label(ff) });
    } else if (isTextField(el)) {
      commit(el);
    }
  }, true);

  document.addEventListener('focusout', function (e) { if (e.isTrusted && dirty.has(e.target)) commit(e.target); }, true);

  document.addEventListener('keydown', function (e) {
    if (!e.isTrusted || ['Enter', 'Tab', 'Escape'].indexOf(e.key) < 0) return;
    commitAll();
    post({ kind: 'key', value: e.key });
  }, true);

  function role(el) {
    var r = el.getAttribute('role');
    if (r) return cut(r, 40);
    var t = el.tagName;
    if (t === 'A') return 'link';
    if (t === 'BUTTON' || t === 'SUMMARY') return 'button';
    if (t === 'INPUT') {
      var ty = String(el.type).toLowerCase();
      if (ty === 'checkbox' || ty === 'radio') return ty;
      return 'button';
    }
    if (t === 'OPTION') return 'option';
    return 'generic';
  }
  function accessibleName(el) {
    var by = el.getAttribute('aria-labelledby');
    var n = el.getAttribute('aria-label') || (by ? by.split(/\s+/).map(textOf).join(' ') : '');
    if (!n && el.labels && el.labels.length) n = el.labels[0].textContent;
    if (!n && el.tagName === 'INPUT') n = el.value;
    if (!n) n = el.innerText || el.textContent || '';
    if (!n) { var img = el.querySelector && el.querySelector('img[alt]'); n = img ? img.alt : ''; }
    if (!n) n = el.getAttribute('title') || '';
    return cut(n, 200);
  }
  document.addEventListener('click', function (e) {
    if (!e.isTrusted) return;
    var t = e.composedPath ? e.composedPath()[0] : e.target;
    if (!t || t.nodeType !== 1) t = t && t.parentElement;
    if (!t || isTextField(t) || t.tagName === 'SELECT') return;
    var el = t.closest('a,button,summary,option,[role],input,label') || t;
    if (isTextField(el) || el.tagName === 'SELECT' || (el.tagName === 'INPUT' && String(el.type).toLowerCase() === 'file')) return;
    // a label clicks its control too: that click is the step
    if (el.tagName === 'LABEL' && el.control) return;
    commitAll();
    post({ kind: 'click', role: role(el), name: accessibleName(el) });
  }, true);

  window.addEventListener('pagehide', function () { commitAll(); }, true);
})();
