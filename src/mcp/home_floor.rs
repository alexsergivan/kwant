//! 26f: on a Home Assistant server, a call that acts on a lock, a door, a garage door, a gate or
//! an alarm always shows the card, in every approval mode (approvals off too), and "Always allow"
//! never covers it. Home Assistant controls everything through a few generic tools
//! (`intent__HassTurnOff` unlocks a lock, `intent__HassTurnOn`/`HassSetPosition` open covers,
//! `homeassistant__schedule_command` runs a free-text command later inside Home Assistant), so a
//! grant given for the lights would otherwise cover the front door too.
//!
//! What a server is comes from what it says it is: the `serverInfo.name` of its `initialize`
//! answer, `home-assistant` for Home Assistant's MCP server. kwnt notes it here each time one of
//! its own connections finishes the handshake (a turn's tools, the gateway, a server test), with
//! the tools the server marks read-only; the server test also stores it with the server's row
//! (`mcp_server_facts`), so it is known after a restart too.
//!
//! This module only matches; `server::mcp_runtime` asks it and shows the card.

use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::{LazyLock, Mutex};

/// The `serverInfo.name` Home Assistant's MCP server answers `initialize` with.
pub const HOME_ASSISTANT: &str = "home-assistant";

/// The card's one-line reason. Product copy is Mira's.
pub const REASON: &str = "Locks, doors, garage doors, gates and alarms always ask. Always allow isn’t offered for them.";

/// Words that make a call sensitive, matched as whole words, case-insensitive, in any string
/// argument. `_`, `.`, `-` and spaces all split words, so `lock.front_door` matches too.
const WORDS: &[&str] = &[
    "lock", "locks", "unlock", "unlocks", "door", "doors", "garage", "garages", "gate", "gates", "alarm", "alarms", "disarm",
    "disarms",
];

/// Home Assistant domains that always count.
const DOMAINS: &[&str] = &["lock", "alarm_control_panel"];

/// Cover device classes that count (`cover` with no device class counts too: it can mean every
/// cover in the house, garage doors included).
const COVER_CLASSES: &[&str] = &["garage", "door", "gate"];

/// Home Assistant's own read of the house. It is a read whatever its arguments say, also on a
/// version that doesn't mark it read-only the way kwnt reads the hints.
const READS: &[&str] = &["GetLiveContext", "GetDateTime"];

/// What kwnt saw of one server on its last connection in this run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Seen {
    /// Its `serverInfo.name`, trimmed; `None` when it sent none.
    pub reported: Option<String>,
    /// The tools it marks read-only, once it listed them.
    pub read_only: Option<HashSet<String>>,
}

static SEEN: LazyLock<Mutex<HashMap<String, Seen>>> = LazyLock::new(Default::default);

fn seen_map() -> std::sync::MutexGuard<'static, HashMap<String, Seen>> {
    SEEN.lock().unwrap_or_else(|e| e.into_inner())
}

/// The `serverInfo.name` in an `initialize` answer's `serverInfo`, trimmed and at most 100
/// characters; `None` when there is none.
pub fn reported_name(server_info: &Value) -> Option<String> {
    let name = server_info.get("name")?.as_str()?.trim();
    (!name.is_empty()).then(|| name.chars().take(100).collect())
}

/// Notes what server `server_id` said it is on a handshake. A new handshake forgets the tools
/// listed before it.
pub fn note_server_info(server_id: &str, server_info: &Value) {
    if server_id.is_empty() {
        return;
    }
    seen_map().insert(server_id.to_string(), Seen { reported: reported_name(server_info), read_only: None });
}

/// Notes which of server `server_id`'s tools are read-only, after it listed them.
pub fn note_tools<'a>(server_id: &str, tools: impl IntoIterator<Item = (&'a str, bool)>) {
    if server_id.is_empty() {
        return;
    }
    let read_only = tools.into_iter().filter(|(_, ro)| *ro).map(|(name, _)| name.to_string()).collect();
    seen_map().entry(server_id.to_string()).or_default().read_only = Some(read_only);
}

