//! Reading a letter and turning it into its case: the pipeline (PDF text first, then one model
//! call for whatever has no text), the case's fields from what was read, the person's corrections,
//! and the reminders that follow a confirmed deadline.

use super::reader::{LetterReader, Page};
use super::{check, parse_amount, parse_answer, parse_date, prompt, Field, Fields, INSTRUCTIONS, MAX_LIST, MAX_VALUE_CHARS};
use crate::db::documents::ExtractionRow;
use crate::db::zombie::CaseRow;
use crate::db::Database;
use crate::zombie::{clip, CaseKind, Evidence, SourceKind};
use chrono::{DateTime, Utc};
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::{Mutex, OwnedMutexGuard};

/// The model sees images up to this size (the Messages API limit).
pub const MAX_IMAGE_BYTES: usize = 5 * 1024 * 1024;
/// A PDF with less text than this is a scan: its pages go to the model.
const MIN_TEXT_CHARS: usize = 20;
/// Most transcript text kept from the model.
const MAX_TRANSCRIPT_CHARS: usize = 50_000;

/// One letter read at a time, and (in tests) the reader to use instead of a real model.
#[derive(Default)]
pub struct Desk {
    lock: Arc<Mutex<()>>,
    pub reader: Option<Arc<dyn LetterReader>>,
}

impl Desk {
    pub fn with_reader(reader: Arc<dyn LetterReader>) -> Self {
        Desk { lock: Default::default(), reader: Some(reader) }
    }

    pub fn busy(&self) -> bool {
        self.lock.try_lock().is_err()
    }

    /// The right to read, or `None` while another letter is being read.
    pub fn try_begin(&self) -> Option<OwnedMutexGuard<()>> {
        self.lock.clone().try_lock_owned().ok()
    }
}

/// One page as stored: its kind and bytes.
#[derive(Debug, Clone)]
pub struct StoredPage {
    pub mime: String,
    pub bytes: Vec<u8>,
}

/// What reading a letter gave.
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    /// `read`, `text_only` or `unreadable`.
    pub status: &'static str,
    /// `pdf_text`, `model_vision`, `pdf_text+model_vision`, or `None` when nothing was read.
    pub method: Option<&'static str>,
    pub note: Option<String>,
    pub text: String,
    pub fields: Fields,
    pub read_by: Option<String>,
}

