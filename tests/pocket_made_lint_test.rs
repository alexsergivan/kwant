// pa12 K2: the lint for bot-written pocket app screens (src/pocket/made/lint.rs). One passing
// app per surface (style-guide.md section 8 patterns, the M1 worked example), one refusing case
// per style-guide.md section 7 rule ID plus this file's own cheap additions (C8, C9, J6), and
// Rigel's K2-owned cases from rigel-cases.md (WK4, NE3, NE4, PH1-PH4, DO1-DO3).

use kwnt::pocket::made::lint::{lint, Finding, Report, Surface};

fn refusal<'a>(report: &'a Report, rule: &str) -> Option<&'a Finding> {
    report.refusals.iter().find(|f| f.rule == rule)
}

fn warning<'a>(report: &'a Report, rule: &str) -> Option<&'a Finding> {
    report.warnings.iter().find(|f| f.rule == rule)
}

fn assert_refuses(report: &Report, rule: &str, sentence: &str) {
    let found = refusal(report, rule).unwrap_or_else(|| panic!("expected a {rule} refusal, got {:?}", report));
    assert_eq!(found.sentence, sentence, "rule {rule} sentence");
}

// -------------------------------------------------------------------------------------------
// the shared stylesheet both passing fixtures use (style-guide.md section 9's worked example,
// pa12/M1/design/shelf.css): colours only through pocket.css tokens, no url(), no fonts, no
// position: fixed, motion gated behind prefers-reduced-motion, a 160ms duration.
// -------------------------------------------------------------------------------------------
const GOOD_CSS: &str = r#"
.shelf-card { gap: 12px; }
.shelf { display: flex; align-items: flex-end; gap: 6px; height: 150px; padding: 0 6px; border-bottom: 6px solid var(--zinc-700); }
.book { flex: 1; min-width: 0; height: 100%; display: flex; flex-direction: column; justify-content: flex-end; align-items: stretch; gap: 4px; }
.book .n { text-align: center; font-size: 13px; line-height: 16px; font-weight: 700; color: var(--zinc-400); }
.book.lead .n { color: var(--zinc-50); }
.spine { border-radius: 4px 4px 0 0; display: flex; justify-content: center; align-items: flex-start; padding: 6px 0; overflow: hidden;
  color: var(--kw-surface-rail); box-shadow: inset -3px 0 0 color-mix(in srgb, var(--kw-surface-rail) 22%, transparent), inset 0 3px 0 var(--kw-hairline-strong); }
.spine span { writing-mode: vertical-rl; transform: rotate(180deg); font-size: 11px; line-height: 13px; font-weight: 700; }
.book.mine .spine { outline: 2px solid var(--sky-200); outline-offset: 2px; }
.v0 { height: 40%; } .v10 { height: 100%; }
.you-voted { color: var(--sky-200); font-weight: 600; }
[data-surface="screen"] .stage { display: grid; grid-template-columns: 1.55fr 1fr; gap: 1.6vw; align-items: start; }
[data-surface="screen"] .shelf { height: 30vw; gap: 0.8vw; padding: 0 1vw; border-bottom-width: 0.8vw; }
[data-surface="screen"] .leadtitle { font-size: 2.9vw; line-height: 1.1; font-weight: 800; }

@media (prefers-reduced-motion: no-preference) {
  .spine { transition: transform 160ms var(--kw-ease); }
  .book.mine .spine { transform: translateY(-2px); }
}
"#;

const GOOD_PHONE_HTML: &str = r#"
<section class="mb" aria-labelledby="h1">
  <h3 id="h1">Pick our next read</h3>
  <p class="lead">Add a book you'd swap, then vote.</p>
</section>
<section class="mb shelf-card" aria-labelledby="sh">
  <h3 id="sh" class="mline"><span class="sp">The shelf</span><span id="tot" class="mtag plain"></span></h3>
  <div id="shelf" class="shelf" role="img"></div>
  <p id="empty" class="mempty" hidden>No books yet. Be the first to add one.</p>
</section>
<section class="mb" aria-labelledby="vh">
  <h3 id="vh">Vote for the next read</h3>
  <ul id="board" class="mboard"></ul>
  <p id="hint" class="small"></p>
</section>
<section class="mb" aria-labelledby="ah">
  <h3 id="ah">Add a book</h3>
  <label class="mfield"><span class="lbl">Title</span><input id="title" class="input" maxlength="80" autocomplete="off" aria-describedby="title-err"></label>
  <p id="title-err" class="err" aria-live="polite"></p>
  <button id="add" class="gbtn" type="button">Put it on the shelf</button>
  <p id="left" class="small"></p>
</section>
"#;