/// What kwnt saw of server `server_id` in this run, if it connected to it.
pub fn seen(server_id: &str) -> Option<Seen> {
    seen_map().get(server_id).cloned()
}

/// Forgets what kwnt saw of `server_id` (its address changed, or it was deleted).
pub fn forget(server_id: &str) {
    seen_map().remove(server_id);
}

/// Whether a server that reported `name` is Home Assistant's.
pub fn is_home_assistant(name: &str) -> bool {
    name.trim().eq_ignore_ascii_case(HOME_ASSISTANT)
}

/// Whether `tool` (the name after the server, `intent__HassTurnOff`) is one of Home Assistant's
/// own reads ([`READS`]), with or without its `homeassistant__` prefix.
pub fn is_home_read(tool: &str) -> bool {
    let bare = tool.rsplit("__").next().unwrap_or(tool);
    READS.contains(&bare)
}

fn words(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty())
}

/// Whether `text` holds one of [`WORDS`] as a whole word, in any case.
pub fn names_a_lock(text: &str) -> bool {
    words(text).any(|w| WORDS.iter().any(|k| w.eq_ignore_ascii_case(k)))
}

fn strings(v: &Value) -> Vec<&str> {
    match v {
        Value::String(s) => vec![s.as_str()],
        Value::Array(items) => items.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    }
}

fn any_string(v: &Value, depth: usize, hit: &dyn Fn(&str) -> bool) -> bool {
    if depth > 8 {
        return false;
    }
    match v {
        Value::String(s) => hit(s),
        Value::Array(items) => items.iter().any(|i| any_string(i, depth + 1, hit)),
        Value::Object(map) => map.values().any(|i| any_string(i, depth + 1, hit)),
        _ => false,
    }
}

/// Whether a call with `args` acts on a lock, a door, a garage door, a gate or an alarm, from its
/// arguments alone: a `domain` of `lock` or `alarm_control_panel`; `cover` with a `device_class`
/// of garage, door or gate, or with none; or any string argument (a name, an area, a free-text
/// command) naming one of [`WORDS`]. A false match only shows one more card.
pub fn sensitive_args(args: &Value) -> bool {
    let domains = strings(&args["domain"]);
    if domains.iter().any(|d| DOMAINS.iter().any(|k| d.trim().eq_ignore_ascii_case(k))) {
        return true;
    }
    if domains.iter().any(|d| d.trim().eq_ignore_ascii_case("cover")) {
        let classes = strings(&args["device_class"]);
        if classes.is_empty() || classes.iter().any(|c| COVER_CLASSES.iter().any(|k| c.trim().eq_ignore_ascii_case(k))) {
            return true;
        }
    }
    any_string(args, 0, &names_a_lock)
}

/// What a sensitive call on a server needs to be known about it, for [`decide`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerKind {
    HomeAssistant,
    Other,
    /// kwnt never got its `serverInfo` (it couldn't connect): treated as Home Assistant's, the
    /// safe side, but only for a call whose arguments name a lock, a door or an alarm.
    Unknown,
}

impl ServerKind {
    pub fn of(reported: Option<&str>) -> Self {
        match reported {
            Some(name) if is_home_assistant(name) => Self::HomeAssistant,
            Some(_) => Self::Other,
            None => Self::Unknown,
        }
    }
}

/// Whether a call to `tool` with `args` on a server of `kind` must always ask. `read_only` is
/// whether the server marks the tool read-only (`None` when kwnt doesn't know): reads never ask
/// for this reason.
pub fn decide(kind: ServerKind, tool: &str, args: &Value, read_only: Option<bool>) -> bool {
    if kind == ServerKind::Other || read_only == Some(true) || is_home_read(tool) {
        return false;
    }
    sensitive_args(args)
}