/// Reads the pages: the PDF text layer first, then one model call (if `reader` can be had) with
/// the text and every page that has none. Never fails: without a model the case is `text_only`
/// or `unreadable` and the person fills the fields in.
pub async fn read_pages(pages: Vec<StoredPage>, reader: Result<Arc<dyn LetterReader>, String>) -> Outcome {
    let mut text = String::new();
    let mut visual: Vec<Page> = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    for (i, p) in pages.into_iter().enumerate() {
        let n = i + 1;
        if p.mime == "application/pdf" {
            match crate::server::attachments::pdf_pages(p.bytes.clone()).await {
                Ok(t) if t.iter().map(|s| s.trim().chars().count()).sum::<usize>() >= MIN_TEXT_CHARS => {
                    for page in t {
                        let page = page.trim();
                        if !page.is_empty() {
                            if !text.is_empty() {
                                text.push_str("\n\n");
                            }
                            text.push_str(page);
                        }
                    }
                    continue;
                }
                Ok(_) => {}
                Err(e) => notes.push(format!("page {n}: the PDF text could not be read ({e})")),
            }
        }
        if p.bytes.len() > MAX_IMAGE_BYTES {
            notes.push(format!("page {n} is over 5 MB, too large for the model to see"));
            continue;
        }
        visual.push(Page { mime: p.mime, bytes: p.bytes });
    }
    let has_text = !text.trim().is_empty();
    let fallback = |why: String, notes: Vec<String>, text: String| {
        let mut notes = notes;
        notes.push(why);
        if has_text {
            Outcome { status: "text_only", method: Some("pdf_text"), note: Some(notes.join("; ")), text, fields: Fields::default(), read_by: None }
        } else {
            Outcome { status: "unreadable", method: None, note: Some(notes.join("; ")), text, fields: Fields::default(), read_by: None }
        }
    };
    if !has_text && visual.is_empty() {
        return fallback("no text could be read from these pages: fill in the fields yourself".into(), notes, text);
    }
    let reader = match reader {
        Ok(r) => r,
        Err(why) => return fallback(format!("no model that can read letters is set up ({why}): fill in the fields yourself"), notes, text),
    };
    let answer = match reader.read(INSTRUCTIONS, &prompt(&text, visual.len()), &visual).await {
        Ok(a) => a,
        Err(e) => return fallback(format!("the model could not read it ({e}): fill in the fields yourself"), notes, text),
    };
    let Some(raw) = parse_answer(&answer) else {
        return fallback("the model's answer was not the JSON asked for: fill in the fields yourself".into(), notes, text);
    };
    // What the model read on the images counts as the letter's text only when it saw images
    let mut all = text.clone();
    if !visual.is_empty() && !raw.transcript.trim().is_empty() {
        if !all.is_empty() {
            all.push_str("\n\n");
        }
        all.push_str(&clip(raw.transcript.trim(), MAX_TRANSCRIPT_CHARS));
    }
    let fields = check(raw, &all);
    let method = match (has_text, !visual.is_empty()) {
        (true, true) => "pdf_text+model_vision",
        (true, false) => "pdf_text",
        _ => "model_vision",
    };
    Outcome {
        status: "read",
        method: Some(method),
        note: (!notes.is_empty()).then(|| notes.join("; ")),
        text: all,
        fields,
        read_by: Some(reader.name()),
    }
}

/// A new letter case, open, while it is being read.
pub fn new_case(id: &str, now: DateTime<Utc>) -> CaseRow {
    CaseRow {
        id: id.into(),
        dedupe_key: format!("{}:{}:{id}", SourceKind::Document.as_str(), SourceKind::Document.as_str()),
        source: SourceKind::Document.as_str().into(),
        source_id: SourceKind::Document.as_str().into(),
        source_ref: format!("document:{id}"),
        kind: CaseKind::DocumentRequest.as_str().into(),
        conv_kind: "document".into(),
        title: "Letter (reading…)".into(),
        counterpart: None,
        counterpart_name: None,
        reference: None,
        link: None,
        why: "A letter to deal with".into(),
        evidence: "[]".into(),
        draft: None,
        status: "open".into(),
        snoozed_until: None,
        waiting_since: None,
        follow_ups: 0,
        first_seen: now,
        last_seen: now,
        last_activity_at: now,
        status_changed_at: now,
        resolution: None,
        dismiss_reason: None,
        deadline: None,
        reply_to_message_id: None,
        space_id: None,
        redacted: false,
        kwant_id: None,
    }
}

/// An email address the draft can go to: one `@`, no spaces.
fn email_of(s: &str) -> Option<String> {
    let s = s.trim().trim_start_matches("mailto:");
    (s.matches('@').count() == 1 && !s.contains(char::is_whitespace) && s.len() <= 254 && !s.starts_with('@') && !s.ends_with('@'))
        .then(|| s.to_string())
}

