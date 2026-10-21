//! kwant's own unfinished work: chats a bot never answered, tasks that failed and were never
//! retried, questions bots asked the person that are still open. Zero setup; reads the database.

use super::{clip, ConvKind, Conversation, MissionSource, Msg, SourceKind, MAX_MESSAGE_CHARS};
use crate::db::Database;
use chrono::{DateTime, Utc};

pub const SOURCE_ID: &str = "kwant";

pub struct KwantSource {
    db: Database,
}

impl KwantSource {
    pub fn new(db: Database) -> Self {
        Self { db }
    }
}

fn msg(from: &str, outbound: bool, at: DateTime<Utc>, text: &str) -> Msg {
    Msg { from: from.into(), outbound, at, text: clip(text.trim(), MAX_MESSAGE_CHARS), auto: false, bulk: false, message_id: None }
}

#[async_trait::async_trait]
impl MissionSource for KwantSource {
    fn kind(&self) -> SourceKind {
        SourceKind::Kwant
    }

    fn id(&self) -> &str {
        SOURCE_ID
    }

    async fn conversations(&self, since: DateTime<Utc>) -> Result<Vec<Conversation>, String> {
        let failed = |e: crate::db::DbError| {
            tracing::warn!("zombie: reading kwant's threads failed: {e}");
            "could not read kwant's own threads".to_string()
        };
        let mut out = Vec::new();
        for t in self.db.zombie_kwant_threads(since).await.map_err(failed)? {
            let mut c = Conversation::new(SourceKind::Kwant, SOURCE_ID, &format!("thread:{}", t.thread_id), ConvKind::KwantThread, &t.title);
            c.counterpart = Some(t.kwant_id.clone());
            c.counterpart_name = Some(t.kwant_name.clone());
            c.space_id = t.space_id.clone();
            c.kwant_id = Some(t.kwant_id.clone());
            c.messages = vec![msg("you", true, t.created_at, &t.text)];
            out.push(with_ref(c, &t.thread_id));
        }
        for d in self.db.zombie_kwant_delegations(since).await.map_err(failed)? {
            let mut c = Conversation::new(SourceKind::Kwant, SOURCE_ID, &format!("delegation:{}", d.id), ConvKind::KwantDelegation, &d.title);
            c.counterpart_name = Some(d.kwant_name.clone());
            c.space_id = d.space_id.clone();
            c.kwant_id = Some(d.kwant_id.clone());
            c.failed = true;
            let outcome = match (d.status.as_str(), d.error.as_deref().map(str::trim).filter(|e| !e.is_empty())) {
                (_, Some(e)) => format!("The task failed: {e}"),
                ("interrupted", None) => "The task was interrupted before it finished.".to_string(),
                _ => "The task failed.".to_string(),
            };
            c.messages = vec![msg("you", true, d.created_at, &d.task), msg(&d.kwant_name, false, d.ended_at, &outcome)];
            out.push(with_ref(c, &d.thread_id));
        }
        for q in self.db.zombie_kwant_questions(since).await.map_err(failed)? {
            let mut c = Conversation::new(SourceKind::Kwant, SOURCE_ID, &format!("question:{}", q.id), ConvKind::KwantQuestion, &clip(&q.question, 120));
            c.counterpart = Some(q.kwant_id.clone());
            c.counterpart_name = Some(q.kwant_name.clone());
            c.space_id = q.space_id.clone();
            c.kwant_id = Some(q.kwant_id.clone());
            c.messages = vec![msg(&q.kwant_name, false, q.created_at, &q.question)];
            let thread = q.thread_id.clone().unwrap_or_default();
            out.push(with_ref(c, &thread));
        }
        Ok(out)
    }
}

/// The thread a follow-up is written in, as the case's `source_ref` (the dedupe key stays the
/// conversation's own key).
fn with_ref(mut c: Conversation, thread_id: &str) -> Conversation {
    c.link = (!thread_id.is_empty()).then(|| format!("kwant://thread/{thread_id}"));
    c
}
