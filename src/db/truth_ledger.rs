//! Mission Control Phase C1: the Truth Ledger in shadow mode. Every bot memory (`kwant_memories`)
//! is mirrored into `knowledge_facts` by SQL triggers, so nothing can write memory without the
//! ledger seeing it; `kwant_memories` stays the write path and nothing is moved or deleted.
//! Add-only: older binaries keep writing memory and the triggers still mirror it, and running it
//! twice changes nothing. See docs/mission-control.md §21.

use super::{Database, DbError};
use chrono::Utc;
use serde::Serialize;
use sqlx::{FromRow, SqliteConnection};

/// A fact still in force (not superseded).
pub const LIVE: &str = "('proposed', 'accepted', 'disputed')";

const TABLES: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS knowledge_facts (
        id TEXT PRIMARY KEY,
        space_id TEXT NOT NULL,
        scope TEXT NOT NULL CHECK (scope IN ('bot', 'team', 'space')),
        scope_id TEXT NOT NULL,
        statement TEXT NOT NULL,
        subject TEXT,
        source_kind TEXT NOT NULL CHECK (source_kind IN ('person', 'bot', 'import', 'check', 'task')),
        source_ref TEXT,
        author_kwant_id TEXT,
        trust TEXT NOT NULL CHECK (trust IN ('person', 'bot', 'outside')),
        status TEXT NOT NULL CHECK (status IN ('proposed', 'accepted', 'superseded', 'disputed')),
        superseded_by TEXT REFERENCES knowledge_facts(id),
        valid_from TEXT NOT NULL,
        valid_until TEXT,
        memory_id TEXT,
        pinned INTEGER NOT NULL DEFAULT 0,
        decided_by TEXT CHECK (decided_by IN ('person', 'evidence')),
        decided_at TEXT,
        created_at TEXT NOT NULL,
        updated_at TEXT NOT NULL
    )",
    // One fact in force per memory; superseded ones stay as its history
    "CREATE UNIQUE INDEX IF NOT EXISTS knowledge_facts_live_memory ON knowledge_facts (memory_id) \
     WHERE memory_id IS NOT NULL AND status IN ('proposed', 'accepted', 'disputed')",
    "CREATE INDEX IF NOT EXISTS knowledge_facts_memory ON knowledge_facts (memory_id)",
    "CREATE INDEX IF NOT EXISTS knowledge_facts_scope ON knowledge_facts (space_id, scope, scope_id, status)",
    "CREATE TABLE IF NOT EXISTS fact_evidence (
        fact_id TEXT NOT NULL REFERENCES knowledge_facts(id),
        evidence_id TEXT REFERENCES evidence(id),
        check_id TEXT REFERENCES outcome_checks(id),
        message_id TEXT,
        added_by TEXT NOT NULL,
        added_at TEXT NOT NULL
    )",
    "CREATE INDEX IF NOT EXISTS fact_evidence_fact ON fact_evidence (fact_id)",
    "CREATE TABLE IF NOT EXISTS fact_citations (
        fact_id TEXT NOT NULL REFERENCES knowledge_facts(id),
        cited_by_kind TEXT NOT NULL CHECK (cited_by_kind IN ('task', 'fact')),
        cited_by_id TEXT NOT NULL,
        created_at TEXT NOT NULL,
        PRIMARY KEY (fact_id, cited_by_kind, cited_by_id)
    )",
    "CREATE TABLE IF NOT EXISTS fact_disputes (
        id TEXT PRIMARY KEY,
        fact_a TEXT NOT NULL,
        fact_b TEXT NOT NULL,
        cause TEXT NOT NULL CHECK (cause IN ('same_subject', 'numbers_differ', 'negation', 'person', 'bot')),
        status TEXT NOT NULL CHECK (status IN ('open', 'resolved', 'dismissed')),
        winner TEXT,
        resolved_by TEXT CHECK (resolved_by IN ('person', 'evidence')),
        created_at TEXT NOT NULL,
        resolved_at TEXT,
        UNIQUE (fact_a, fact_b),
        CHECK (fact_a < fact_b)
    )",
    "CREATE TABLE IF NOT EXISTS fact_events (
        id INTEGER PRIMARY KEY,
        fact_id TEXT NOT NULL,
        kind TEXT NOT NULL,
        actor TEXT NOT NULL,
        detail TEXT,
        created_at TEXT NOT NULL
    )",
    "CREATE INDEX IF NOT EXISTS fact_events_fact ON fact_events (fact_id, id)",
    // Where the next write of a memory came from, when the row alone can't say (a tainted turn,
    // an import). Written in the memory write's transaction; the mirror trigger reads and deletes it.
    "CREATE TABLE IF NOT EXISTS fact_provenance (
        memory_id TEXT PRIMARY KEY,
        trust TEXT NOT NULL CHECK (trust IN ('person', 'bot', 'outside')),
        source_kind TEXT NOT NULL CHECK (source_kind IN ('person', 'bot', 'import', 'check', 'task')),
        source_ref TEXT
    )",
    // Lets the privacy paths (a deleted bot, an undone import) delete that bot's facts and events;
    // written and cleared inside the deletion's transaction (see [`PURGE_BOT_FACTS`]).
    "CREATE TABLE IF NOT EXISTS fact_purge_permits (scope_id TEXT PRIMARY KEY)",
    // A bot re-adding a note it removed: finds the outside fact with the same text
    "CREATE INDEX IF NOT EXISTS knowledge_facts_outside_text ON knowledge_facts (scope_id, statement) WHERE trust = 'outside'",
    // C2: disputes by status, citations by the citing task, one row per piece of evidence
    "CREATE INDEX IF NOT EXISTS fact_disputes_status ON fact_disputes (status, created_at)",
    "CREATE INDEX IF NOT EXISTS fact_disputes_b ON fact_disputes (fact_b)",
    "CREATE INDEX IF NOT EXISTS fact_citations_by ON fact_citations (cited_by_kind, cited_by_id)",
    "CREATE UNIQUE INDEX IF NOT EXISTS fact_evidence_once ON fact_evidence \
     (fact_id, COALESCE(evidence_id, ''), COALESCE(check_id, ''), COALESCE(message_id, ''))",
];

/// C2 (contract §2.3): a bot's memory as the ledger sees it: notes whose fact is in force and not
/// expired. Superseded (rejected, lost a dispute) and expired facts drop out. Recreated each start.
const MEMORY_VIEW: &str = "CREATE VIEW bot_memory_v AS \
    SELECT f.* FROM knowledge_facts f JOIN kwant_memories m ON m.id = f.memory_id \
    WHERE f.scope = 'bot' AND f.status IN ('accepted', 'proposed', 'disputed') \
      AND (f.valid_until IS NULL OR f.valid_until > strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))";

/// A fact in force and not expired, for queries on `knowledge_facts f`.
const IN_FORCE: &str = "f.status IN ('proposed', 'accepted', 'disputed') \
    AND (f.valid_until IS NULL OR f.valid_until > strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))";