/// The case's own fields from the letter's: title, who, reference, why, evidence, the deadline
/// (only a confirmed one) and the template draft (a model draft the person asked for stays).
pub fn apply(c: &mut CaseRow, f: &Fields, now: DateTime<Utc>) {
    let sender = f.sender.as_ref().map(|s| clip(s.value.trim(), 120)).filter(|s| !s.is_empty());
    c.title = match &sender {
        Some(s) => format!("Letter from {s}"),
        None => "Letter".into(),
    };
    c.counterpart_name = sender.clone();
    c.counterpart = f.reply_to.as_ref().and_then(|r| email_of(&r.value));
    c.reference = f.references.first().map(|r| clip(r.value.trim(), 120));
    c.why = f.request.as_ref().map(|r| clip(r.value.trim(), 300)).filter(|r| !r.is_empty()).unwrap_or_else(|| "A letter to deal with".into());
    let from = sender.unwrap_or_else(|| "the letter".into());
    let evidence: Vec<Evidence> = [&f.request, &f.deadline]
        .into_iter()
        .flatten()
        .filter(|x| !x.quote.is_empty())
        .map(|x| Evidence { quote: x.quote.clone(), from: from.clone(), at: c.first_seen, outbound: false })
        .collect();
    c.evidence = serde_json::to_string(&evidence).unwrap_or_else(|_| "[]".into());
    c.deadline = f.confirmed_deadline().map(|d| d.format("%Y-%m-%d").to_string());
    let model_draft = c
        .draft
        .as_deref()
        .and_then(|d| serde_json::from_str::<crate::zombie::draft::Draft>(d).ok())
        .is_some_and(|d| d.generated_by == "model");
    if !model_draft {
        c.draft = crate::zombie::hunt::with_draft_input(c, |i| serde_json::to_string(&crate::zombie::draft::template(i, now)).ok()).flatten();
    }
    c.last_seen = now;
}

/// Replaces a letter case's reminders: three for a confirmed deadline, none otherwise (or once
/// the case is closed or dismissed). Returns how many are scheduled.
pub async fn reschedule(db: &Database, c: &CaseRow, f: &Fields, now: DateTime<Utc>) -> Result<usize, crate::db::DbError> {
    let open = !c.redacted && !matches!(c.status.as_str(), "closed" | "dismissed");
    match f.confirmed_deadline().filter(|_| open) {
        Some(deadline) => {
            let tz = crate::zombie::settings::local_tz(db).await;
            let due = super::reminders::schedule(deadline, &tz, now);
            db.document_set_reminders(&c.id, &due).await?;
            Ok(due.len())
        }
        None => {
            db.document_clear_reminders(&c.id).await?;
            Ok(0)
        }
    }
}

/// After a status change on any case: a letter's reminders go when it closes or is dismissed and
/// come back when it reopens.
pub async fn after_status_change(db: &Database, c: &CaseRow) -> Result<(), crate::db::DbError> {
    if c.source != SourceKind::Document.as_str() {
        return Ok(());
    }
    let fields = match db.document_extraction(&c.id).await? {
        Some(e) => serde_json::from_str::<Fields>(&e.fields).unwrap_or_default(),
        None => Fields::default(),
    };
    reschedule(db, c, &fields, Utc::now()).await.map(|_| ())
}

/// The person's corrections. Each key is optional:
/// - `sender`, `request`, `reply_to`, `deadline`: `null` clears it, `{}` confirms it as it is,
///   `{"value": "..."}` sets it (confirmed).
/// - `references`, `amounts`, `checklist`: the whole list, each
///   `{"value": "...", "done"?: bool, "confirmed"?: true}` (`done` for checklist items). An entry
///   whose value was already there keeps its quote, and keeps its confirmed/uncertain state unless
///   it is sent with `"confirmed": true`; a new or changed entry is confirmed.
///
/// Every single field the person touches is confirmed and no longer uncertain. A deadline must be
/// `YYYY-MM-DD`; an amount must parse (`GBP 142.60`, `£142.60`, `142,60 EUR`).
pub fn patch(old: &Fields, body: &Value) -> Result<Fields, String> {
    let obj = body.as_object().ok_or("send a JSON object of fields")?;
    let mut f = old.clone();
    for (key, v) in obj {
        match key.as_str() {
            "sender" => f.sender = single(&old.sender, v, key, Kind::Text)?,
            "request" => f.request = single(&old.request, v, key, Kind::Text)?,
            "reply_to" => f.reply_to = single(&old.reply_to, v, key, Kind::Text)?,
            "deadline" => f.deadline = single(&old.deadline, v, key, Kind::Date)?,
            "references" => f.references = list(&old.references, v, key, Kind::Text)?,
            "amounts" => f.amounts = list(&old.amounts, v, key, Kind::Amount)?,
            "checklist" => f.checklist = list(&old.checklist, v, key, Kind::Checklist)?,
            other => return Err(format!("unknown field '{other}'")),
        }
    }
    Ok(f)
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Text,
    Date,
    Amount,
    Checklist,
}

