//! Card generation (Vega, plan §4): turns a pending card's DevFacts into the card JSON (§3.4).
//! This is Lin's stub so the core is testable alone: it marks the card ready with no text.

use crate::server::AppState;

/// Generates the card `card_id`, then sets its status (`ready` or `failed`) and sends `dev_card`.
pub async fn generate(state: AppState, card_id: String) {
    match state.db.set_dev_card(&card_id, crate::db::dev_mode::READY, None).await {
        Ok(Some(row)) => super::notify_card(&state, &row),
        Ok(None) => {}
        Err(e) => tracing::warn!(card = %card_id, "dev: could not mark the card ready: {e}"),
    }
}