const GOOD_PHONE_JS: &str = r#"
(function () {
  function h(tag, cls, text) {
    var el = document.createElement(tag);
    if (cls) el.className = cls;
    if (text != null) el.textContent = String(text);
    return el;
  }
  kwant.onState(function (s) { draw(s); });

  function draw(s) {
    var shelf = s.view.collections.shelf, picks = s.view.collections.picks;
    var books = shelf.rows || [], tally = picks.tally || {};
    var el = document.getElementById('shelf');
    el.replaceChildren();
    books.forEach(function (b) { el.appendChild(h('div', 'book', b.values.title)); });
    document.getElementById('empty').hidden = books.length > 0;

    var board = document.getElementById('board');
    board.replaceChildren();
    books.forEach(function (b) {
      var li = h('li', 'mopt-row');
      var btn = h('button', 'mvote', b.values.title + ' (' + (tally[b.id] || 0) + ')');
      btn.type = 'button';
      btn.addEventListener('click', function () { kwant.act('vote', { collection: 'picks', value: b.id }); });
      li.appendChild(btn);
      board.appendChild(li);
    });
    document.getElementById('hint').textContent = 'Tap a book to vote for it.';
  }

  var add = document.getElementById('add');
  add.addEventListener('click', function () {
    var title = document.getElementById('title').value.trim();
    if (!title) { document.getElementById('title-err').textContent = 'Type a title first.'; return; }
    document.getElementById('title-err').textContent = '';
    kwant.act('put', { collection: 'shelf', values: { title: title } });
    document.getElementById('title').value = '';
  });
})();
"#;

const GOOD_SCREEN_HTML: &str = r#"
<div class="stage">
  <section class="mb shelf-card" aria-labelledby="sh">
    <h3 id="sh" class="mline"><span class="sp">The shelf: the tallest book wins</span><span id="tot" class="mtag plain"></span></h3>
    <div id="shelf" class="shelf" role="img"></div>
    <p id="empty" class="mempty" hidden>No books yet. Add one on your phone.</p>
  </section>
  <div class="mblocks">
    <section class="mb" aria-labelledby="lh">
      <p id="lh" class="mlbl">Leading</p>
      <p id="leadtitle" class="leadtitle"></p>
      <div class="mtiles">
        <div class="mtile"><b id="t-votes"></b><small>votes for it</small></div>
        <div class="mtile"><b id="t-books"></b><small>books on the shelf</small></div>
      </div>
    </section>
  </div>
</div>
"#;

const GOOD_SCREEN_JS: &str = r#"
(function () {
  function h(tag, cls, text) {
    var el = document.createElement(tag);
    if (cls) el.className = cls;
    if (text != null) el.textContent = String(text);
    return el;
  }
  kwant.onState(function (s) {
    var shelf = s.view.collections.shelf;
    var books = shelf.rows || [], tally = shelf.tally || {};
    var el = document.getElementById('shelf');
    el.replaceChildren();
    var most = 0, lead = null;
    books.forEach(function (b) {
      var n = tally[b.id] || 0;
      if (n > most) { most = n; lead = b; }
      el.appendChild(h('div', 'book', b.values.title));
    });
    document.getElementById('empty').hidden = books.length > 0;
    document.getElementById('leadtitle').textContent = lead ? lead.values.title : 'No votes yet';
    document.getElementById('t-votes').textContent = String(most);
    document.getElementById('t-books').textContent = String(books.length);
  });
})();
"#;

#[test]
fn good_phone_app_passes_clean() {
    let r = lint(Surface::Phone, GOOD_PHONE_HTML, GOOD_CSS, GOOD_PHONE_JS);
    assert!(r.refusals.is_empty(), "unexpected refusals: {:?}", r.refusals);
    assert!(r.warnings.is_empty(), "unexpected warnings: {:?}", r.warnings);
}

#[test]
fn good_screen_app_passes_clean() {
    let r = lint(Surface::Screen, GOOD_SCREEN_HTML, GOOD_CSS, GOOD_SCREEN_JS);
    assert!(r.refusals.is_empty(), "unexpected refusals: {:?}", r.refusals);
    assert!(r.warnings.is_empty(), "unexpected warnings: {:?}", r.warnings);
}

// -------------------------------------------------------------------------------------------
// one refusing case per style-guide.md section 7 rule, plus this file's own C8/C9/J6.
// -------------------------------------------------------------------------------------------

#[test]
fn m1_refuses_an_element_outside_the_allowlist() {
    let r = lint(Surface::Phone, "<form><p>hi</p></form>", "", "");
    assert_refuses(&r, "M1", "It used a <form>, and pocket apps can't use that.");
}

