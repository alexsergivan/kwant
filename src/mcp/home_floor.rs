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

/// 28m: the paths Home Assistant serves its MCP server at: `/api/mcp` (streamable HTTP) and
/// `/mcp_server/sse` (the older SSE endpoint).
const PATHS: &[&str] = &["/api/mcp", "/mcp_server/sse"];

/// 28m: whether `address` is an `http(s)` URL at Home Assistant's own MCP path ([`PATHS`]), also
/// under a prefix a proxy adds (`https://example.com/ha/api/mcp`); a trailing `/`, the query and
/// the fragment don't matter. Only a hint, for a server kwnt never heard from: what a server says
/// it is always wins.
pub fn at_home_assistant_path(address: &str) -> bool {
    let Some((scheme, rest)) = address.trim().split_once("://") else {
        return false;
    };
    if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
        return false;
    }
    let rest = rest.split(['?', '#']).next().unwrap_or_default();
    let Some(slash) = rest.find('/') else {
        return false;
    };
    let path = rest[slash..].trim_end_matches('/');
    PATHS.iter().any(|p| path.len().checked_sub(p.len()).and_then(|at| path.get(at..)).is_some_and(|end| end.eq_ignore_ascii_case(p)))
}

/// 28m: whether a server counts as Home Assistant's for the delivery check. `heard` is what kwnt
/// heard it say it is (on a connection in this run, else at its last test): `Some(None)` when it
/// gave no name, `None` when kwnt never heard from it. A server kwnt never heard from counts when
/// its address is at Home Assistant's MCP path: its `url`, or, for a `stdio` server (a proxy such
/// as `mcp-proxy`), one of its arguments (`--url=…` too). Counting a server that isn't means at
/// most one "did that change really happen?" continuation, the safe side; missing one lets a
/// false "Unlocking the front door now." through.
pub fn counts_as_home_assistant(heard: Option<Option<&str>>, transport: &str, url: Option<&str>, args: &[String]) -> bool {
    match heard {
        Some(name) => name.is_some_and(is_home_assistant),
        None if transport == "stdio" => {
            args.iter().any(|a| at_home_assistant_path(a) || a.split_once('=').is_some_and(|(_, v)| at_home_assistant_path(v)))
        }
        None => url.is_some_and(at_home_assistant_path),
    }
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

/// 27e: said to the model (never in Details) after every `GetLiveContext` with a name, area or
/// domain that ended with an error: Home Assistant's own miss ("No exposed entities found in
/// domain(s): switch"), where a 7B model answered "There are no switches left on" while two lights
/// were on, but also bad arguments or Home Assistant being down, and even after an earlier whole
/// read worked (it is per call). The wording is true in all of those. Product copy is Mira's.
pub const READ_MISS_HINT: &str = "That filtered read didn't work, and a filter can miss devices. Call GetLiveContext \
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

    /// 28m: Home Assistant's own MCP paths, also behind a proxy's prefix; other paths aren't.
    #[test]
    fn home_assistants_mcp_path_is_known_by_its_address() {
        for address in [
            "http://homeassistant.local:8123/api/mcp",
            "http://192.168.1.20:8123/api/mcp/",
            "https://abcd1234.ui.nabu.casa/api/mcp",
            "HTTPS://ha.example.com/API/MCP?x=1",
            "https://example.com/homeassistant/api/mcp#top",
            "http://homeassistant.local:8123/mcp_server/sse",
            "  http://localhost:8123/api/mcp  ",
        ] {
            assert!(at_home_assistant_path(address), "{address}");
        }
        for address in [
            "http://homeassistant.local:8123/",
            "http://homeassistant.local:8123",
            "http://127.0.0.1:7777/api/mcp/gateway",
            "http://127.0.0.1:7777/api/mcp/self",
            "https://mcp.notion.com/mcp",
            "https://example.com/xapi/mcp",
            "https://example.com/?next=/api/mcp",
            "ftp://ha.local/api/mcp",
            "/api/mcp",
            "api/mcp",
            "",
            "http://ha.local/ápi/mcp",
            "http://ha.local/é/ab/mcp",
        ] {
            assert!(!at_home_assistant_path(address), "{address}");
        }
    }

    /// 28m: what a server said it is always wins; only one kwnt never heard from goes by its
    /// address (its url, or a proxy's argument for a stdio server).
    #[test]
    fn a_server_never_heard_from_counts_by_its_address() {
        let ha = Some("http://homeassistant.local:8123/api/mcp");
        let other = Some("https://mcp.notion.com/mcp");
        let none: &[String] = &[];
        // Never heard from: the address decides
        assert!(counts_as_home_assistant(None, "http", ha, none));
        assert!(!counts_as_home_assistant(None, "http", other, none));
        assert!(!counts_as_home_assistant(None, "http", None, none));
        // Heard from: what it said, whatever its address
        assert!(counts_as_home_assistant(Some(Some("home-assistant")), "http", other, none));
        assert!(counts_as_home_assistant(Some(Some(" Home-Assistant ")), "http", None, none));
        assert!(!counts_as_home_assistant(Some(Some("kwnt-echo")), "http", ha, none));
        assert!(!counts_as_home_assistant(Some(None), "http", ha, none), "it gave no name: not counted");
        // A stdio proxy to Home Assistant (mcp-proxy, as Home Assistant's docs set it up)
        let proxy: Vec<String> = ["--transport=streamablehttp", "--stateless", "http://localhost:8123/api/mcp"].map(String::from).into();
        assert!(counts_as_home_assistant(None, "stdio", None, &proxy));
        let flag: Vec<String> = ["--url=http://localhost:8123/mcp_server/sse"].map(String::from).into();
        assert!(counts_as_home_assistant(None, "stdio", None, &flag));
        let notes: Vec<String> = ["mcp-server-notes", "--root", "/home/me/api/mcp"].map(String::from).into();
        assert!(!counts_as_home_assistant(None, "stdio", None, &notes));
        assert!(!counts_as_home_assistant(Some(Some("mcp-proxy")), "stdio", None, &proxy), "heard from: what it said");
        // A stdio server's url field isn't its address
        assert!(!counts_as_home_assistant(None, "stdio", ha, none));
    }

    /// 27e round 2 (Mira): the miss hint goes with every filtered read that ended in an error
    /// (Home Assistant's own "No exposed entities…", bad arguments, Home Assistant down) and with
    /// each call, even after a whole read worked, so it claims only what all of those share.
    #[test]
    fn the_miss_hint_is_true_for_every_error() {
        assert_eq!(
            READ_MISS_HINT,
            "That filtered read didn't work, and a filter can miss devices. Call GetLiveContext again with no arguments: \
it lists every device and sensor you can use, with their real names. Answer only from that list."
        );
        assert!(!READ_MISS_HINT.contains("Nothing matched") && !READ_MISS_HINT.contains("haven't seen"));
        // The record's status is all it goes by: the same hint whatever the error says
        let read = "homeassistant__GetLiveContext";
        for args in [json!({"domain": ["switch"]}), json!({"name": 7}), json!({"area": "kitchen"})] {
            assert_eq!(live_context_hint(read, &args, "error"), Some(READ_MISS_HINT), "{args}");
        }
    }
}