/// A `disputed` fact with no open dispute left goes back to where it was: `accepted` when someone
/// decided it or it is the bot's own note, else `proposed` (an outside note, a shared proposal).
/// Two statements, event first (the purge runs them too, binding a `?1` they don't use).
const RESTORE: &[&str] = &[
    "INSERT INTO fact_events (fact_id, kind, actor, created_at) \
     SELECT f.id, 'restored', 'system', strftime('%Y-%m-%dT%H:%M:%fZ', 'now') FROM knowledge_facts f \
     WHERE f.status = 'disputed' AND NOT EXISTS (SELECT 1 FROM fact_disputes d \
       WHERE d.status = 'open' AND (d.fact_a = f.id OR d.fact_b = f.id))",
    "UPDATE knowledge_facts SET status = CASE WHEN decided_by IS NOT NULL OR (scope = 'bot' AND trust = 'bot') \
       THEN 'accepted' ELSE 'proposed' END, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') \
     WHERE status = 'disputed' AND NOT EXISTS (SELECT 1 FROM fact_disputes d \
       WHERE d.status = 'open' AND (d.fact_a = knowledge_facts.id OR d.fact_b = knowledge_facts.id))",
];

/// The new fact of a memory row `new`: trust and source from its provenance row, else from the
/// row itself (the person's note, an import, the bot's own). Outside facts land `proposed`.
const MIRROR_INSERT: &str = "INSERT INTO knowledge_facts (id, space_id, scope, scope_id, statement, source_kind, source_ref, \
        author_kwant_id, trust, status, valid_from, memory_id, pinned, decided_by, decided_at, created_at, updated_at) \
    SELECT 'f_' || lower(hex(randomblob(12))), COALESCE((SELECT space_id FROM kwants WHERE id = new.kwant_id), 'spc_default'), \
        'bot', new.kwant_id, new.text, d.kind, d.ref, CASE WHEN new.source = 'agent' THEN new.kwant_id END, d.trust, \
        CASE WHEN d.trust = 'outside' THEN 'proposed' ELSE 'accepted' END, \
        {valid_from}, new.id, new.pinned_by IS NOT NULL, \
        CASE WHEN d.trust = 'person' THEN 'person' END, CASE WHEN d.trust = 'person' THEN strftime('%Y-%m-%dT%H:%M:%fZ', 'now') END, \
        strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), strftime('%Y-%m-%dT%H:%M:%fZ', 'now') \
    FROM (SELECT COALESCE(p.trust, CASE WHEN new.imported = 1 THEN 'outside' WHEN new.source = 'user' THEN 'person' ELSE 'bot' END) AS trust, \
                 COALESCE(p.source_kind, CASE WHEN new.imported = 1 THEN 'import' WHEN new.source = 'user' THEN 'person' ELSE 'bot' END) AS kind, \
                 p.source_ref AS ref \
          FROM (SELECT 1) LEFT JOIN fact_provenance p ON p.memory_id = new.id) d;";