fn valid(value: &str, key: &str, kind: Kind) -> Result<String, String> {
    let v = value.trim();
    if v.is_empty() {
        return Err(format!("{key}: the value is empty (send null to clear it)"));
    }
    if v.chars().count() > MAX_VALUE_CHARS {
        return Err(format!("{key}: at most {MAX_VALUE_CHARS} characters"));
    }
    match kind {
        Kind::Date => parse_date(v).map(|d| d.format("%Y-%m-%d").to_string()).ok_or_else(|| format!("{key}: give the date as YYYY-MM-DD")),
        Kind::Amount => parse_amount(v).ok_or_else(|| format!("{key}: give an amount with its currency, like GBP 142.60")),
        Kind::Text | Kind::Checklist => Ok(v.to_string()),
    }
}

fn single(old: &Option<Field>, v: &Value, key: &str, kind: Kind) -> Result<Option<Field>, String> {
    let obj = match v {
        Value::Null => return Ok(None),
        Value::Object(o) => o,
        _ => return Err(format!("{key}: send null, {{}} or {{\"value\": \"...\"}}")),
    };
    match obj.get("value") {
        None if obj.is_empty() => {
            let mut f = old.clone().ok_or_else(|| format!("{key}: there is nothing to confirm"))?;
            // Confirming as it is still needs a value that parses
            f.value = valid(&f.value, key, kind)?;
            f.confirmed = true;
            f.uncertain = false;
            Ok(Some(f))
        }
        Some(Value::String(s)) if obj.len() == 1 => {
            let value = valid(s, key, kind)?;
            let quote = old.as_ref().filter(|o| o.value == value).map(|o| o.quote.clone()).unwrap_or_default();
            Ok(Some(Field { value, quote, uncertain: false, confirmed: true, done: None }))
        }
        _ => Err(format!("{key}: send null, {{}} or {{\"value\": \"...\"}}")),
    }
}

fn list(old: &[Field], v: &Value, key: &str, kind: Kind) -> Result<Vec<Field>, String> {
    let items = v.as_array().ok_or_else(|| format!("{key}: send a list"))?;
    if items.len() > MAX_LIST {
        return Err(format!("{key}: at most {MAX_LIST} entries"));
    }
    items
        .iter()
        .map(|item| {
            let value = item.get("value").and_then(Value::as_str).ok_or_else(|| format!("{key}: each entry needs a value"))?;
            let value = valid(value, key, kind)?;
            let before = old.iter().find(|o| o.value == value);
            let confirm = match item.get("confirmed") {
                None | Some(Value::Bool(false)) => false,
                Some(Value::Bool(true)) => true,
                Some(_) => return Err(format!("{key}: confirmed is true or false")),
            };
            let done = match (kind, item.get("done")) {
                (Kind::Checklist, Some(Value::Bool(b))) => Some(*b),
                (Kind::Checklist, None) => Some(before.and_then(|b| b.done).unwrap_or(false)),
                (Kind::Checklist, Some(_)) => return Err(format!("{key}: done is true or false")),
                (_, Some(_)) => return Err(format!("{key}: only checklist entries can be done")),
                (_, None) => None,
            };
            // A new or changed entry, or one the person confirms, is confirmed; an unchanged one
            // (say, ticking another checklist item) keeps what it was
            let (uncertain, confirmed) = match before {
                Some(b) if !confirm => (b.uncertain, b.confirmed),
                _ => (false, true),
            };
            Ok(Field { quote: before.map(|b| b.quote.clone()).unwrap_or_default(), value, uncertain, confirmed, done })
        })
        .collect()
}