#[test]
fn m2_refuses_an_attribute_outside_the_allowlist() {
    let r = lint(Surface::Phone, r#"<div onclick="go()">hi</div>"#, "", "");
    assert_refuses(&r, "M2", "It put onclick on a <div>; style and events go in the CSS and the script.");
}

#[test]
fn m3_refuses_an_input_type_outside_the_allowlist() {
    let r = lint(Surface::Phone, r#"<input type="password">"#, "", "");
    assert_refuses(&r, "M3", "It asked for a password field, and pocket apps can't.");
}

#[test]
fn m4_refuses_autocomplete_other_than_off() {
    let r = lint(Surface::Phone, r#"<input autocomplete="name">"#, "", "");
    assert_refuses(&r, "M4", "It asked the phone to fill in saved details.");
}

#[test]
fn m5_refuses_a_password_style_ask() {
    let r = lint(Surface::Phone, r#"<label>Enter your password<input id="p"></label>"#, "", "");
    assert_refuses(&r, "M5", "It asked guests for their password. Pocket apps never ask for that.");
}

#[test]
fn m5_warns_on_an_email_style_ask() {
    let r = lint(Surface::Phone, r#"<label>Your email<input id="e"></label>"#, "", "");
    let f = warning(&r, "M5").expect("expected an M5 warning");
    assert_eq!(f.sentence, "It asked guests for their email. Pocket apps don't need that.");
}

#[test]
fn m6_refuses_a_src_off_kwant() {
    let r = lint(Surface::Phone, r#"<img src="https://evil.test/x.png" alt="">"#, "", "");
    assert_refuses(&r, "M6", "It tried to load something from the web.");
}

#[test]
fn m6_allows_the_svg_namespace_inside_an_allowed_data_image() {
    let svg = "data:image/svg+xml;utf8,<svg xmlns='http://www.w3.org/2000/svg'></svg>";
    let r = lint(Surface::Phone, &format!(r#"<img src="{svg}" alt="">"#), "", "");
    assert!(refusal(&r, "M6").is_none(), "the SVG namespace exception should not refuse: {:?}", r.refusals);
}

#[test]
fn m7_refuses_kwants_own_class_prefix() {
    let r = lint(Surface::Phone, r#"<div class="kw-toast">hi</div>"#, "", "");
    assert_refuses(&r, "M7", "It used kwant's own names; those are kwant's alone.");
}

#[test]
fn c1_refuses_an_import() {
    let r = lint(Surface::Phone, "", "@import url('x.css');", "");
    assert_refuses(&r, "C1", "Its CSS tried to load a file or a font.");
}

#[test]
fn c1_refuses_a_url_that_is_not_a_data_image() {
    let r = lint(Surface::Phone, "", ".x { background: url(//evil.test/x.png); }", "");
    assert_refuses(&r, "C1", "Its CSS tried to load a file or a font.");
}

#[test]
fn c2_refuses_position_fixed() {
    let r = lint(Surface::Phone, "", ".x { position: fixed; }", "");
    assert_refuses(&r, "C2", "It tried to pin something over the page.");
}

#[test]
fn c3_refuses_a_hex_colour() {
    let r = lint(Surface::Phone, "", ".x { color: #ff0000; }", "");
    assert_refuses(&r, "C3", "It used its own colour; use kwant's colour names.");
}

#[test]
fn c4_refuses_a_changed_font() {
    let r = lint(Surface::Phone, "", ".x { font-family: Arial, sans-serif; }", "");
    assert_refuses(&r, "C4", "It changed the font.");
}

#[test]
fn c5_refuses_motion_outside_the_reduced_motion_guard() {
    let r = lint(Surface::Phone, "", ".x { transition: transform 160ms linear; }", "");
    assert_refuses(&r, "C5", "It moves without checking 'reduce motion'.");
}

#[test]
fn c5_refuses_a_duration_outside_120_to_200ms_even_inside_the_guard() {
    let css = "@media (prefers-reduced-motion: no-preference) { .x { transition: transform 900ms linear; } }";
    let r = lint(Surface::Phone, "", css, "");
    assert_refuses(&r, "C5", "It moves without checking 'reduce motion'.");
}

#[test]
fn c6_refuses_a_surface_over_120kb() {
    let big_css = format!(".x {{ color: inherit; }} /* {} */", "a".repeat(130 * 1024));
    let r = lint(Surface::Phone, "", &big_css, "");
    assert_refuses(&r, "C6", "It's too big for a phone.");
}

#[test]
fn c7_refuses_a_z_index_above_10() {
    let r = lint(Surface::Phone, "", ".x { z-index: 11; }", "");
    assert_refuses(&r, "C7", "It tried to cover part of the page.");
}

#[test]
fn c8_refuses_webkit_text_security() {
    let r = lint(Surface::Phone, "", ".x { -webkit-text-security: disc; }", "");
    assert_refuses(&r, "C8", "Its CSS hid typed letters like a password box.");
}

#[test]
fn c9_refuses_a_style_tag_breakout() {
    let r = lint(Surface::Phone, "", "</style><input type=password>", "");
    assert_refuses(&r, "C9", "Its CSS tried to close the style tag.");
}

#[test]
fn j1_refuses_a_whole_word_token_but_allows_a_longer_identifier() {
    let r = lint(Surface::Phone, "", "", "var top = 1;");
    assert_refuses(&r, "J1", "Its script used top, which pocket apps can't.");
    let r2 = lint(Surface::Phone, "", "", "var topping = 1;");
    assert!(refusal(&r2, "J1").is_none(), "topping should not be refused as a substring of top: {:?}", r2.refusals);
}

#[test]
fn j2_refuses_an_old_style_event_assignment() {
    let r = lint(Surface::Phone, "", "", "el.onclick = function () {};");
    assert_refuses(&r, "J2", "It wired an event the old way.");
}

#[test]
fn j3_refuses_an_act_name_outside_the_allowlist() {
    let r = lint(Surface::Phone, "", "", "kwant.act('delete_everything', {});");
    assert_refuses(&r, "J3", "It tried an action guests can't do.");
}

#[test]
fn j3_refuses_the_screen_document_calling_act() {
    let r = lint(Surface::Screen, "", "", "kwant.act('vote', { collection: 'picks', value: 1 });");
    assert_refuses(&r, "J3", "The big screen tried to change something.");
}

#[test]
fn j4_refuses_an_image_src_literal_that_is_not_an_allowed_data_image() {
    let r = lint(Surface::Phone, "", "", "img.src = 'https://evil.test/x.png';");
    assert_refuses(&r, "J4", "It put guests' words in as code.");
}

#[test]
fn j5_refuses_a_second_script_element() {
    let r = lint(Surface::Phone, "", "", "var x = '<script src=\"evil.js\">';");
    assert_refuses(&r, "J5", "It had more than one script.");
}

#[test]
fn j6_refuses_a_script_tag_breakout() {
    let r = lint(Surface::Phone, "", "", "var x = 'y'; </script><input type=password>");
    assert_refuses(&r, "J6", "Its script tried to close the script tag.");
}

// -------------------------------------------------------------------------------------------
// Rigel's K2-owned cases (rigel-cases.md section 1 WK4; section 3 NE3/NE4; section 4
// PH1-PH4; section 5 DO1-DO3).
// -------------------------------------------------------------------------------------------

#[test]
fn wk4_custom_scheme_link_is_refused_because_a_is_not_an_allowed_element() {
    // rigel-cases.md WK4: "the lint must refuse every scheme-ful href (not only absolute
    // http)". M1 refuses the whole <a> element, which is stronger: no href scheme ever reaches
    // the page at all.
    let r = lint(Surface::Phone, r#"<a href="tel:+15551234567">Call</a>"#, "", "");
    assert_refuses(&r, "M1", "It used a <a>, and pocket apps can't use that.");
}

#[test]
fn ne3_refuses_formaction_without_a_form_element() {
    let r = lint(Surface::Phone, r#"<button formaction="https://evil.test/x">Go</button>"#, "", "");
    assert_refuses(&r, "M2", "It put formaction on a <button>; style and events go in the CSS and the script.");
}

#[test]
fn ne3_refuses_an_area_element_outright() {
    let r = lint(Surface::Phone, r#"<area ping="https://evil.test/x">"#, "", "");
    assert_refuses(&r, "M1", "It used a <area>, and pocket apps can't use that.");
}

#[test]
fn ne4_refuses_css_exfil_through_font_face() {
    let r = lint(Surface::Phone, "", "@font-face { font-family: X; src: url(//evil.test/f.woff2); }", "");
    assert_refuses(&r, "C1", "Its CSS tried to load a file or a font.");
}

#[test]
fn ne4_refuses_css_exfil_through_a_background_url() {
    let r = lint(Surface::Phone, "", ".x { background: url(https://evil.test/x.png); }", "");
    assert_refuses(&r, "C1", "Its CSS tried to load a file or a font.");
}

#[test]
fn ph1_refuses_autofill_tokens_beyond_off() {
    for token in ["username", "email", "tel", "street-address", "name", "cc-number", "current-password", "new-password", "one-time-code"] {
        let r = lint(Surface::Phone, &format!(r#"<input autocomplete="{token}">"#), "", "");
        assert_refuses(&r, "M4", "It asked the phone to fill in saved details.");
    }
}

#[test]
fn ph2_refuses_a_disguised_password_field() {
    let r = lint(Surface::Phone, "", ".x { -webkit-text-security: disc; }", "");
    assert_refuses(&r, "C8", "Its CSS hid typed letters like a password box.");
    let r2 = lint(Surface::Phone, r#"<div contenteditable="true"></div>"#, "", "");
    assert_refuses(&r2, "M2", "It let guests type into a <div>; use an input.");
    let r3 = lint(Surface::Phone, r#"<input type="password">"#, "", "");
    assert_refuses(&r3, "M3", "It asked for a password field, and pocket apps can't.");
}

#[test]
fn ph3_warns_on_a_plain_password_label_but_a_leetspeak_label_is_a_known_gap() {
    // the word list is a warning-vs-refusal boundary only in the sense that it's text matching:
    // documented in impl.md as a known, not-chased gap (style-guide.md section 7's own words
    // list isn't homoglyph/leetspeak aware either).
    let r = lint(Surface::Phone, r#"<label>password<input id="p"></label>"#, "", "");
    assert_refuses(&r, "M5", "It asked guests for their password. Pocket apps never ask for that.");
    let r2 = lint(Surface::Phone, r#"<label>p4ssw0rd<input id="p"></label>"#, "", "");
    assert!(refusal(&r2, "M5").is_none(), "leetspeak bypass is a documented, not-chased gap");
}

#[test]
fn ph4_refuses_a_file_input() {
    let r = lint(Surface::Phone, r#"<input type="file" accept="image/*">"#, "", "");
    assert_refuses(&r, "M3", "It asked for a file field, and pocket apps can't.");
}

#[test]
fn ph4_refuses_a_color_input() {
    let r = lint(Surface::Phone, r#"<input type="color">"#, "", "");
    assert_refuses(&r, "M3", "It asked for a color field, and pocket apps can't.");
}

#[test]
fn do1_refuses_script_breakout_case_insensitively() {
    for payload in ["</script><form><input type=password>", "</SCRIPT ><form>", "</script/x>"] {
        let r = lint(Surface::Phone, "", "", payload);
        assert_refuses(&r, "J6", "Its script tried to close the script tag.");
    }
}

#[test]
fn do1_refuses_a_style_breakout_in_css() {
    let r = lint(Surface::Phone, "", "</style><input type=password>", "");
    assert_refuses(&r, "C9", "Its CSS tried to close the style tag.");
}

#[test]
fn do1_refuses_body_and_html_as_unknown_elements() {
    let r = lint(Surface::Phone, "<p>hi</p></body></html>", "", "");
    assert_refuses(&r, "M1", "It used a <body>, and pocket apps can't use that.");
    let r2 = refusal(&r, "M1");
    assert!(r2.is_some());
}

#[test]
fn do2_refuses_exactly_one_byte_over_the_size_budget() {
    let html = "a".repeat(122_880);
    let r_at_budget = lint(Surface::Phone, &html, "", "");
    assert!(refusal(&r_at_budget, "C6").is_none(), "exactly 122,880 bytes must still pass");
    let html_over = "a".repeat(122_881);
    let r_over = lint(Surface::Phone, &html_over, "", "");
    assert_refuses(&r_over, "C6", "It's too big for a phone.");
}

#[test]
fn do3_refuses_uppercase_and_malformed_script_tag_spellings() {
    let r = lint(Surface::Phone, "<SCRIPT>evil()</SCRIPT>", "", "");
    assert_refuses(&r, "M1", "It used a <script>, and pocket apps can't use that.");
}

#[test]
fn do3_refuses_a_password_type_even_when_a_duplicate_text_type_is_present_first() {
    // browsers keep the first `type`; this lint checks every occurrence, so the hidden second
    // `type=password` is still refused even though a naive "last wins" reading would miss it.
    let r = lint(Surface::Phone, r#"<input type="text" type="password">"#, "", "");
    assert_refuses(&r, "M3", "It asked for a password field, and pocket apps can't.");
}

#[test]
fn do3_refuses_a_password_input_nested_inside_a_template() {
    let r = lint(Surface::Phone, "<template><input type=password></template>", "", "");
    assert_refuses(&r, "M3", "It asked for a password field, and pocket apps can't.");
}

// -------------------------------------------------------------------------------------------
// copy rules (warnings only, style-guide.md section 7's "Copy rules")
// -------------------------------------------------------------------------------------------

#[test]
fn copy_warns_on_a_banned_button_word() {
    let r = lint(Surface::Phone, r#"<button type="button">Submit</button>"#, "", "");
    assert!(warning(&r, "copy").is_some(), "expected a copy warning: {:?}", r.warnings);
    assert!(refusal(&r, "copy").is_none(), "copy rules are warnings only, never refusals");
}

#[test]
fn copy_warns_on_a_kwant_only_claim() {
    let r = lint(Surface::Phone, "<p>Verified by kwant</p>", "", "");
    assert!(warning(&r, "copy").is_some(), "expected a copy warning: {:?}", r.warnings);
}

// -------------------------------------------------------------------------------------------
// Lin's backend review of #1407 (lin-review.md): 1a (M5 scoping), 1c (findings cap and the
// C6 early return) and every row of section 2 that got fixed.
// -------------------------------------------------------------------------------------------

#[test]
fn m5_1a_scopes_to_label_legend_button_not_the_whole_page() {
    // lin-review.md 1a: an M1 heading like "Pin your favourite" must not trip M5.
    let r = lint(Surface::Phone, "<h2>Pin your favourite</h2>", "", "");
    assert!(refusal(&r, "M5").is_none(), "a heading outside label/legend/button must not trip M5: {:?}", r.refusals);

    let r2 = lint(Surface::Phone, r#"<label>PIN<input id="p"></label>"#, "", "");
    assert_refuses(&r2, "M5", "It asked guests for their PIN. Pocket apps never ask for that.");
}

#[test]
fn c6_1c_oversized_input_returns_only_c6() {
    let big_css = format!(".x {{ color: inherit; }} /* {} */", "a".repeat(130 * 1024));
    let html = r#"<div onclick="go()">hi</div>"#;
    let r = lint(Surface::Phone, html, &big_css, "var top = 1;");
    assert_eq!(r.refusals.len(), 1, "an oversized input must return only C6: {:?}", r.refusals);
    assert_refuses(&r, "C6", "It's too big for a phone.");
}

#[test]
fn c6_1c_fifteen_thousand_distinct_attrs_caps_findings_at_20() {
    let attrs: Vec<String> = (0..15_000).map(|i| format!("a{i}")).collect();
    let html = format!("<b {}>", attrs.join(" "));
    let r = lint(Surface::Phone, &html, "", "");
    assert!(r.refusals.len() <= 20, "expected at most 20 findings, got {}", r.refusals.len());
}

#[test]
fn c2_refuses_position_fixed_with_important() {
    let r = lint(Surface::Phone, "", ".x { position: fixed !important; }", "");
    assert_refuses(&r, "C2", "It tried to pin something over the page.");
}

#[test]
fn c7_refuses_a_z_index_above_10_with_important() {
    let r = lint(Surface::Phone, "", ".x { z-index: 99 !important; }", "");
    assert_refuses(&r, "C7", "It tried to cover part of the page.");
}

#[test]
fn c5_refuses_animation_name_and_duration_outside_the_guard() {
    let r = lint(Surface::Phone, "", ".x { animation-name: spin; animation-duration: 5s; }", "");
    assert_refuses(&r, "C5", "It moves without checking 'reduce motion'.");
}

#[test]
fn c5_refuses_transition_property_and_duration_outside_the_guard() {
    let r = lint(Surface::Phone, "", ".x { transition-property: color; transition-duration: 3s; }", "");
    assert_refuses(&r, "C5", "It moves without checking 'reduce motion'.");
}

#[test]
fn c1_and_m6_refuse_an_off_kwant_url_inside_a_css_string_literal() {
    let css = r#".x { background-image: image-set("https://evil.test/x.png" 1x); }"#;
    let r = lint(Surface::Phone, "", css, "");
    assert_refuses(&r, "C1", "Its CSS tried to load a file or a font.");
    assert_refuses(&r, "M6", "It tried to load something from the web.");
}

#[test]
fn m6_refuses_a_relative_src_path() {
    let r = lint(Surface::Phone, r#"<img src="/api/pockets" alt="">"#, "", "");
    assert_refuses(&r, "M6", "It tried to load something from the web.");
}

#[test]
fn m6_refuses_a_backslash_disguised_host_in_src() {
    let r = lint(Surface::Phone, r#"<img src="/\evil.test/x.png" alt="">"#, "", "");
    assert_refuses(&r, "M6", "It tried to load something from the web.");
}

#[test]
fn j1_refuses_fetch_with_a_space_before_the_parenthesis() {
    let r = lint(Surface::Phone, "", "", "fetch ('/x');");
    assert_refuses(&r, "J1", "Its script used fetch(, which pocket apps can't.");
}

#[test]
fn j1_refuses_a_unicode_escape_used_to_hide_an_identifier_outside_a_string() {
    // JS lets a bare identifier spell a letter through a unicode escape (this spells "top"),
    // which would otherwise dodge the word-matched J1 check above without that literal token
    // ever appearing in the source (lin-review.md section 2).
    let hidden_top = format!("t{}u006fp", '\\');
    let js = format!("var x = {hidden_top};");
    let r = lint(Surface::Phone, "", "", &js);
    assert_refuses(&r, "J1", "Its script hid a name behind a \\u escape, which pocket apps can't.");
}

#[test]
fn j1_allows_a_unicode_escape_inside_a_string_literal() {
    let escaped = format!("var x = '{}u00e9';", '\\');
    let r = lint(Surface::Phone, "", "", &escaped);
    assert!(refusal(&r, "J1").is_none(), "a unicode escape inside a string literal is not the J1 bypass: {:?}", r.refusals);
}

#[test]
fn j2_bracket_property_event_assignment_is_a_known_gap() {
    // `el['onclick'] = go` bypasses the literal `.onclick =` check (lin-review.md section 2);
    // documented in impl.md as a known, not-chased gap alongside window["top"].
    let r = lint(Surface::Phone, "", "", "el['onclick'] = go;");
    assert!(refusal(&r, "J2").is_none(), "bracket-property event assignment is a documented, not-chased gap");
}

#[test]
fn c3_refuses_a_named_color_on_border() {
    let r = lint(Surface::Phone, "", ".x { border: 1px solid red; }", "");
    assert_refuses(&r, "C3", "It used its own colour; use kwant's colour names.");
}

#[test]
fn m2_1d_caps_a_long_attribute_name_in_the_sentence() {
    let long_attr = "a".repeat(100);
    let html = format!("<div {long_attr}=\"x\">hi</div>");
    let r = lint(Surface::Phone, &html, "", "");
    let expected = format!("It put {}? on a <div>; style and events go in the CSS and the script.", "a".repeat(40));
    assert_refuses(&r, "M2", &expected);
}

#[test]
fn m2_1d_replaces_a_non_printable_byte_in_the_attribute_name_with_a_marker() {
    let html = format!("<div a{}b=\"x\">hi</div>", '\u{0007}');
    let r = lint(Surface::Phone, &html, "", "");
    assert_refuses(&r, "M2", "It put a?b on a <div>; style and events go in the CSS and the script.");
}

#[test]
fn m3_1d_caps_a_long_input_type_value_in_the_sentence() {
    let long_type = "x".repeat(100);
    let html = format!(r#"<input type="{long_type}">"#);
    let r = lint(Surface::Phone, &html, "", "");
    let expected = format!("It asked for a {}? field, and pocket apps can't.", "x".repeat(40));
    assert_refuses(&r, "M3", &expected);
}

#[test]
fn m3_1d_replaces_a_non_printable_byte_in_the_input_type_with_a_marker() {
    let html = format!("<input type=\"a{}b\">", '\u{0007}');
    let r = lint(Surface::Phone, &html, "", "");
    assert_refuses(&r, "M3", "It asked for a a?b field, and pocket apps can't.");
}

// -------------------------------------------------------------------------------------------
// Rigel's QA of #1407 (qa.md): B2 (the CSS declaration scanner read a selector's or an at-rule's
// own colon as a `name: value` pair, swallowing the real declaration that followed it), plus the
// "not blocking, but worth fixing" items this round folds in (the image-set/CSS-escape gap, the
// `.16s` duration false positive, `color-mix()` smuggling a named colour, and PH5's one-token add).
// -------------------------------------------------------------------------------------------

#[test]
fn b2_a_pseudo_class_colon_does_not_swallow_the_declaration_that_follows_it() {
    let r = lint(Surface::Phone, "", ".x:hover { position: fixed; }", "");
    assert_refuses(&r, "C2", "It tried to pin something over the page.");
}

#[test]
fn b2_a_pseudo_element_colon_does_not_swallow_the_declaration_that_follows_it() {
    let r = lint(Surface::Phone, "", ".x::before { position: fixed; content: ''; }", "");
    assert_refuses(&r, "C2", "It tried to pin something over the page.");
}

#[test]
fn b2_a_pseudo_class_colon_does_not_hide_a_bad_colour() {
    let r = lint(Surface::Phone, "", ".x:focus { color: #f00; }", "");
    assert_refuses(&r, "C3", "It used its own colour; use kwant's colour names.");
}

#[test]
fn b2_a_pseudo_class_colon_does_not_hide_a_changed_font() {
    let r = lint(Surface::Phone, "", ".x:hover { font-family: Arial; }", "");
    assert_refuses(&r, "C4", "It changed the font.");
}

#[test]
fn b2_an_at_rule_feature_query_colon_does_not_swallow_the_rule_inside() {
    let r = lint(Surface::Phone, "", "@media (min-width:1px) { .x { position: fixed; } }", "");
    assert_refuses(&r, "C2", "It tried to pin something over the page.");
}

#[test]
fn b2_the_reduced_motion_guard_condition_colon_does_not_swallow_position_fixed_inside_it() {
    let css = "@media (prefers-reduced-motion: no-preference) { .x { position: fixed; } }";
    let r = lint(Surface::Phone, "", css, "");
    assert_refuses(&r, "C2", "It tried to pin something over the page.");
}

#[test]
fn b2_the_reduced_motion_guard_condition_colon_does_not_swallow_a_bad_z_index_inside_it() {
    let css = "@media (prefers-reduced-motion: no-preference) { .x { z-index: 9999; } }";
    let r = lint(Surface::Phone, "", css, "");
    assert_refuses(&r, "C7", "It tried to cover part of the page.");
}

#[test]
fn b2_an_at_supports_condition_colon_does_not_swallow_the_rule_inside() {
    let r = lint(Surface::Phone, "", "@supports (display:grid) { .x { position: fixed; } }", "");
    assert_refuses(&r, "C2", "It tried to pin something over the page.");
}

#[test]
fn b2_still_refuses_position_fixed_with_important_now_that_declarations_are_block_scoped() {
    let r = lint(Surface::Phone, "", ".x { position: fixed !important; }", "");
    assert_refuses(&r, "C2", "It tried to pin something over the page.");
}

#[test]
fn b2_still_refuses_an_oversized_z_index_with_important_now_that_declarations_are_block_scoped() {
    let r = lint(Surface::Phone, "", ".x { z-index: 9999 !important; }", "");
    assert_refuses(&r, "C7", "It tried to cover part of the page.");
}

#[test]
fn b2_a_css_escape_of_fixed_does_not_dodge_position_fixed() {
    let escaped = format!("position: {}66ixed;", '\\');
    let css = format!(".x {{ {escaped} }}");
    let r = lint(Surface::Phone, "", &css, "");
    assert_refuses(&r, "C2", "It tried to pin something over the page.");
}

#[test]
fn b2_a_comment_hidden_inside_a_value_does_not_dodge_position_fixed() {
    let r = lint(Surface::Phone, "", ".x { position: /**/ fixed; }", "");
    assert_refuses(&r, "C2", "It tried to pin something over the page.");
}

#[test]
fn qa2_a_css_escape_of_url_does_not_dodge_an_off_kwant_load() {
    let escaped = format!("background: {}75rl(//evil.test/x.png);", '\\');
    let css = format!(".x {{ {escaped} }}");
    let r = lint(Surface::Phone, "", &css, "");
    assert_refuses(&r, "C1", "Its CSS tried to load a file or a font.");
}

#[test]
fn qa3_color_mix_still_refuses_a_named_colour_inside_it() {
    let r = lint(Surface::Phone, "", ".x { color: color-mix(in srgb, red 10%, blue); }", "");
    assert_refuses(&r, "C3", "It used its own colour; use kwant's colour names.");
}

#[test]
fn qa3_color_mix_over_kwant_tokens_still_passes() {
    let css = ".x { color: color-mix(in srgb, var(--kw-surface-rail) 22%, transparent); }";
    let r = lint(Surface::Phone, "", css, "");
    assert!(refusal(&r, "C3").is_none(), "color-mix() over kwant's own tokens is allowed");
}

#[test]
fn qa4_a_leading_dot_duration_is_read_as_the_fraction_of_a_second_it_is() {
    let css = "@media (prefers-reduced-motion: no-preference) { .x { transition: transform .16s linear; } }";
    let r = lint(Surface::Phone, "", css, "");
    assert!(refusal(&r, "C5").is_none(), ".16s is 160ms, inside the 120-200ms window");
}

#[test]
fn qa4_a_leading_dot_duration_outside_the_window_still_refuses() {
    let css = "@media (prefers-reduced-motion: no-preference) { .x { transition: transform .9s linear; } }";
    let r = lint(Surface::Phone, "", css, "");
    assert_refuses(&r, "C5", "It moves without checking 'reduce motion'.");
}

#[test]
fn qa5_refuses_execcommand() {
    let r = lint(Surface::Phone, "", "", "document.execCommand('copy');");
    assert_refuses(&r, "J1", "Its script used execCommand(, which pocket apps can't.");
}

#[test]
fn b4_an_outer_block_declaration_before_a_nested_block_still_refuses_position_fixed() {
    let r = lint(Surface::Phone, "", ".a { position: fixed; .b { margin: 0; } }", "");
    assert_refuses(&r, "C2", "It tried to pin something over the page.");
}

#[test]
fn b4_an_outer_block_declaration_after_a_nested_block_still_refuses_an_oversized_z_index() {
    let r = lint(Surface::Phone, "", ".a { .b { margin: 0; } z-index: 9999; }", "");
    assert_refuses(&r, "C7", "It tried to cover part of the page.");
}

#[test]
fn b4_an_outer_block_declaration_alongside_a_nested_ampersand_rule_still_refuses_a_bad_colour() {
    let r = lint(Surface::Phone, "", ".a { color: #f00; &:hover { text-decoration: underline; } }", "");
    assert_refuses(&r, "C3", "It used its own colour; use kwant's colour names.");
}

#[test]
fn b4_an_outer_block_declaration_still_refuses_a_changed_font_inside_a_media_query() {
    let css = "@media (min-width:1px) { .a { font-family: Arial; .b { margin: 0; } } }";
    let r = lint(Surface::Phone, "", css, "");
    assert_refuses(&r, "C4", "It changed the font.");
}

#[test]
fn b4_a_three_level_nest_still_refuses_the_outermost_blocks_own_declaration() {
    let r = lint(Surface::Phone, "", ".a { .b { .c { margin: 0; } } position: fixed; }", "");
    assert_refuses(&r, "C2", "It tried to pin something over the page.");
}

#[test]
fn b4_an_unclosed_block_still_refuses_the_declaration_a_browser_would_apply_to_end_of_input() {
    let r = lint(Surface::Phone, "", ".a { position: fixed", "");
    assert_refuses(&r, "C2", "It tried to pin something over the page.");
}

#[test]
fn b5_a_sixty_kilobyte_nest_of_unclosed_blocks_still_lints_in_well_under_a_second() {
    // Rigel's QA (B5): scanning every `{ }` body's full text once per level it's nested at (the
    // range-collecting version of `declarations()` that fixed B4) turned this exact shape
    // (well inside the 120 KB C6 budget, so nothing short-circuits it) quadratic: 60 KB of
    // unclosed "a{" opens took 62s, against 1.8ms before B4's fix. The linear single-pass
    // replacement (B5's fix) must stay fast on it.
    let css = "a{".repeat(30_000);
    let start = std::time::Instant::now();
    let _ = lint(Surface::Phone, "", &css, "");
    let elapsed = start.elapsed();
    assert!(elapsed.as_secs() < 2, "expected well under a second, took {elapsed:?}");
}

#[test]
fn b5_twenty_eight_kilobytes_of_single_declaration_blocks_still_lints_in_well_under_a_second() {
    let css = ".a{b:c;".repeat(4_000);
    let start = std::time::Instant::now();
    let _ = lint(Surface::Phone, "", &css, "");
    let elapsed = start.elapsed();
    assert!(elapsed.as_secs() < 2, "expected well under a second, took {elapsed:?}");
}

#[test]
fn b6_a_double_quoted_string_left_open_by_a_bare_newline_still_refuses_what_follows_it() {
    // Rigel's QA (B6): a CSS string ends at an unescaped newline, same as a browser (a CSS
    // string can't contain a literal newline). The scan used to keep looking for the closing
    // quote past that newline, which could run to the end of the stylesheet and hide every
    // declaration after the dropped quote from this scan.
    let r = lint(Surface::Phone, "", ".a{content:\"x\n}.b{position:fixed}", "");
    assert_refuses(&r, "C2", "It tried to pin something over the page.");
}

#[test]
fn b6_a_single_quoted_string_left_open_by_a_bare_newline_still_refuses_what_follows_it() {
    let r = lint(Surface::Phone, "", ".a{content:'x\n}.b{position:fixed}", "");
    assert_refuses(&r, "C2", "It tried to pin something over the page.");
}

#[test]
fn b6_a_string_left_open_by_a_bare_crlf_still_refuses_what_follows_it() {
    let r = lint(Surface::Phone, "", ".a{content:\"x\r\n}.b{position:fixed}", "");
    assert_refuses(&r, "C2", "It tried to pin something over the page.");
}

#[test]
fn b7_a_backslash_escaped_crlf_keeps_the_string_open_and_still_refuses_what_follows_it() {
    // Rigel's QA (B7): a backslash followed by CRLF is one escaped newline in CSS, same as a
    // browser, so the string isn't ended by it and keeps going to its real closing quote.
    // Splitting the escape into "backslash + CR" (2 bytes) left the LF to be read on its own as
    // a bare newline, ending the string early and opening a bogus new string at the next quote
    // that swallowed everything after it, including this C2 refusal.
    let r = lint(Surface::Phone, "", ".a{content:\"x\\\r\ny\"}.b{position:fixed}", "");
    assert_refuses(&r, "C2", "It tried to pin something over the page.");
}