fn triggers() -> Vec<String> {
    let now = "strftime('%Y-%m-%dT%H:%M:%fZ', 'now')";
    let live = LIVE;
    vec![
        // A bot re-adding the exact text of an outside fact it had (delete, then add) keeps it outside.
        format!(
            "CREATE TRIGGER IF NOT EXISTS kwant_memories_ledger_ai AFTER INSERT ON kwant_memories BEGIN \
             INSERT OR IGNORE INTO fact_provenance (memory_id, trust, source_kind, source_ref) \
               SELECT new.id, 'outside', f.source_kind, f.source_ref FROM knowledge_facts f \
               WHERE new.source = 'agent' AND f.trust = 'outside' AND f.scope = 'bot' AND f.scope_id = new.kwant_id \
                 AND f.statement = new.text ORDER BY f.created_at DESC, f.rowid DESC LIMIT 1; \
             {insert} \
             INSERT INTO fact_events (fact_id, kind, actor, created_at) \
               SELECT id, 'created', CASE source_kind WHEN 'person' THEN 'person' WHEN 'import' THEN 'import' ELSE new.kwant_id END, {now} \
               FROM knowledge_facts WHERE memory_id = new.id AND status IN {live}; \
             DELETE FROM fact_provenance WHERE memory_id = new.id; END",
            insert = MIRROR_INSERT.replace("{valid_from}", &format!("COALESCE(new.created_at, {now})")),
        ),
        // A changed note: the fact in force goes `superseded`, a new one replaces it (history kept).
        // A bot editing a note that ever came from outside text keeps it outside, even in a clean
        // turn and even after the person accepted an earlier version.
        format!(
            "CREATE TRIGGER IF NOT EXISTS kwant_memories_ledger_au AFTER UPDATE OF text, source, imported ON kwant_memories \
             WHEN old.text IS NOT new.text OR old.source IS NOT new.source OR old.imported IS NOT new.imported BEGIN \
             INSERT OR IGNORE INTO fact_provenance (memory_id, trust, source_kind, source_ref) \
               SELECT new.id, 'outside', f.source_kind, f.source_ref FROM knowledge_facts f \
               WHERE new.source = 'agent' AND f.memory_id = new.id AND f.trust = 'outside' \
               ORDER BY f.created_at DESC, f.rowid DESC LIMIT 1; \
             INSERT INTO fact_events (fact_id, kind, actor, created_at) \
               SELECT id, 'superseded', CASE WHEN new.source = 'agent' THEN new.kwant_id ELSE 'person' END, {now} \
               FROM knowledge_facts WHERE memory_id = new.id AND status IN {live}; \
             UPDATE knowledge_facts SET status = 'superseded', updated_at = {now} WHERE memory_id = new.id AND status IN {live}; \
             {insert} \
             UPDATE knowledge_facts SET superseded_by = (SELECT id FROM knowledge_facts WHERE memory_id = new.id AND status IN {live}) \
               WHERE memory_id = new.id AND status = 'superseded' AND superseded_by IS NULL AND decided_by IS NULL; \
             INSERT INTO fact_events (fact_id, kind, actor, created_at) \
               SELECT id, 'created', CASE WHEN new.source = 'agent' THEN new.kwant_id ELSE 'person' END, {now} \
               FROM knowledge_facts WHERE memory_id = new.id AND status IN {live}; \
             DELETE FROM fact_provenance WHERE memory_id = new.id; END",
            insert = MIRROR_INSERT.replace("{valid_from}", now),
        ),
        format!(
            "CREATE TRIGGER IF NOT EXISTS kwant_memories_ledger_pin AFTER UPDATE OF pinned_by ON kwant_memories \
             WHEN (old.pinned_by IS NULL) <> (new.pinned_by IS NULL) BEGIN \
             UPDATE knowledge_facts SET pinned = new.pinned_by IS NOT NULL, updated_at = {now} WHERE memory_id = new.id AND status IN {live}; \
             INSERT INTO fact_events (fact_id, kind, actor, created_at) \
               SELECT id, CASE WHEN new.pinned_by IS NULL THEN 'unpinned' ELSE 'pinned' END, \
                 CASE COALESCE(new.pinned_by, old.pinned_by) WHEN 'user' THEN 'person' ELSE new.kwant_id END, {now} \
               FROM knowledge_facts WHERE memory_id = new.id AND status IN {live}; END"
        ),
        // A removed note: its fact goes `superseded` with no successor; the row stays as history.
        format!(
            "CREATE TRIGGER IF NOT EXISTS kwant_memories_ledger_ad AFTER DELETE ON kwant_memories BEGIN \
             INSERT INTO fact_events (fact_id, kind, actor, created_at) \
               SELECT id, 'removed', 'system', {now} FROM knowledge_facts WHERE memory_id = old.id AND status IN {live}; \
             UPDATE knowledge_facts SET status = 'superseded', superseded_by = NULL, updated_at = {now} \
               WHERE memory_id = old.id AND status IN {live}; \
             DELETE FROM fact_provenance WHERE memory_id = old.id; END"
        ),
        // Status rules (contract §2.2, §5 rules 1–2)
        "CREATE TRIGGER IF NOT EXISTS knowledge_facts_terminal BEFORE UPDATE ON knowledge_facts \
         WHEN old.status = 'superseded' AND (new.status IS NOT 'superseded' OR new.statement IS NOT old.statement \
           OR new.trust IS NOT old.trust OR new.scope IS NOT old.scope OR new.decided_by IS NOT old.decided_by \
           OR (old.superseded_by IS NOT NULL AND new.superseded_by IS NOT old.superseded_by)) BEGIN \
         SELECT RAISE(ABORT, 'superseded facts are final'); END"
            .into(),
        // A bot's own note (bot scope, bot trust) goes back to `accepted` once its last open dispute
        // is dismissed or settled: it was accepted before, so nobody needs to decide it again.
        "CREATE TRIGGER IF NOT EXISTS knowledge_facts_decided BEFORE UPDATE OF status ON knowledge_facts \
         WHEN new.status = 'accepted' AND old.status IN ('proposed', 'disputed') AND (new.decided_by IS NULL OR new.decided_at IS NULL) \
           AND NOT (old.status = 'disputed' AND new.scope = 'bot' AND new.trust = 'bot' AND NOT EXISTS (SELECT 1 FROM fact_disputes d \
             WHERE d.status = 'open' AND (d.fact_a = new.id OR d.fact_b = new.id))) BEGIN \
         SELECT RAISE(ABORT, 'accepting a proposed or disputed fact needs decided_by'); END"
            .into(),
        // A settled dispute is history
        "CREATE TRIGGER IF NOT EXISTS fact_disputes_final BEFORE UPDATE ON fact_disputes WHEN old.status <> 'open' BEGIN \
         SELECT RAISE(ABORT, 'a resolved or dismissed dispute is final'); END"
            .into(),
        "CREATE TRIGGER IF NOT EXISTS fact_disputes_no_delete BEFORE DELETE ON fact_disputes \
         WHEN NOT EXISTS (SELECT 1 FROM knowledge_facts f JOIN fact_purge_permits p ON p.scope_id = f.scope_id \
           WHERE f.scope = 'bot' AND f.id IN (old.fact_a, old.fact_b)) BEGIN \
         SELECT RAISE(ABORT, 'disputes are deleted only with their bot or import'); END"
            .into(),
        "CREATE TRIGGER IF NOT EXISTS knowledge_facts_rules_insert BEFORE INSERT ON knowledge_facts \
         WHEN (new.trust = 'outside' AND new.status = 'accepted' AND new.decided_by IS NOT 'person') \
           OR (new.scope <> 'bot' AND (new.trust = 'outside' OR new.status = 'accepted') AND new.decided_by IS NOT 'person') BEGIN \
         SELECT RAISE(ABORT, 'outside or shared facts are accepted only by the person'); END"
            .into(),
        "CREATE TRIGGER IF NOT EXISTS knowledge_facts_rules_update BEFORE UPDATE ON knowledge_facts \
         WHEN (new.trust = 'outside' AND new.status = 'accepted' AND new.decided_by IS NOT 'person') \
           OR (new.scope <> 'bot' AND (new.trust = 'outside' OR new.status = 'accepted') AND new.decided_by IS NOT 'person') BEGIN \
         SELECT RAISE(ABORT, 'outside or shared facts are accepted only by the person'); END"
            .into(),
        // Outside text stays outside unless the person decides; and nothing overrules the person
        // afterwards, so an ex-outside fact can't end up `decided_by = 'evidence'`.
        "CREATE TRIGGER IF NOT EXISTS knowledge_facts_trust BEFORE UPDATE ON knowledge_facts \
         WHEN (old.trust = 'outside' AND new.trust IS NOT 'outside' AND new.decided_by IS NOT 'person') \
           OR (old.decided_by = 'person' AND new.decided_by IS NOT 'person') BEGIN \
         SELECT RAISE(ABORT, 'only the person takes a fact out of outside, and their decision stands'); END"
            .into(),
        "CREATE TRIGGER IF NOT EXISTS fact_events_no_update BEFORE UPDATE ON fact_events BEGIN \
         SELECT RAISE(ABORT, 'fact_events is append-only'); END"
            .into(),
        // Facts and their events are deleted only by the privacy paths, which hold a permit for the
        // bot (PURGE_BOT_FACTS). Events of a fact that is already gone may go too.
        "CREATE TRIGGER IF NOT EXISTS knowledge_facts_no_delete BEFORE DELETE ON knowledge_facts \
         WHEN NOT (old.scope = 'bot' AND EXISTS (SELECT 1 FROM fact_purge_permits WHERE scope_id = old.scope_id)) BEGIN \
         SELECT RAISE(ABORT, 'facts are deleted only with their bot or import'); END"
            .into(),
        "CREATE TRIGGER IF NOT EXISTS fact_events_no_delete BEFORE DELETE ON fact_events \
         WHEN EXISTS (SELECT 1 FROM knowledge_facts f WHERE f.id = old.fact_id \
           AND NOT (f.scope = 'bot' AND EXISTS (SELECT 1 FROM fact_purge_permits p WHERE p.scope_id = f.scope_id))) BEGIN \
         SELECT RAISE(ABORT, 'fact_events is append-only'); END"
            .into(),
    ]
}

/// Undoes [`Database::migrate_truth_ledger`]: memory itself is untouched. For tests and a manual
/// rollback.
#[doc(hidden)]
pub const TRUTH_LEDGER_DOWN: &[&str] = &[
    "DROP TRIGGER IF EXISTS kwant_memories_ledger_ai",
    "DROP TRIGGER IF EXISTS kwant_memories_ledger_au",
    "DROP TRIGGER IF EXISTS kwant_memories_ledger_pin",
    "DROP TRIGGER IF EXISTS kwant_memories_ledger_ad",
    "DROP VIEW IF EXISTS bot_memory_v",
    "DROP TABLE IF EXISTS fact_purge_permits",
    "DROP TABLE IF EXISTS fact_provenance",
    "DROP TABLE IF EXISTS fact_events",
    "DROP TABLE IF EXISTS fact_disputes",
    "DROP TABLE IF EXISTS fact_citations",
    "DROP TABLE IF EXISTS fact_evidence",
    "DROP TABLE IF EXISTS knowledge_facts",
];