/// Reads the letter of `case_id` from its stored pages, saves what was read and updates the case.
/// Holds `_guard` (the one-at-a-time right) until done. Logs ids, counts and statuses only.
pub async fn run(state: crate::server::AppState, case_id: String, attachments: Vec<crate::db::attachments::Attachment>, _guard: OwnedMutexGuard<()>) {
    let root = crate::server::attachments::root(&state);
    let mut pages = Vec::new();
    for a in &attachments {
        let bytes = match crate::server::attachments::blob_path(&root, &a.sha256) {
            Some(p) => tokio::fs::read(p).await.ok(),
            None => None,
        };
        match bytes {
            Some(bytes) => pages.push(StoredPage { mime: a.mime.clone(), bytes }),
            None => tracing::warn!(case_id = %case_id, attachment_id = %a.id, "documents: a page's file is missing"),
        }
    }
    let page_count = pages.len();
    let reader = super::reader::reader_for(&state).await;
    let out = read_pages(pages, reader).await;
    let now = Utc::now();
    if let Err(e) = save(&state.db, &case_id, out.clone(), now).await {
        tracing::warn!(case_id = %case_id, "documents: could not save what was read: {e}");
        return;
    }
    tracing::info!(
        case_id = %case_id,
        pages = page_count,
        status = out.status,
        method = out.method.unwrap_or("none"),
        fields = out.fields.iter().count(),
        uncertain = out.fields.uncertain_count(),
        "documents: letter read"
    );
}

/// Stores `out` for `case_id` and brings the case up to date. A case deleted meanwhile stays gone.
///
/// The case and its reminders are written first and the extraction's status last: a poller that
/// sees the letter read always gets the updated case and its draft with it. If the case write
/// fails, the extraction stays `running` and the next start marks it interrupted.
pub async fn save(db: &Database, case_id: &str, out: Outcome, now: DateTime<Utc>) -> Result<(), crate::db::DbError> {
    let Some(mut e) = db.document_extraction(case_id).await? else { return Ok(()) };
    let Some(mut c) = db.zombie_case(case_id).await? else { return Ok(()) };
    apply(&mut c, &out.fields, now);
    db.zombie_put_case(&c).await?;
    reschedule(db, &c, &out.fields, now).await?;
    e.status = out.status.into();
    e.method = out.method.map(str::to_string);
    e.note = out.note;
    e.text = out.text;
    e.fields = serde_json::to_string(&out.fields).unwrap_or_else(|_| "{}".into());
    e.read_by = out.read_by;
    e.updated_at = now;
    db.document_put_extraction(&e).await?;
    Ok(())
}

