//! A native `<select>` opens its list outside the page, so the screencast never shows it. When the
//! person presses on one, the stream sends the options for kwant's own list instead of the press.
//! Top document only; selects inside iframes are out of scope.
//!
//! Everything read back is page-controlled text: lengths and counts are capped and the box must
//! be finite, or the press goes through as usual.

use super::browser::BrowserController;
use serde::Serialize;
use serde_json::{json, Value};

/// Most options sent for one select; a longer list is cut (the person can still type to search
/// in a later round).
pub const MAX_SELECT_OPTIONS: usize = 500;
/// Longest option label or value sent, in characters.
pub const MAX_OPTION_TEXT: usize = 200;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SelectOption {
    pub label: String,
    pub value: String,
    pub disabled: bool,
    pub selected: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SelectRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// The `select` message for the client, without its `type`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SelectPopup {
    pub options: Vec<SelectOption>,
    pub multiple: bool,
    pub rect: SelectRect,
}

impl SelectPopup {
    pub fn message(&self) -> Value {
        json!({"type": "select", "options": self.options, "multiple": self.multiple, "rect": self.rect})
    }
}

/// The page script: finds an enabled select under the point, focuses it, keeps it in
/// `window.__kwantSelect` for `select_option`, and describes it. Null when there is none.
pub fn select_probe_expression(x: f64, y: f64) -> String {
    format!(
        r#"(function() {{
            var el = document.elementFromPoint({x}, {y});
            var s = el && el.closest ? el.closest('select') : null;
            if (!s || s.disabled) return null;
            try {{ s.focus(); }} catch (e) {{}}
            window.__kwantSelect = s;
            var r = s.getBoundingClientRect();
            var options = [];
            for (var i = 0; i < s.options.length && i < {max}; i++) {{
                var o = s.options[i];
                options.push({{label: String(o.label || o.text || ''), value: String(o.value), disabled: !!(o.disabled || (o.parentElement && o.parentElement.disabled)), selected: !!o.selected}});
            }}
            return {{options: options, multiple: !!s.multiple, rect: {{x: r.left, y: r.top, width: r.width, height: r.height}}}};
        }})()"#,
        x = x, y = y, max = MAX_SELECT_OPTIONS,
    )
}

fn capped(value: &Value) -> String {
    value.as_str().unwrap_or_default().trim().chars().take(MAX_OPTION_TEXT).collect()
}

/// Validates what the probe returned. `None` for null, a malformed reply or a box that isn't
/// finite, so the caller dispatches the native press instead.
pub fn parse_select_probe(value: &Value) -> Option<SelectPopup> {
    let options = value["options"].as_array()?
        .iter()
        .take(MAX_SELECT_OPTIONS)
        .map(|o| SelectOption {
            label: capped(&o["label"]),
            value: capped(&o["value"]),
            disabled: o["disabled"].as_bool().unwrap_or(false),
            selected: o["selected"].as_bool().unwrap_or(false),
        })
        .collect();
    let r = &value["rect"];
    let n = |k: &str| r[k].as_f64().filter(|v| v.is_finite());
    let rect = SelectRect { x: n("x")?, y: n("y")?, width: n("width")?.max(0.0), height: n("height")?.max(0.0) };
    Some(SelectPopup { options, multiple: value["multiple"].as_bool().unwrap_or(false), rect })
}

impl BrowserController {
    /// The select under a press at (x, y), if any. A failed probe counts as none, so input
    /// never stalls on it.
    pub async fn select_at(&self, x: f64, y: f64) -> Option<SelectPopup> {
        if !x.is_finite() || !y.is_finite() { return None; }
        let res = self
            .send_cdp_command("Runtime.evaluate", json!({"expression": select_probe_expression(x, y), "returnByValue": true}))
            .await
            .ok()?;
        parse_select_probe(&res["result"]["value"])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_probe_reply_becomes_the_select_message() {
        let reply = json!({
            "options": [
                {"label": " Germany ", "value": "de", "disabled": false, "selected": true},
                {"label": "France", "value": "fr", "disabled": true, "selected": false},
            ],
            "multiple": false,
            "rect": {"x": 10.5, "y": 20.0, "width": 200.0, "height": 32.0},
        });
        let popup = parse_select_probe(&reply).unwrap();
        assert_eq!(popup.options.len(), 2);
        assert_eq!(popup.options[0].label, "Germany");
        assert!(popup.options[0].selected && popup.options[1].disabled);
        let msg = popup.message();
        assert_eq!(msg["type"], "select");
        assert_eq!(msg["options"][1]["value"], "fr");
        assert_eq!(msg["rect"]["width"], 200.0);
        assert_eq!(msg["multiple"], false);
    }

    #[test]
    fn no_select_or_a_malformed_reply_lets_the_press_through() {
        assert!(parse_select_probe(&Value::Null).is_none());
        assert!(parse_select_probe(&json!({"options": "x", "rect": {"x":0,"y":0,"width":1,"height":1}})).is_none());
        assert!(parse_select_probe(&json!({"options": [], "rect": {"x":0,"y":0,"width":1}})).is_none());
        assert!(parse_select_probe(&json!({"options": [], "rect": {"x":"0","y":0,"width":1,"height":1}})).is_none());
    }

    #[test]
    fn page_text_is_capped() {
        let long = "x".repeat(MAX_OPTION_TEXT * 3);
        let many: Vec<Value> = (0..MAX_SELECT_OPTIONS + 50).map(|i| json!({"label": long, "value": i.to_string()})).collect();
        let popup = parse_select_probe(&json!({"options": many, "multiple": true, "rect": {"x":0,"y":0,"width":-5,"height":10}})).unwrap();
        assert_eq!(popup.options.len(), MAX_SELECT_OPTIONS);
        assert_eq!(popup.options[0].label.chars().count(), MAX_OPTION_TEXT);
        assert_eq!(popup.rect.width, 0.0);
        assert!(popup.multiple);
    }

    #[test]
    fn the_probe_only_uses_numbers_from_the_client() {
        let js = select_probe_expression(12.5, 300.0);
        assert!(js.contains("elementFromPoint(12.5, 300)"));
        assert!(js.contains("closest('select')"));
        assert!(js.contains(&format!("i < {MAX_SELECT_OPTIONS}")));
    }
}