/// Removes a deleted bot's facts, their events, evidence, citations and disputes (run in the
/// deletion's transaction, after its memories): a bot that no longer exists keeps no history of what it remembered. The only path
/// allowed to delete facts: it holds a permit for the bot while it runs.
pub const PURGE_BOT_FACTS: &[&str] = &[
    "INSERT OR IGNORE INTO fact_purge_permits (scope_id) VALUES (?1)",
    "DELETE FROM fact_evidence WHERE fact_id IN (SELECT id FROM knowledge_facts WHERE scope = 'bot' AND scope_id = ?1)",
    "DELETE FROM fact_citations WHERE fact_id IN (SELECT id FROM knowledge_facts WHERE scope = 'bot' AND scope_id = ?1)",
    "DELETE FROM fact_citations WHERE cited_by_kind = 'fact' AND cited_by_id IN (SELECT id FROM knowledge_facts WHERE scope = 'bot' AND scope_id = ?1)",
    "DELETE FROM fact_disputes WHERE fact_a IN (SELECT id FROM knowledge_facts WHERE scope = 'bot' AND scope_id = ?1) \
       OR fact_b IN (SELECT id FROM knowledge_facts WHERE scope = 'bot' AND scope_id = ?1)",
    "DELETE FROM fact_events WHERE fact_id IN (SELECT id FROM knowledge_facts WHERE scope = 'bot' AND scope_id = ?1)",
    "DELETE FROM knowledge_facts WHERE scope = 'bot' AND scope_id = ?1",
    "DELETE FROM fact_events WHERE fact_id NOT IN (SELECT id FROM knowledge_facts)",
    // The other side of a dispute with a deleted bot's fact goes back to where it was
    RESTORE[0],
    RESTORE[1],
    "DELETE FROM fact_purge_permits WHERE scope_id = ?1",
];

/// Takes the delete guards off again, for a rollback to a release whose bot deletion purges facts
/// without a permit (a703732). Older releases don't purge facts at all and need nothing.
#[doc(hidden)]
pub const DELETE_GUARDS_DOWN: &[&str] =
    &["DROP TRIGGER IF EXISTS knowledge_facts_no_delete", "DROP TRIGGER IF EXISTS fact_events_no_delete"];