/// 27e: said to the model (never in Details) after a `GetLiveContext` with a name, area or domain
/// that Home Assistant answered with an error ("No exposed entities found in domain(s): switch"):
/// a 7B model answered "There are no switches left on" while two lights were on. Product copy is
/// Mira's.
pub const READ_MISS_HINT: &str = "Nothing matched that filter, so you haven't seen the home yet. Call GetLiveContext \
again with no arguments: it lists every device and sensor you can use, with their real names. Answer only from that list.";

/// 27e: said to the model (never in Details) after a `GetLiveContext` with a filter that worked:
/// "Check the lights" read one room and House said "The living room light is off".
pub const READ_PART_HINT: &str =
    "This list only covers that filter. If the person asked about the whole home, call GetLiveContext again with no arguments.";

/// The filters Home Assistant's `GetLiveContext` takes.
const FILTERS: &[&str] = &["name", "area", "floor", "domain", "device_class"];

/// Whether `tool` is Home Assistant's `GetLiveContext`, with or without its `homeassistant__`
/// prefix.
pub fn is_live_context(tool: &str) -> bool {
    tool.rsplit("__").next().unwrap_or(tool) == "GetLiveContext"
}

/// Whether a read's `args` narrow it: a non-empty name, area, floor, domain or device class.
pub fn filtered(args: &Value) -> bool {
    FILTERS.iter().any(|k| match &args[*k] {
        Value::String(s) => !s.trim().is_empty(),
        Value::Array(items) => items.iter().any(|i| i.as_str().is_some_and(|s| !s.trim().is_empty())),
        Value::Null => false,
        _ => true,
    })
}

/// The hint the model gets after Home Assistant's `GetLiveContext` (`tool`, the name after the
/// server) with `args` ended with record `status` ("ok" or "error"): [`READ_MISS_HINT`] after a
/// filtered miss, [`READ_PART_HINT`] after a filtered read that worked, none for a read with no
/// filter, another tool or a call that didn't run. The caller checks that the server is Home
/// Assistant's.
pub fn live_context_hint(tool: &str, args: &Value, status: &str) -> Option<&'static str> {
    if !is_live_context(tool) || !filtered(args) {
        return None;
    }
    match status {
        "error" => Some(READ_MISS_HINT),
        "ok" => Some(READ_PART_HINT),
        _ => None,
    }
}

