//! Dev mode (docs/dev-mode.md, plan 0.7): a space watches one GitHub repo, read only. The poller
//! writes a pending card per merge to the watched branch and per open PR head, with its DevFacts;
//! `generate` turns them into cards. The production floor lives in `crate::mcp::prod_floor`.
//!
//! Lin owns `facts`, `routes`, `poller`, `repo` and `backup`; Vega owns `generate`, `landed`,
//! `review`, `risk`, `tamper` and `notes`.

pub mod backup;
pub mod facts;
pub mod generate;
pub mod landed;
pub mod notes;
pub mod poller;
pub mod repo;
pub mod review;
pub mod risk;
pub mod routes;
pub mod tamper;

pub use generate::generate;

use crate::db::dev_mode::DevCardRow;
use crate::drivers::events::RuntimeEvent;
use crate::server::AppState;

/// Sends `dev_card {space_id, id, kind, status}` on the event stream.
pub fn notify_card(state: &AppState, row: &DevCardRow) {
    let _ = state.event_sender.send(RuntimeEvent::DevCard {
        space_id: row.space_id.clone(),
        id: row.id.clone(),
        kind: row.kind.clone(),
        status: row.status.clone(),
    });
}

/// Sends `dev_mode {space_id, on}` on the event stream.
pub fn notify_mode(state: &AppState, space_id: &str, on: bool) {
    let _ = state.event_sender.send(RuntimeEvent::DevMode { space_id: space_id.into(), on });
}