/// The extraction row for a letter that is about to be read.
pub fn running_row(case_id: &str, attachment_ids: &[String], now: DateTime<Utc>) -> ExtractionRow {
    ExtractionRow {
        case_id: case_id.into(),
        status: "running".into(),
        method: None,
        note: None,
        text: String::new(),
        fields: serde_json::to_string(&Fields::default()).unwrap_or_else(|_| "{}".into()),
        attachment_ids: serde_json::to_string(attachment_ids).unwrap_or_else(|_| "[]".into()),
        read_by: None,
        created_at: now,
        updated_at: now,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct Fixed(String);

    #[async_trait::async_trait]
    impl LetterReader for Fixed {
        fn name(&self) -> String {
            "stub:fixed".into()
        }
        async fn read(&self, _: &str, _: &str, _: &[Page]) -> Result<String, String> {
            Ok(self.0.clone())
        }
    }

    fn png() -> StoredPage {
        StoredPage { mime: "image/png".into(), bytes: include_bytes!("../../tests/fixtures/letters/letter-page2.png").to_vec() }
    }

    fn pdf() -> StoredPage {
        StoredPage { mime: "application/pdf".into(), bytes: include_bytes!("../../tests/fixtures/letters/water-bill.pdf").to_vec() }
    }

    #[tokio::test]
    async fn an_image_with_no_model_is_unreadable_and_has_no_fields() {
        let out = read_pages(vec![png()], Err("no Claude engine".into())).await;
        assert_eq!(out.status, "unreadable");
        assert_eq!(out.method, None);
        assert_eq!(out.fields, Fields::default());
        assert!(out.note.unwrap().contains("fill in the fields yourself"));
    }

    #[tokio::test]
    async fn a_text_pdf_with_no_model_keeps_its_text_for_the_person() {
        let out = read_pages(vec![pdf()], Err("no Claude engine".into())).await;
        assert_eq!(out.status, "text_only");
        assert_eq!(out.method, Some("pdf_text"));
        assert!(out.text.contains("Account number: NW-448812"), "{}", out.text);
        assert_eq!(out.fields, Fields::default());
    }

    #[tokio::test]
    async fn a_model_that_fails_or_answers_nonsense_falls_back_without_its_words() {
        let out = read_pages(vec![png()], Ok(Arc::new(Fixed("I cannot help with that".into())))).await;
        assert_eq!(out.status, "unreadable");
        assert!(!out.note.unwrap().contains("cannot help"));
    }

    #[tokio::test]
    async fn the_transcript_counts_only_for_pages_the_model_saw() {
        let answer = json!({
            "transcript": "Please return the signed form.",
            "request": {"value": "Return the form", "quote": "Please return the signed form.", "uncertain": false}
        })
        .to_string();
        let read = read_pages(vec![pdf()], Ok(Arc::new(Fixed(answer.clone())))).await;
        assert_eq!(read.method, Some("pdf_text"));
        assert!(read.fields.request.unwrap().uncertain, "a transcript with no images is ignored");
        let read = read_pages(vec![pdf(), png()], Ok(Arc::new(Fixed(answer)))).await;
        assert_eq!(read.method, Some("pdf_text+model_vision"));
        assert!(!read.fields.request.unwrap().uncertain);
        assert_eq!(read.read_by.as_deref(), Some("stub:fixed"));
    }

    fn fields() -> Fields {
        Fields {
            sender: Some(Field { value: "Northfield Water".into(), quote: "Northfield Water".into(), ..Default::default() }),
            deadline: Some(Field { value: "2026-11-30".into(), quote: "by 30 November 2026".into(), uncertain: true, ..Default::default() }),
            amounts: vec![Field { value: "GBP 142.60".into(), quote: "GBP 142.60".into(), ..Default::default() }],
            checklist: vec![Field { value: "Pay".into(), done: Some(false), ..Default::default() }],
            ..Default::default()
        }
    }

    #[test]
    fn confirming_clears_uncertain_and_corrections_are_validated() {
        let f = patch(&fields(), &json!({"deadline": {}})).unwrap();
        let d = f.deadline.as_ref().unwrap();
        assert!(d.confirmed && !d.uncertain);
        assert_eq!(d.quote, "by 30 November 2026");
        assert_eq!(f.confirmed_deadline(), chrono::NaiveDate::from_ymd_opt(2026, 11, 30));
        assert!(patch(&fields(), &json!({"deadline": {"value": "30/11/2026"}})).is_err());
        assert!(patch(&fields(), &json!({"amounts": [{"value": "GBP 1,000"}]})).is_err());
        assert!(patch(&fields(), &json!({"sender": {"value": "  "}})).is_err());
        assert!(patch(&fields(), &json!({"reply_to": {}})).is_err(), "nothing to confirm");
        assert!(patch(&fields(), &json!({"colour": null})).is_err());
        assert!(patch(&fields(), &json!({"amounts": [{"value": "GBP 1.00", "done": true}]})).is_err());
        let f = patch(&fields(), &json!({"sender": null, "amounts": [{"value": "£142.60"}, {"value": "EUR 5"}], "checklist": [{"value": "Pay", "done": true}]})).unwrap();
        assert!(f.sender.is_none());
        assert_eq!(f.amounts[0].quote, "GBP 142.60", "an unchanged value keeps its quote");
        assert_eq!(f.amounts[1].value, "EUR 5.00");
        assert_eq!(f.checklist[0].done, Some(true));
    }

    #[test]
    fn a_list_confirms_only_new_changed_or_confirmed_entries() {
        let mut old = fields();
        old.checklist = vec![
            Field { value: "Pay".into(), quote: "pay by".into(), uncertain: true, done: Some(false), ..Default::default() },
            Field { value: "Call".into(), done: Some(false), ..Default::default() },
        ];
        // Ticking one item: neither changes its confirmed/uncertain state
        let f = patch(&old, &json!({"checklist": [{"value": "Pay", "done": true}, {"value": "Call", "done": false}]})).unwrap();
        assert_eq!(f.checklist[0].done, Some(true));
        assert!(f.checklist[0].uncertain && !f.checklist[0].confirmed, "ticked, still uncertain");
        assert!(!f.checklist[1].uncertain && !f.checklist[1].confirmed, "untouched, still unconfirmed");
        assert_eq!(f.checklist[0].quote, "pay by");
        // A changed value and a new one are confirmed; the unchanged one isn't
        let f = patch(&old, &json!({"checklist": [{"value": "Pay"}, {"value": "Call back"}, {"value": "File it"}]})).unwrap();
        assert!(f.checklist[0].uncertain && !f.checklist[0].confirmed);
        assert!(f.checklist[1..].iter().all(|x| x.confirmed && !x.uncertain && x.quote.is_empty()));
        // Confirming as they are
        let f = patch(&old, &json!({"checklist": [{"value": "Pay", "confirmed": true}, {"value": "Call", "confirmed": true}]})).unwrap();
        assert!(f.checklist.iter().all(|x| x.confirmed && !x.uncertain && x.done == Some(false)));
        assert_eq!(f.checklist[0].quote, "pay by");
        let f = patch(&old, &json!({"amounts": [{"value": "GBP 142.60"}]})).unwrap();
        assert!(!f.amounts[0].confirmed, "an unchanged amount isn't confirmed by resending it");
        assert!(patch(&old, &json!({"amounts": [{"value": "GBP 142.60", "confirmed": "yes"}]})).is_err());
    }

    #[test]
    fn the_case_takes_only_a_confirmed_deadline() {
        let now = Utc::now();
        let mut c = new_case("c1", now);
        apply(&mut c, &fields(), now);
        assert_eq!(c.title, "Letter from Northfield Water");
        assert_eq!(c.deadline, None, "uncertain deadline");
        assert_eq!(c.kind, "document_request");
        assert_eq!(c.conv_kind, "document");
        let draft: crate::zombie::draft::Draft = serde_json::from_str(c.draft.as_deref().unwrap()).unwrap();
        assert_eq!(draft.channel, "letter");
        let confirmed = patch(&fields(), &json!({"deadline": {}, "reply_to": {"value": "billing@northfield-water.example"}})).unwrap();
        apply(&mut c, &confirmed, now);
        assert_eq!(c.deadline.as_deref(), Some("2026-11-30"));
        assert_eq!(c.counterpart.as_deref(), Some("billing@northfield-water.example"));
        let draft: crate::zombie::draft::Draft = serde_json::from_str(c.draft.as_deref().unwrap()).unwrap();
        assert_eq!(draft.channel, "email");
        assert!(draft.mailto.is_some());
    }
}