/// How a memory write in the current turn should be trusted, when the row alone can't say.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provenance<'a> {
    /// A bot write in a turn that read outside text (`source_ref` is the turn id).
    Outside { turn_id: &'a str },
    /// An import (`source_ref` is the import id).
    Import { import_id: &'a str },
}

/// Records where the next write of memory `memory_id` came from; call it in the write's
/// transaction, before the INSERT or UPDATE. The mirror trigger consumes it.
pub async fn note_provenance(conn: &mut SqliteConnection, memory_id: &str, p: Provenance<'_>) -> Result<(), sqlx::Error> {
    let (kind, reference) = match p {
        Provenance::Outside { turn_id } => ("bot", turn_id),
        Provenance::Import { import_id } => ("import", import_id),
    };
    sqlx::query("INSERT OR REPLACE INTO fact_provenance (memory_id, trust, source_kind, source_ref) VALUES (?, 'outside', ?, ?)")
        .bind(memory_id)
        .bind(kind)
        .bind(reference)
        .execute(conn)
        .await?;
    Ok(())
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct Fact {
    pub id: String,
    pub space_id: String,
    pub scope: String,
    pub scope_id: String,
    pub statement: String,
    pub subject: Option<String>,
    pub source_kind: String,
    pub source_ref: Option<String>,
    pub author_kwant_id: Option<String>,
    pub trust: String,
    pub status: String,
    pub superseded_by: Option<String>,
    pub valid_from: String,
    pub valid_until: Option<String>,
    pub memory_id: Option<String>,
    pub pinned: bool,
    pub decided_by: Option<String>,
    pub decided_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct FactEvent {
    pub id: i64,
    pub kind: String,
    pub actor: String,
    pub detail: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Default)]
pub struct FactFilter<'a> {
    pub scope: Option<&'a str>,
    pub scope_id: Option<&'a str>,
    pub status: Option<&'a str>,
    pub q: Option<&'a str>,
}

/// What a person's accept or reject did.
#[derive(Debug, PartialEq, Eq)]
pub enum Decision {
    /// This call changed the fact.
    Done,
    /// The fact isn't `proposed` (any more): someone decided it first, or it was superseded.
    NotProposed(String),
    NotFound,
}

pub const MAX_FACTS: i64 = 500;

/// Two facts that can't both hold (contract §2.5). `fact_a < fact_b`; one row per pair ever, so a
/// dismissed pair is never raised again.
#[derive(Debug, Clone, Serialize, FromRow)]
pub struct Dispute {
    pub id: String,
    pub fact_a: String,
    pub fact_b: String,
    pub cause: String,
    pub status: String,
    pub winner: Option<String>,
    pub resolved_by: Option<String>,
    pub created_at: String,
    pub resolved_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct FactEvidence {
    pub fact_id: String,
    pub evidence_id: Option<String>,
    pub check_id: Option<String>,
    pub message_id: Option<String>,
    pub added_by: String,
    pub added_at: String,
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct Citation {
    pub fact_id: String,
    pub cited_by_kind: String,
    pub cited_by_id: String,
    pub created_at: String,
}

/// What resolving or dismissing a dispute did.
#[derive(Debug)]
pub enum DisputeDecision {
    /// This call settled it; the dispute as it now stands.
    Done(Box<Dispute>),
    /// It was settled first (by someone else, or because a side was superseded).
    NotOpen(String),
    NotFound,
    /// `winner` isn't one of its two facts.
    BadWinner,
}

/// A new fact written through the API or a tool (not mirrored from memory).
#[derive(Debug)]
pub struct NewFact<'a> {
    pub space_id: &'a str,
    pub scope: &'a str,
    pub scope_id: &'a str,
    pub statement: &'a str,
    pub subject: Option<&'a str>,
    pub source_kind: &'a str,
    pub source_ref: Option<&'a str>,
    pub author_kwant_id: Option<&'a str>,
    pub trust: &'a str,
    /// `person` for the person's own fact (lands `accepted`); else it lands `proposed`.
    pub decided_by_person: bool,
    pub actor: &'a str,
}

/// Check kinds kwant runs itself; peer reviews and person checks are judgements, not evidence.
/// Whether a status is one of [`LIVE`].
pub fn live(status: &str) -> bool {
    matches!(status, "proposed" | "accepted" | "disputed")
}

const RUN_CHECKS: &str = "('command', 'http', 'file_exists', 'file_contains', 'file_sha256')";

pub fn now() -> String {
    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

async fn event(conn: &mut SqliteConnection, fact_id: &str, kind: &str, actor: &str, detail: Option<&str>) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO fact_events (fact_id, kind, actor, detail, created_at) VALUES (?, ?, ?, ?, ?)")
        .bind(fact_id)
        .bind(kind)
        .bind(actor)
        .bind(detail)
        .bind(now())
        .execute(conn)
        .await?;
    Ok(())
}

async fn restore(conn: &mut SqliteConnection) -> Result<(), sqlx::Error> {
    for sql in RESTORE {
        sqlx::query(sql).execute(&mut *conn).await?;
    }
    Ok(())
}

/// Open disputes with a side no longer in force (removed, lost elsewhere) are over: the side
/// still in force wins without anyone deciding (`resolved_by` stays NULL).
async fn settle(conn: &mut SqliteConnection) -> Result<u64, sqlx::Error> {
    let res = sqlx::query(&format!(
        "UPDATE fact_disputes SET status = 'resolved', resolved_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), \
           winner = (SELECT f.id FROM knowledge_facts f WHERE f.id IN (fact_disputes.fact_a, fact_disputes.fact_b) AND {IN_FORCE} \
                     ORDER BY f.id LIMIT 1) \
         WHERE status = 'open' AND EXISTS (SELECT 1 FROM knowledge_facts f WHERE f.id IN (fact_disputes.fact_a, fact_disputes.fact_b) \
           AND NOT ({IN_FORCE}))"
    ))
    .execute(&mut *conn)
    .await?;
    if res.rows_affected() > 0 {
        restore(conn).await?;
    }
    Ok(res.rows_affected())
}

impl Database {
    pub(super) async fn migrate_truth_ledger(&self) -> Result<(), DbError> {
        for stmt in TABLES {
            sqlx::query(stmt).execute(&self.pool).await?;
        }
        // The mirror needs memory's `imported`/`pinned_by` and the bots' spaces; a database from
        // before spaces (migration tests) gets them on its next start.
        let cols: Vec<(String,)> = sqlx::query_as(
            "SELECT name FROM pragma_table_info('kwants') WHERE name = 'space_id' \
             UNION ALL SELECT name FROM pragma_table_info('kwant_memories') WHERE name IN ('imported', 'pinned_by')",
        )
        .fetch_all(&self.pool)
        .await?;
        if cols.len() < 3 {
            return Ok(());
        }
        // Recreated on every start, in one transaction, so a database an earlier build migrated
        // gets this build's rules and no memory write slips between drop and create.
        let mut tx = self.pool.begin().await?;
        for t in triggers() {
            let name = t.split_whitespace().nth(5).unwrap_or_default();
            sqlx::query(&format!("DROP TRIGGER IF EXISTS {name}")).execute(&mut *tx).await?;
            sqlx::query(&t).execute(&mut *tx).await?;
        }
        sqlx::query("DROP VIEW IF EXISTS bot_memory_v").execute(&mut *tx).await?;
        sqlx::query(MEMORY_VIEW).execute(&mut *tx).await?;
        tx.commit().await?;
        self.backfill_truth_ledger().await?;
        Ok(())
    }

    /// Mirrors every memory that has no fact at all (so a rejected or removed one isn't brought
    /// back). Idempotent. Notes from before the ledger can't say whether the bot had read outside
    /// text, so a bot's own note is `bot`/`accepted` and an imported one `outside`/`proposed`.
    pub async fn backfill_truth_ledger(&self) -> Result<u64, DbError> {
        let imports: Option<(String,)> = sqlx::query_as("SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'imports'")
            .fetch_optional(&self.pool)
            .await?;
        let import_ref = if imports.is_some() {
            "(SELECT i.id FROM imports i, json_each(i.created, '$.kwants') k \
              WHERE json_extract(k.value, '$.id') = m.kwant_id ORDER BY i.created_at DESC LIMIT 1)"
        } else {
            "NULL"
        };
        let now = "strftime('%Y-%m-%dT%H:%M:%fZ', 'now')";
        let mut tx = self.pool.begin().await?;
        let res = sqlx::query(&format!(
            "INSERT INTO knowledge_facts (id, space_id, scope, scope_id, statement, source_kind, source_ref, author_kwant_id, \
                 trust, status, valid_from, memory_id, pinned, decided_by, decided_at, created_at, updated_at) \
             SELECT 'f_' || lower(hex(randomblob(12))), COALESCE(k.space_id, 'spc_default'), 'bot', m.kwant_id, m.text, \
                 CASE WHEN m.imported = 1 THEN 'import' WHEN m.source = 'user' THEN 'person' ELSE 'bot' END, \
                 CASE WHEN m.imported = 1 THEN {import_ref} END, \
                 CASE WHEN m.source = 'agent' THEN m.kwant_id END, \
                 CASE WHEN m.imported = 1 THEN 'outside' WHEN m.source = 'user' THEN 'person' ELSE 'bot' END, \
                 CASE WHEN m.imported = 1 THEN 'proposed' ELSE 'accepted' END, \
                 COALESCE(m.created_at, {now}), m.id, m.pinned_by IS NOT NULL, \
                 CASE WHEN m.imported = 0 AND m.source = 'user' THEN 'person' END, \
                 CASE WHEN m.imported = 0 AND m.source = 'user' THEN {now} END, {now}, {now} \
             FROM kwant_memories m LEFT JOIN kwants k ON k.id = m.kwant_id \
             WHERE NOT EXISTS (SELECT 1 FROM knowledge_facts f WHERE f.memory_id = m.id) \
             ORDER BY m.created_at, m.rowid"
        ))
        .execute(&mut *tx)
        .await?;
        sqlx::query(&format!(
            "INSERT INTO fact_events (fact_id, kind, actor, created_at) \
             SELECT f.id, 'backfilled', 'system', {now} FROM knowledge_facts f \
             WHERE NOT EXISTS (SELECT 1 FROM fact_events e WHERE e.fact_id = f.id)"
        ))
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(res.rows_affected())
    }

    /// Facts in `space`, newest first, at most [`MAX_FACTS`].
    pub async fn list_facts(&self, space: &str, f: &FactFilter<'_>) -> Result<Vec<Fact>, DbError> {
        let mut sql = String::from("SELECT * FROM knowledge_facts WHERE space_id = ?");
        if f.scope.is_some() {
            sql.push_str(" AND scope = ?");
        }
        if f.scope_id.is_some() {
            sql.push_str(" AND scope_id = ?");
        }
        if f.status.is_some() {
            sql.push_str(" AND status = ?");
        }
        if f.q.is_some() {
            sql.push_str(" AND statement LIKE '%' || ? || '%' ESCAPE '\\'");
        }
        sql.push_str(" ORDER BY created_at DESC, rowid DESC LIMIT ?");
        let mut q = sqlx::query_as::<_, Fact>(&sql).bind(space);
        for v in [f.scope, f.scope_id, f.status].into_iter().flatten() {
            q = q.bind(v);
        }
        if let Some(text) = f.q {
            q = q.bind(text.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_"));
        }
        Ok(q.bind(MAX_FACTS).fetch_all(&self.pool).await?)
    }

    pub async fn get_fact(&self, space: &str, id: &str) -> Result<Option<Fact>, DbError> {
        Ok(sqlx::query_as::<_, Fact>("SELECT * FROM knowledge_facts WHERE id = ? AND space_id = ?")
            .bind(id)
            .bind(space)
            .fetch_optional(&self.pool)
            .await?)
    }

    /// A fact's history, oldest first.
    pub async fn fact_events(&self, id: &str) -> Result<Vec<FactEvent>, DbError> {
        Ok(sqlx::query_as::<_, FactEvent>("SELECT id, kind, actor, detail, created_at FROM fact_events WHERE fact_id = ? ORDER BY id")
            .bind(id)
            .fetch_all(&self.pool)
            .await?)
    }

    /// The person accepting (`accept`) or rejecting a `proposed` fact: claim-first, so of racing
    /// calls exactly one changes it. Rejecting supersedes it with no successor; in shadow mode the
    /// memory row itself is not touched.
    pub async fn decide_fact(&self, space: &str, id: &str, accept: bool) -> Result<Decision, DbError> {
        let now = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let mut tx = self.pool.begin().await?;
        let res = sqlx::query(
            "UPDATE knowledge_facts SET status = ?, decided_by = 'person', decided_at = ?, updated_at = ? \
             WHERE id = ? AND space_id = ? AND status = 'proposed'",
        )
        .bind(if accept { "accepted" } else { "superseded" })
        .bind(&now)
        .bind(&now)
        .bind(id)
        .bind(space)
        .execute(&mut *tx)
        .await?;
        if res.rows_affected() == 0 {
            let status: Option<(String,)> = sqlx::query_as("SELECT status FROM knowledge_facts WHERE id = ? AND space_id = ?")
                .bind(id)
                .bind(space)
                .fetch_optional(&mut *tx)
                .await?;
            return Ok(status.map_or(Decision::NotFound, |(s,)| Decision::NotProposed(s)));
        }
        sqlx::query("INSERT INTO fact_events (fact_id, kind, actor, created_at) VALUES (?, ?, 'person', ?)")
            .bind(id)
            .bind(if accept { "accepted" } else { "rejected" })
            .bind(&now)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(Decision::Done)
    }
}

// C2: memory as a view, shared facts, disputes, evidence, citations

impl Database {
    /// The bot's notes the ledger keeps in force (`bot_memory_v`), keyed by memory id elsewhere.
    pub async fn bot_memory_facts(&self, kwant_id: &str) -> Result<Vec<Fact>, DbError> {
        Ok(sqlx::query_as::<_, Fact>("SELECT * FROM bot_memory_v WHERE scope_id = ? ORDER BY created_at, id")
            .bind(kwant_id)
            .fetch_all(&self.pool)
            .await?)
    }

    /// Teams the bot leads or belongs to.
    pub async fn teams_of_kwant(&self, kwant_id: &str) -> Result<Vec<String>, DbError> {
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT t.id FROM teams t WHERE t.lead_kwant_id = ?1 \
             OR EXISTS (SELECT 1 FROM json_each(CASE WHEN json_valid(t.member_kwants) THEN t.member_kwants ELSE '[]' END) j WHERE j.value = ?1) \
             ORDER BY t.id",
        )
        .bind(kwant_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(|r| r.0).collect())
    }

    /// Team facts of `teams` and space facts of `space` in force, oldest first; only `accepted`
    /// ones with `accepted_only` (what a bot's prompt shows).
    pub async fn shared_facts(&self, space: &str, teams: &[String], accepted_only: bool) -> Result<Vec<Fact>, DbError> {
        let status = if accepted_only { " AND f.status = 'accepted'" } else { "" };
        Ok(sqlx::query_as::<_, Fact>(&format!(
            "SELECT f.* FROM knowledge_facts f WHERE f.space_id = ? AND {IN_FORCE}{status} \
             AND (f.scope = 'space' OR (f.scope = 'team' AND f.scope_id IN (SELECT value FROM json_each(?)))) \
             ORDER BY f.created_at, f.rowid LIMIT ?"
        ))
        .bind(space)
        .bind(serde_json::to_string(teams).unwrap_or_else(|_| "[]".into()))
        .bind(MAX_FACTS)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn get_fact_any(&self, id: &str) -> Result<Option<Fact>, DbError> {
        Ok(sqlx::query_as::<_, Fact>("SELECT * FROM knowledge_facts WHERE id = ?").bind(id).fetch_optional(&self.pool).await?)
    }

    /// Facts by id, any space (for labels and dispute sides).
    pub async fn facts_by_ids(&self, ids: &[String]) -> Result<Vec<Fact>, DbError> {
        Ok(sqlx::query_as::<_, Fact>("SELECT * FROM knowledge_facts WHERE id IN (SELECT value FROM json_each(?))")
            .bind(serde_json::to_string(ids).unwrap_or_else(|_| "[]".into()))
            .fetch_all(&self.pool)
            .await?)
    }

    /// Every dispute (any status) touching one of `ids`, newest first.
    pub async fn disputes_of(&self, ids: &[String]) -> Result<Vec<Dispute>, DbError> {
        Ok(sqlx::query_as::<_, Dispute>(
            "SELECT * FROM fact_disputes WHERE fact_a IN (SELECT value FROM json_each(?1)) OR fact_b IN (SELECT value FROM json_each(?1)) \
             ORDER BY created_at DESC, rowid DESC",
        )
        .bind(serde_json::to_string(ids).unwrap_or_else(|_| "[]".into()))
        .fetch_all(&self.pool)
        .await?)
    }

    /// Disputes in `space`, newest first; open ones only by default.
    pub async fn list_disputes(&self, space: &str, status: Option<&str>) -> Result<Vec<Dispute>, DbError> {
        Ok(sqlx::query_as::<_, Dispute>(
            "SELECT d.* FROM fact_disputes d JOIN knowledge_facts f ON f.id = d.fact_a \
             WHERE f.space_id = ? AND (? IS NULL OR d.status = ?) ORDER BY d.created_at DESC, d.rowid DESC LIMIT ?",
        )
        .bind(space)
        .bind(status)
        .bind(status)
        .bind(MAX_FACTS)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn get_dispute(&self, space: &str, id: &str) -> Result<Option<Dispute>, DbError> {
        Ok(sqlx::query_as::<_, Dispute>(
            "SELECT d.* FROM fact_disputes d JOIN knowledge_facts f ON f.id = d.fact_a WHERE d.id = ? AND f.space_id = ?",
        )
        .bind(id)
        .bind(space)
        .fetch_optional(&self.pool)
        .await?)
    }

    /// Opens a dispute between two facts in force in the same space, unless the pair ever had
    /// one (a dismissed pair is never raised again). Both sides go `disputed`, except a fact the
    /// person decided, which keeps its status until the person looks. `None` when nothing opened.
    pub async fn open_dispute(&self, a: &str, b: &str, cause: &str, actor: &str) -> Result<Option<Dispute>, DbError> {
        if a == b {
            return Ok(None);
        }
        let (a, b) = if a < b { (a, b) } else { (b, a) };
        let mut tx = self.pool.begin().await?;
        let (live,): (i64,) = sqlx::query_as(&format!(
            "SELECT COUNT(DISTINCT f.space_id) * 10 + COUNT(*) FROM knowledge_facts f WHERE f.id IN (?, ?) AND {IN_FORCE}"
        ))
        .bind(a)
        .bind(b)
        .fetch_one(&mut *tx)
        .await?;
        // Both in force (2) and in one space (10)
        if live != 12 {
            return Ok(None);
        }
        let id = format!("fd_{}", uuid::Uuid::new_v4().simple());
        let at = now();
        let res = sqlx::query(
            "INSERT OR IGNORE INTO fact_disputes (id, fact_a, fact_b, cause, status, created_at) VALUES (?, ?, ?, ?, 'open', ?)",
        )
        .bind(&id)
        .bind(a)
        .bind(b)
        .bind(cause)
        .bind(&at)
        .execute(&mut *tx)
        .await?;
        if res.rows_affected() == 0 {
            return Ok(None);
        }
        for side in [a, b] {
            sqlx::query(
                "UPDATE knowledge_facts SET status = 'disputed', updated_at = ? \
                 WHERE id = ? AND status IN ('proposed', 'accepted') AND decided_by IS NOT 'person'",
            )
            .bind(&at)
            .bind(side)
            .execute(&mut *tx)
            .await?;
            event(&mut tx, side, "disputed", actor, Some(&id)).await?;
        }
        tx.commit().await?;
        Ok(sqlx::query_as::<_, Dispute>("SELECT * FROM fact_disputes WHERE id = ?").bind(&id).fetch_optional(&self.pool).await?)
    }

    /// Settles an open dispute for `winner` (`by` is `person` or `evidence`): claim-first, so of
    /// racing calls exactly one changes it. The loser is superseded by the winner; the winner is
    /// accepted (a shared fact only when the person decides, and it stays `disputed` while another
    /// dispute is open on it). Callers check the evidence rules before `by = "evidence"`.
    pub async fn resolve_dispute(&self, space: &str, id: &str, winner: &str, by: &str) -> Result<DisputeDecision, DbError> {
        let Some(d) = self.get_dispute(space, id).await? else { return Ok(DisputeDecision::NotFound) };
        let loser = if winner == d.fact_a {
            d.fact_b.clone()
        } else if winner == d.fact_b {
            d.fact_a.clone()
        } else {
            return Ok(DisputeDecision::BadWinner);
        };
        let at = now();
        let mut tx = self.pool.begin().await?;
        let res = sqlx::query(
            "UPDATE fact_disputes SET status = 'resolved', winner = ?, resolved_by = ?, resolved_at = ? WHERE id = ? AND status = 'open'",
        )
        .bind(winner)
        .bind(by)
        .bind(&at)
        .bind(id)
        .execute(&mut *tx)
        .await?;
        if res.rows_affected() == 0 {
            let (status,): (String,) = sqlx::query_as("SELECT status FROM fact_disputes WHERE id = ?").bind(id).fetch_one(&mut *tx).await?;
            return Ok(DisputeDecision::NotOpen(status));
        }
        let lost = sqlx::query(&format!(
            "UPDATE knowledge_facts SET status = 'superseded', superseded_by = ?, decided_by = ?, decided_at = ?, updated_at = ? \
             WHERE id = ? AND status IN {LIVE}"
        ))
        .bind(winner)
        .bind(by)
        .bind(&at)
        .bind(&at)
        .bind(&loser)
        .execute(&mut *tx)
        .await?;
        if lost.rows_affected() > 0 {
            event(&mut tx, &loser, "superseded", by, Some(id)).await?;
        }
        sqlx::query(&format!(
            "UPDATE knowledge_facts SET \
               decided_by = CASE WHEN decided_by = 'person' THEN 'person' ELSE ?1 END, decided_at = ?2, updated_at = ?2, \
               status = CASE \
                 WHEN ?1 <> 'person' AND decided_by IS NOT 'person' AND scope <> 'bot' THEN 'proposed' \
                 WHEN ?1 <> 'person' AND decided_by IS NOT 'person' AND EXISTS (SELECT 1 FROM fact_disputes d WHERE d.status = 'open' \
                   AND (d.fact_a = knowledge_facts.id OR d.fact_b = knowledge_facts.id)) THEN 'disputed' \
                 ELSE 'accepted' END \
             WHERE id = ?3 AND status IN {LIVE}"
        ))
        .bind(by)
        .bind(&at)
        .bind(winner)
        .execute(&mut *tx)
        .await?;
        event(&mut tx, winner, "won", by, Some(id)).await?;
        settle(&mut tx).await?;
        restore(&mut tx).await?;
        tx.commit().await?;
        let d = self.get_dispute(space, id).await?.ok_or(DbError::Sqlx(sqlx::Error::RowNotFound))?;
        Ok(DisputeDecision::Done(Box::new(d)))
    }

    /// The person saying "Not a conflict": claim-first; both sides go back to where they were and
    /// the pair is never raised again.
    pub async fn dismiss_dispute(&self, space: &str, id: &str) -> Result<DisputeDecision, DbError> {
        let Some(d) = self.get_dispute(space, id).await? else { return Ok(DisputeDecision::NotFound) };
        let at = now();
        let mut tx = self.pool.begin().await?;
        let res = sqlx::query("UPDATE fact_disputes SET status = 'dismissed', resolved_by = 'person', resolved_at = ? WHERE id = ? AND status = 'open'")
            .bind(&at)
            .bind(id)
            .execute(&mut *tx)
            .await?;
        if res.rows_affected() == 0 {
            let (status,): (String,) = sqlx::query_as("SELECT status FROM fact_disputes WHERE id = ?").bind(id).fetch_one(&mut *tx).await?;
            return Ok(DisputeDecision::NotOpen(status));
        }
        for side in [&d.fact_a, &d.fact_b] {
            event(&mut tx, side, "dispute_dismissed", "person", Some(id)).await?;
        }
        restore(&mut tx).await?;
        tx.commit().await?;
        let d = self.get_dispute(space, id).await?.ok_or(DbError::Sqlx(sqlx::Error::RowNotFound))?;
        Ok(DisputeDecision::Done(Box::new(d)))
    }

    /// Closes open disputes that lost a side (a removed note, a superseded fact). Returns how many.
    pub async fn settle_disputes(&self) -> Result<u64, DbError> {
        let mut tx = self.pool.begin().await?;
        let n = settle(&mut tx).await?;
        tx.commit().await?;
        Ok(n)
    }

    /// Attaches evidence to a fact; `false` when that piece was already attached.
    pub async fn add_fact_evidence(
        &self,
        fact_id: &str,
        evidence_id: Option<&str>,
        check_id: Option<&str>,
        message_id: Option<&str>,
        added_by: &str,
    ) -> Result<bool, DbError> {
        let mut tx = self.pool.begin().await?;
        let res = sqlx::query(
            "INSERT OR IGNORE INTO fact_evidence (fact_id, evidence_id, check_id, message_id, added_by, added_at) VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(fact_id)
        .bind(evidence_id)
        .bind(check_id)
        .bind(message_id)
        .bind(added_by)
        .bind(now())
        .execute(&mut *tx)
        .await?;
        if res.rows_affected() > 0 {
            let what = check_id.map(|c| format!("check {c}")).or(evidence_id.map(|e| format!("evidence {e}"))).or(message_id.map(|m| format!("message {m}")));
            event(&mut tx, fact_id, "evidence_added", added_by, what.as_deref()).await?;
        }
        tx.commit().await?;
        Ok(res.rows_affected() > 0)
    }

    pub async fn fact_evidence(&self, fact_id: &str) -> Result<Vec<FactEvidence>, DbError> {
        Ok(sqlx::query_as::<_, FactEvidence>("SELECT * FROM fact_evidence WHERE fact_id = ? ORDER BY added_at, rowid")
            .bind(fact_id)
            .fetch_all(&self.pool)
            .await?)
    }

    /// `(passed, failed)` kwant-run checks attached to the fact. Message and file evidence never
    /// count (contract §5 rule 3).
    pub async fn check_strength(&self, fact_id: &str) -> Result<(i64, i64), DbError> {
        let (passed, failed): (i64, i64) = sqlx::query_as(&format!(
            "SELECT COALESCE(SUM(c.status = 'passed'), 0), COALESCE(SUM(c.status IN ('failed', 'error')), 0) \
             FROM fact_evidence e JOIN outcome_checks c ON c.id = e.check_id \
             WHERE e.fact_id = ? AND c.kind IN {RUN_CHECKS} AND c.approved = 1"
        ))
        .bind(fact_id)
        .fetch_one(&self.pool)
        .await?;
        Ok((passed, failed))
    }

    /// `task` or `fact` `by_id` relies on `fact_id`; `false` when it already did.
    pub async fn cite_fact(&self, fact_id: &str, kind: &str, by_id: &str, actor: &str) -> Result<bool, DbError> {
        let mut tx = self.pool.begin().await?;
        let res = sqlx::query("INSERT OR IGNORE INTO fact_citations (fact_id, cited_by_kind, cited_by_id, created_at) VALUES (?, ?, ?, ?)")
            .bind(fact_id)
            .bind(kind)
            .bind(by_id)
            .bind(now())
            .execute(&mut *tx)
            .await?;
        if res.rows_affected() > 0 {
            event(&mut tx, fact_id, "cited", actor, Some(&format!("{kind} {by_id}"))).await?;
        }
        tx.commit().await?;
        Ok(res.rows_affected() > 0)
    }

    /// The person saying a task no longer relies on a fact (clears its `fact_changed` item).
    pub async fn uncite_fact(&self, fact_id: &str, task_id: &str) -> Result<bool, DbError> {
        let mut tx = self.pool.begin().await?;
        let res = sqlx::query("DELETE FROM fact_citations WHERE fact_id = ? AND cited_by_kind = 'task' AND cited_by_id = ?")
            .bind(fact_id)
            .bind(task_id)
            .execute(&mut *tx)
            .await?;
        if res.rows_affected() > 0 {
            event(&mut tx, fact_id, "uncited", "person", Some(&format!("task {task_id}"))).await?;
        }
        tx.commit().await?;
        Ok(res.rows_affected() > 0)
    }

    /// A note on a fact's history (`dispute_reason`: why a bot disputed it).
    pub async fn add_fact_note(&self, fact_id: &str, kind: &str, actor: &str, detail: &str) -> Result<(), DbError> {
        let mut conn = self.pool.acquire().await?;
        event(&mut conn, fact_id, kind, actor, Some(detail)).await?;
        Ok(())
    }

    pub async fn citations(&self, fact_id: &str) -> Result<Vec<Citation>, DbError> {
        Ok(sqlx::query_as::<_, Citation>("SELECT * FROM fact_citations WHERE fact_id = ? ORDER BY created_at, rowid")
            .bind(fact_id)
            .fetch_all(&self.pool)
            .await?)
    }

    /// Facts a task cites that are `disputed`, or were superseded after it cited them.
    pub async fn changed_facts_of_task(&self, task_id: &str) -> Result<Vec<Fact>, DbError> {
        Ok(sqlx::query_as::<_, Fact>(
            "SELECT f.* FROM fact_citations c JOIN knowledge_facts f ON f.id = c.fact_id \
             WHERE c.cited_by_kind = 'task' AND c.cited_by_id = ? \
               AND (f.status = 'disputed' OR (f.status = 'superseded' AND f.updated_at >= c.created_at)) \
             ORDER BY c.created_at, c.rowid",
        )
        .bind(task_id)
        .fetch_all(&self.pool)
        .await?)
    }

    /// Writes a fact that isn't a memory: the person's own (accepted) or a bot's shared proposal.
    pub async fn insert_fact(&self, f: &NewFact<'_>) -> Result<Fact, DbError> {
        let id = format!("f_{}", &uuid::Uuid::new_v4().simple().to_string()[..24]);
        let at = now();
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "INSERT INTO knowledge_facts (id, space_id, scope, scope_id, statement, subject, source_kind, source_ref, author_kwant_id, \
                 trust, status, valid_from, pinned, decided_by, decided_at, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 0, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(f.space_id)
        .bind(f.scope)
        .bind(f.scope_id)
        .bind(f.statement)
        .bind(f.subject)
        .bind(f.source_kind)
        .bind(f.source_ref)
        .bind(f.author_kwant_id)
        .bind(f.trust)
        .bind(if f.decided_by_person { "accepted" } else { "proposed" })
        .bind(&at)
        .bind(f.decided_by_person.then_some("person"))
        .bind(f.decided_by_person.then_some(at.as_str()))
        .bind(&at)
        .bind(&at)
        .execute(&mut *tx)
        .await?;
        event(&mut tx, &id, "created", f.actor, None).await?;
        tx.commit().await?;
        self.get_fact_any(&id).await?.ok_or(DbError::Sqlx(sqlx::Error::RowNotFound))
    }

    /// Sets the subject of a fact in force (lowercased, single-spaced); `None` clears it.
    pub async fn set_fact_subject(&self, fact_id: &str, subject: Option<&str>) -> Result<(), DbError> {
        sqlx::query(&format!("UPDATE knowledge_facts SET subject = ?, updated_at = ? WHERE id = ? AND status IN {LIVE}"))
            .bind(subject)
            .bind(now())
            .bind(fact_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// The person ending a fact at `until` (now when `None`); it drops out of prompts from then.
    pub async fn expire_fact(&self, space: &str, id: &str, until: Option<&str>) -> Result<Option<Fact>, DbError> {
        let until = until.map(str::to_string).unwrap_or_else(now);
        let mut tx = self.pool.begin().await?;
        let res = sqlx::query(&format!("UPDATE knowledge_facts SET valid_until = ?, updated_at = ? WHERE id = ? AND space_id = ? AND status IN {LIVE}"))
            .bind(&until)
            .bind(now())
            .bind(id)
            .bind(space)
            .execute(&mut *tx)
            .await?;
        if res.rows_affected() > 0 {
            event(&mut tx, id, "expires", "person", Some(&until)).await?;
        }
        tx.commit().await?;
        if res.rows_affected() == 0 {
            return Ok(None);
        }
        self.get_fact(space, id).await
    }

    /// The person sharing a fact with a team or the space: a new accepted fact in that scope
    /// whose source is the old one (which stays as it is).
    pub async fn promote_fact(&self, space: &str, id: &str, scope: &str, scope_id: &str) -> Result<Option<Fact>, DbError> {
        let Some(old) = self.get_fact(space, id).await? else { return Ok(None) };
        if !LIVE.contains(&format!("'{}'", old.status)) {
            return Ok(None);
        }
        let new = self
            .insert_fact(&NewFact {
                space_id: space,
                scope,
                scope_id,
                statement: &old.statement,
                subject: old.subject.as_deref(),
                source_kind: &old.source_kind,
                source_ref: Some(&old.id),
                author_kwant_id: old.author_kwant_id.as_deref(),
                trust: &old.trust,
                decided_by_person: true,
                actor: "person",
            })
            .await?;
        let mut conn = self.pool.acquire().await?;
        event(&mut conn, &old.id, "promoted", "person", Some(&new.id)).await?;
        Ok(Some(new))
    }
}