/// Both hints, for what shows the person a step (they are the model's, never Details').
pub const HINTS: &[&str] = &[READ_MISS_HINT, READ_PART_HINT];

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn locks_doors_gates_and_alarms_are_sensitive_and_lights_are_not() {
        for args in [
            json!({"domain": ["lock"]}),
            json!({"domain": "lock"}),
            json!({"domain": ["alarm_control_panel"]}),
            json!({"domain": ["cover"], "device_class": ["garage"]}),
            json!({"domain": ["cover"], "device_class": "gate"}),
            json!({"domain": ["cover"]}),
            json!({"name": "Front door"}),
            json!({"name": "Garage"}),
            json!({"name": "Back door lock"}),
            json!({"name": "lock.front_door"}),
            json!({"area": "GATES"}),
            json!({"command": "unlock the back door"}),
            json!({"command": "Disarm the alarm at 7"}),
            json!({"data": {"nested": ["the garage, please"]}}),
        ] {
            assert!(sensitive_args(&args), "{args}");
        }
        for args in [
            json!({"domain": ["light"]}),
            json!({"name": "Living room lamp"}),
            json!({"area": "Kitchen", "domain": ["light"]}),
            json!({"domain": ["cover"], "device_class": ["blind"]}),
            json!({"name": "Doorbell camera light"}),
            json!({"name": "Blocked drain"}),
            json!({"name": "Gateway router"}),
            json!({"brightness": 40}),
            json!({}),
        ] {
            assert!(!sensitive_args(&args), "{args}");
        }
    }

    #[test]
    fn only_home_assistant_or_an_unknown_server_asks_and_reads_never_do() {
        let lock = json!({"domain": ["lock"]});
        assert!(decide(ServerKind::HomeAssistant, "intent__HassTurnOff", &lock, None));
        assert!(decide(ServerKind::HomeAssistant, "intent__HassTurnOff", &lock, Some(false)));
        assert!(decide(ServerKind::Unknown, "intent__HassTurnOff", &lock, None));
        assert!(!decide(ServerKind::Other, "intent__HassTurnOff", &lock, None), "a server that isn't Home Assistant");
        assert!(!decide(ServerKind::HomeAssistant, "intent__HassTurnOff", &json!({"domain": ["light"]}), None));
        assert!(!decide(ServerKind::HomeAssistant, "homeassistant__GetLiveContext", &lock, Some(false)), "HA's own read");
        assert!(!decide(ServerKind::HomeAssistant, "todo_get_items", &json!({"name": "Door list"}), Some(true)), "a read-only tool");
        assert!(decide(ServerKind::HomeAssistant, "homeassistant__schedule_command", &json!({"command": "unlock the back door"}), None));
    }

    #[test]
    fn the_server_says_what_it_is() {
        assert_eq!(ServerKind::of(reported_name(&json!({"name": " home-assistant ", "version": "1"})).as_deref()), ServerKind::HomeAssistant);
        assert_eq!(ServerKind::of(Some("Home-Assistant")), ServerKind::HomeAssistant);
        assert_eq!(ServerKind::of(Some("kwnt-echo")), ServerKind::Other);
        assert_eq!(ServerKind::of(reported_name(&json!({"name": ""})).as_deref()), ServerKind::Unknown);
        assert_eq!(ServerKind::of(reported_name(&Value::Null).as_deref()), ServerKind::Unknown);

        note_server_info("srv-unit-1", &json!({"name": "home-assistant"}));
        note_tools("srv-unit-1", [("homeassistant__GetLiveContext", true), ("intent__HassTurnOff", false)]);
        let s = seen("srv-unit-1").unwrap();
        assert_eq!(s.reported.as_deref(), Some("home-assistant"));
        assert!(s.read_only.as_ref().unwrap().contains("homeassistant__GetLiveContext"));
        assert!(!s.read_only.as_ref().unwrap().contains("intent__HassTurnOff"));
        // A new handshake starts over
        note_server_info("srv-unit-1", &json!({"name": "other"}));
        assert_eq!(seen("srv-unit-1").unwrap().read_only, None);
        forget("srv-unit-1");
        assert_eq!(seen("srv-unit-1"), None);
    }

    #[test]
    fn a_filtered_read_gets_a_hint_and_a_whole_read_does_not() {
        let read = "homeassistant__GetLiveContext";
        // HA 2026.11.1's misses from the report
        assert_eq!(live_context_hint(read, &json!({"area": "", "domain": ["switch"]}), "error"), Some(READ_MISS_HINT));
        assert_eq!(live_context_hint(read, &json!({"domain": "sensor", "name": "temperature"}), "error"), Some(READ_MISS_HINT));
        assert_eq!(live_context_hint("GetLiveContext", &json!({"area": "common", "domain": "switch"}), "error"), Some(READ_MISS_HINT));
        assert_eq!(live_context_hint(read, &json!({"area": "living_room", "domain": ["light"]}), "ok"), Some(READ_PART_HINT));
        // No filter: nothing to widen
        assert_eq!(live_context_hint(read, &json!({}), "ok"), None);
        assert_eq!(live_context_hint(read, &json!({}), "error"), None);
        assert_eq!(live_context_hint(read, &json!({"area": "", "domain": []}), "error"), None);
        // A call that didn't run, or another tool
        assert_eq!(live_context_hint(read, &json!({"domain": ["light"]}), "denied"), None);
        assert_eq!(live_context_hint("intent__HassTurnOff", &json!({"domain": ["light"]}), "error"), None);
        assert_eq!(live_context_hint("homeassistant__GetDateTime", &json!({"name": "x"}), "ok"), None);
    }
}
