//! Back, forward and reload for the person's takeover view (`{action:"history"}`).
//!
//! The target entry comes from `Page.getNavigationHistory`, so the move stays inside the tab's
//! own history. Like `navigate`, the person may only land on an HTTP(S) page (or about:blank).

use super::browser::{BrowserController, BrowserError};
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryDirection {
    Back,
    Forward,
    Reload,
}

/// The history entry id a back or forward move goes to, from a `Page.getNavigationHistory`
/// result. An error at either end of the history, on a malformed result, or when the entry is
/// not a web page.
pub fn history_target(history: &Value, direction: HistoryDirection) -> Result<i64, BrowserError> {
    let step: i64 = match direction {
        HistoryDirection::Back => -1,
        HistoryDirection::Forward => 1,
        HistoryDirection::Reload => return Err(BrowserError::Protocol("Reload has no history entry".into())),
    };
    let entries = history["entries"].as_array()
        .ok_or_else(|| BrowserError::Protocol("No navigation history".into()))?;
    let current = history["currentIndex"].as_i64()
        .filter(|i| *i >= 0 && (*i as usize) < entries.len())
        .ok_or_else(|| BrowserError::Protocol("No navigation history".into()))?;
    let index = current + step;
    if index < 0 || index as usize >= entries.len() {
        return Err(BrowserError::Protocol(match direction {
            HistoryDirection::Back => "No page to go back to".into(),
            _ => "No page to go forward to".into(),
        }));
    }
    let entry = &entries[index as usize];
    let url = entry["url"].as_str().unwrap_or_default();
    let web = url == "about:blank"
        || reqwest::Url::parse(url).is_ok_and(|u| matches!(u.scheme(), "http" | "https"));
    if !web { return Err(BrowserError::Protocol("That history entry isn't a web page".into())); }
    entry["id"].as_i64().ok_or_else(|| BrowserError::Protocol("No navigation history".into()))
}

impl BrowserController {
    /// Moves the tab one entry back or forward, or reloads it. Returns once Chrome accepted the
    /// command; the screencast and the location tick show the result.
    pub async fn navigate_history(&self, direction: HistoryDirection) -> Result<(), BrowserError> {
        if direction == HistoryDirection::Reload {
            self.send_cdp_command("Page.reload", json!({})).await?;
            return Ok(());
        }
        let history = self.send_cdp_command("Page.getNavigationHistory", json!({})).await?;
        let entry = history_target(&history, direction)?;
        self.send_cdp_command("Page.navigateToHistoryEntry", json!({"entryId": entry})).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn history(current: i64, urls: &[&str]) -> Value {
        json!({
            "currentIndex": current,
            "entries": urls.iter().enumerate().map(|(i, url)| json!({"id": 100 + i as i64, "url": url, "title": ""})).collect::<Vec<_>>(),
        })
    }

    fn message(result: Result<i64, BrowserError>) -> String {
        result.expect_err("expected an error").to_string()
    }

    #[test]
    fn back_and_forward_pick_the_neighbouring_entry() {
        let h = history(1, &["https://a.example/", "https://b.example/", "https://c.example/"]);
        assert_eq!(history_target(&h, HistoryDirection::Back).unwrap(), 100);
        assert_eq!(history_target(&h, HistoryDirection::Forward).unwrap(), 102);
    }

    #[test]
    fn the_ends_of_history_are_errors() {
        let h = history(0, &["https://a.example/", "https://b.example/"]);
        assert!(message(history_target(&h, HistoryDirection::Back)).contains("go back"));
        let h = history(1, &["https://a.example/", "https://b.example/"]);
        assert!(message(history_target(&h, HistoryDirection::Forward)).contains("go forward"));
    }

    #[test]
    fn only_web_pages_and_blank_are_reachable() {
        let h = history(1, &["about:blank", "https://b.example/", "chrome://settings/"]);
        assert_eq!(history_target(&h, HistoryDirection::Back).unwrap(), 100);
        assert!(message(history_target(&h, HistoryDirection::Forward)).contains("isn't a web page"));
        let h = history(1, &["file:///etc/passwd", "http://b.example/"]);
        assert!(history_target(&h, HistoryDirection::Back).is_err());
        let h = history(1, &["data:text/html,hi", "http://b.example/"]);
        assert!(history_target(&h, HistoryDirection::Back).is_err());
    }

    #[test]
    fn malformed_history_is_an_error() {
        for bad in [json!(null), json!({"entries": []}), json!({"currentIndex": 3, "entries": [{"id": 1, "url": "https://a.example/"}]}),
                    json!({"currentIndex": -1, "entries": [{"id": 1, "url": "https://a.example/"}]}),
                    json!({"currentIndex": 1, "entries": [{"url": "https://a.example/"}, {"id": 2, "url": "https://b.example/"}]})] {
            assert!(history_target(&bad, HistoryDirection::Back).is_err(), "{bad}");
        }
        assert!(history_target(&history(0, &["https://a.example/"]), HistoryDirection::Reload).is_err());
    }

    #[test]
    fn direction_parses_from_the_wire() {
        for (wire, want) in [("back", HistoryDirection::Back), ("forward", HistoryDirection::Forward), ("reload", HistoryDirection::Reload)] {
            assert_eq!(serde_json::from_value::<HistoryDirection>(json!(wire)).unwrap(), want);
        }
        assert!(serde_json::from_value::<HistoryDirection>(json!("sideways")).is_err());
    }
}
