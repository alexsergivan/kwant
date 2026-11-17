//! 26h: pictures, audio and video an MCP tool returns are saved and shown in the turn's chat as
//! the bot's files, instead of reaching nobody as `[image: image/png]`.
//!
//! [`finish`] runs on every successful call kwnt makes itself ([`super::session::McpToolSession`],
//! so API engines and the CLI gateway both get it):
//! - `image` and `audio` parts and `resource` parts with a `blob` are decoded, stored and posted
//!   through the turn's [`MediaSink`] (the bot's file share: sha256 dedupe, the 20 MB / 100 MB
//!   caps). The model gets one line per file ("Image saved in this chat as … (id …)"), never the
//!   base64.
//! - Expiring delivery links (FLUX's `https://delivery-….bfl.ai/…`) are downloaded right away,
//!   saved the same way, and the link in the text is replaced by the saved file. The download is
//!   an SSRF boundary: HTTPS only, only the app's allowlisted hosts (a redirect off the list
//!   stops it), public addresses only, a time limit and a size cap ([`DeliveryRule`]).
//!
//! At most [`MAX_FILES_PER_CALL`] files are saved per call; later ones are named as skipped. A
//! file that can't be saved keeps its stand-in with the reason, and the others still save.

use super::client::{result_text_with, ToolCallResult};
use super::payment_floor::CreditApp;
use async_trait::async_trait;
use base64::Engine;
use serde_json::Value;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::Duration;

/// Most files one call saves (the chat's per-message attachment limit).
pub const MAX_FILES_PER_CALL: usize = crate::server::attachments::MAX_ATTACHMENTS_PER_MESSAGE;
/// Largest file decoded or downloaded: the store's cap for videos and audio. The store then holds
/// everything else to 20 MB.
pub const MAX_FILE_BYTES: usize = crate::server::attachments::MAX_VIDEO_BYTES as usize;
/// Longest a delivery download may take, start to last byte.
const DOWNLOAD_SECS: u64 = 60;
/// Redirects a delivery download follows, each one checked against the allowlist.
const MAX_REDIRECTS: usize = 3;

/// A file [`MediaSink::save`] posted in the chat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedFile {
    pub id: String,
    pub name: String,
    /// The bot shared these exact bytes in this chat a few minutes ago: that file, not posted again.
    pub already: bool,
}

/// Where a turn's tool files go: the server stores them and posts each as the bot's file share.
#[async_trait]
pub trait MediaSink: Send + Sync {
    /// Stores `bytes` named `name` and posts it in the turn's chat, saying it was made with the
    /// MCP server `server`. `Err` says why not, as a clause people read ("the image is larger
    /// than 20 MB").
    async fn save(&self, bytes: Vec<u8>, name: &str, server: &str) -> Result<SavedFile, String>;
}

/// Hosts a delivery link may be on: `suffix` (`.bfl.ai`) with a first label starting with
/// `label_prefix` (`delivery`, so `delivery-eu1.bfl.ai` and `delivery.eu.bfl.ai`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostPattern {
    pub suffix: &'static str,
    pub label_prefix: &'static str,
}

impl HostPattern {
    fn matches(&self, host: &str) -> bool {
        let host = host.trim_end_matches('.').to_ascii_lowercase();
        let Some(rest) = host.strip_suffix(self.suffix) else { return false };
        let first = rest.split('.').next().unwrap_or("");
        !rest.is_empty() && first.starts_with(self.label_prefix)
    }
}

/// Which links of one app kwnt downloads, and how. Kept per app (preset), so another app's file
/// host (Picsart's CDN, if its results come as links) can join with its own list.
#[derive(Debug, Clone)]
pub struct DeliveryRule {
    hosts: Vec<HostPattern>,
    /// Off only in this module's tests, which serve plain http on 127.0.0.1.
    https_only: bool,
    public_only: bool,
}

impl DeliveryRule {
    /// The delivery hosts of `app`; `None` when its results aren't fetched from links.
    pub fn for_app(app: CreditApp) -> Option<Self> {
        let hosts = match app {
            // BFL's results: https://delivery-eu1.bfl.ai/results/…?se=…&sig=… (signed, expiring)
            CreditApp::Bfl => vec![HostPattern { suffix: ".bfl.ai", label_prefix: "delivery" }],
            // Picsart's results are links too, but on a CDN not checked yet: not fetched
            CreditApp::Picsart => return None,
        };
        Some(Self { hosts, https_only: true, public_only: true })
    }

    /// Whether kwnt may fetch `url`: https, no user or password, the default port, an
    /// allowlisted host name (never an IP address).
    pub fn allows(&self, url: &reqwest::Url) -> bool {
        if self.https_only && (url.scheme() != "https" || url.port().is_some()) {
            return false;
        }
        if !self.https_only && !matches!(url.scheme(), "https" | "http") {
            return false;
        }
        if !url.username().is_empty() || url.password().is_some() {
            return false;
        }
        match url.host() {
            Some(url::Host::Domain(d)) => self.hosts.iter().any(|h| h.matches(d)),
            Some(url::Host::Ipv4(ip)) if !self.public_only => self.hosts.iter().any(|h| h.suffix == ip.to_string()),
            _ => false,
        }
    }

    fn client(&self) -> reqwest::Client {
        let rule = self.clone();
        let policy = reqwest::redirect::Policy::custom(move |attempt| {
            if attempt.previous().len() > MAX_REDIRECTS {
                attempt.error("too many redirects")
            } else if rule.allows(attempt.url()) {
                attempt.follow()
            } else {
                attempt.error("it redirects off the app's file hosts")
            }
        });
        let mut b = reqwest::Client::builder()
            .redirect(policy)
            .timeout(Duration::from_secs(DOWNLOAD_SECS))
            .connect_timeout(Duration::from_secs(10))
            .no_proxy();
        if self.public_only {
            b = b.dns_resolver(std::sync::Arc::new(PublicOnly)).https_only(true);
        }
        b.build().unwrap_or_default()
    }
}

/// Resolves delivery hosts and keeps only public addresses: an allowlisted name pointed into this
/// network is never connected to.
struct PublicOnly;

impl reqwest::dns::Resolve for PublicOnly {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let host = name.as_str().to_string();
        Box::pin(async move {
            let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host.as_str(), 0))
                .await?
                .filter(|a| crate::server::push::is_public_ip(a.ip()))
                .collect();
            if addrs.is_empty() {
                return Err("the file host doesn't resolve to a public address".into());
            }
            Ok(Box::new(addrs.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

/// Downloads `url` under `rule`, at most [`MAX_FILE_BYTES`]. `Err` says why not.
async fn download(rule: &DeliveryRule, url: &reqwest::Url) -> Result<Vec<u8>, String> {
    if !rule.allows(url) {
        return Err("its address isn't one of the app's file hosts".into());
    }
    let mut res = rule.client().get(url.clone()).send().await.map_err(|e| {
        if e.is_redirect() {
            "it redirects off the app's file hosts".to_string()
        } else if e.is_timeout() {
            "the download took too long".to_string()
        } else {
            "the download failed".to_string()
        }
    })?;
    if !res.status().is_success() {
        return Err(format!("the file host answered {}", res.status().as_u16()));
    }
    if res.content_length().is_some_and(|n| n > MAX_FILE_BYTES as u64) {
        return Err("it is larger than 100 MB".into());
    }
    let mut bytes = Vec::new();
    loop {
        match res.chunk().await {
            Ok(Some(chunk)) => {
                if bytes.len() + chunk.len() > MAX_FILE_BYTES {
                    return Err("it is larger than 100 MB".into());
                }
                bytes.extend_from_slice(&chunk);
            }
            Ok(None) => return Ok(bytes),
            Err(e) if e.is_timeout() => return Err("the download took too long".into()),
            Err(_) => return Err("the download failed".into()),
        }
    }
}

/// A file inside a result part: its declared type and its base64 bytes.
struct FilePart<'a> {
    mime: &'a str,
    data: &'a str,
}

fn file_part(item: &Value) -> Option<FilePart<'_>> {
    match item["type"].as_str()? {
        "image" | "audio" => Some(FilePart { mime: item["mimeType"].as_str().unwrap_or("?"), data: item["data"].as_str()? }),
        "resource" => Some(FilePart {
            mime: item["resource"]["mimeType"].as_str().unwrap_or("application/octet-stream"),
            data: item["resource"]["blob"].as_str()?,
        }),
        _ => None,
    }
}

/// "Image", "Audio file", "Video" or "File", for a type.
fn noun(mime: &str) -> &'static str {
    match mime.split('/').next().unwrap_or("") {
        "image" => "Image",
        "audio" => "Audio file",
        "video" => "Video",
        _ => "File",
    }
}

fn extension(mime: &str) -> &'static str {
    match mime {
        "image/png" => "png",
        "image/jpeg" | "image/jpg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "video/mp4" => "mp4",
        "video/quicktime" => "mov",
        "video/webm" => "webm",
        "audio/mpeg" | "audio/mp3" => "mp3",
        "audio/mp4" | "audio/m4a" => "m4a",
        "audio/aac" => "aac",
        "audio/wav" | "audio/x-wav" => "wav",
        "audio/ogg" => "ogg",
        "audio/flac" => "flac",
        "application/pdf" => "pdf",
        _ => "bin",
    }
}

/// The type the bytes prove, else the declared one.
fn real_mime<'a>(bytes: &[u8], declared: &'a str) -> &'a str {
    crate::server::attachments::sniff_magic(&bytes[..bytes.len().min(16)]).unwrap_or(declared)
}

/// The file's name in the chat: `<server>-<tool>-<n>.<ext>`, letters, digits, `-` and `_` only,
/// so it can stand inside the model's JSON text.
fn file_name(server: &str, tool: &str, n: usize, mime: &str) -> String {
    let clean = |s: &str| -> String {
        let s: String = s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' }).collect();
        s.chars().take(40).collect()
    };
    format!("{}-{}-{n}.{}", clean(server), clean(tool), extension(mime))
}

/// What became of one file, for the model.
fn saved_line(noun: &str, saved: &SavedFile) -> String {
    if saved.already {
        format!("{noun} already in this chat as {} (id {}): the same file was shared a few minutes ago.", saved.name, saved.id)
    } else {
        format!("{noun} saved in this chat as {} (id {}); the person sees it there.", saved.name, saved.id)
    }
}

/// One call's file saving: counts what it saved against [`MAX_FILES_PER_CALL`] and numbers names.
struct Saver<'a> {
    sink: Option<&'a dyn MediaSink>,
    server: &'a str,
    tool: &'a str,
    saved: usize,
    named: usize,
}

impl Saver<'_> {
    fn full(&self) -> bool {
        self.saved >= MAX_FILES_PER_CALL
    }

    async fn save(&mut self, bytes: Vec<u8>, declared: &str) -> Result<SavedFile, String> {
        let Some(sink) = self.sink else {
            return Err("this turn has no chat to save it in".into());
        };
        self.named += 1;
        let mime = real_mime(&bytes, declared);
        let name = file_name(self.server, self.tool, self.named, mime);
        let saved = sink.save(bytes, &name, self.server).await?;
        self.saved += 1;
        Ok(saved)
    }
}

/// The text the model gets for a successful call: [`ToolCallResult::text`], with every file part
/// saved in the chat and named by its saved file, and (for an app with a [`DeliveryRule`]) every
/// delivery link downloaded and replaced by its saved file. `sink` is the turn's chat; without one
/// the model is told a file came back but couldn't be saved.
pub async fn finish(result: &ToolCallResult, server: &str, server_url: Option<&str>, tool: &str, sink: Option<&dyn MediaSink>) -> String {
    let delivery = CreditApp::of_server(server, server_url).and_then(DeliveryRule::for_app);
    finish_with(result, server, tool, sink, delivery.as_ref()).await
}

async fn finish_with(result: &ToolCallResult, server: &str, tool: &str, sink: Option<&dyn MediaSink>, delivery: Option<&DeliveryRule>) -> String {
    let parts: Vec<(usize, FilePart)> = result.raw["content"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
        .filter_map(|(i, item)| file_part(item).map(|p| (i, p)))
        .collect();
    if parts.is_empty() && delivery.is_none() {
        return result.text.clone();
    }
    let mut saver = Saver { sink, server, tool, saved: 0, named: 0 };
    let mut lines: HashMap<usize, String> = HashMap::new();
    for (i, part) in parts {
        let stand_in = format!("[{}: {}]", noun(part.mime).to_lowercase(), part.mime);
        let line = if saver.full() {
            format!("{stand_in} Not saved: only the first {MAX_FILES_PER_CALL} files a tool returns are saved in the chat.")
        } else if part.data.len() / 4 * 3 > MAX_FILE_BYTES {
            format!("{stand_in} {} came back but couldn't be saved: it is larger than 100 MB.", article(noun(part.mime)))
        } else {
            let decoded = base64::engine::general_purpose::STANDARD
                .decode(part.data.trim())
                .map_err(|_| "its data isn't valid base64".to_string());
            match decoded {
                Ok(bytes) => {
                    let mime = real_mime(&bytes, part.mime).to_string();
                    match saver.save(bytes, &mime).await {
                        Ok(saved) => saved_line(noun(&mime), &saved),
                        Err(why) => format!("{stand_in} {} came back but couldn't be saved: {why}.", article(noun(&mime))),
                    }
                }
                Err(why) => format!("{stand_in} {} came back but couldn't be saved: {why}.", article(noun(part.mime))),
            }
        };
        lines.insert(i, line);
    }
    let mut text = if lines.is_empty() { result.text.clone() } else { result_text_with(&result.raw, |i| lines.get(&i).cloned()) };
    if let Some(rule) = delivery {
        text = save_links(text, rule, &mut saver).await;
    }
    text
}

/// "An image", "An audio file", "A video", "A file".
fn article(noun: &str) -> String {
    let lower = noun.to_lowercase();
    if lower.starts_with(['a', 'e', 'i', 'o', 'u']) {
        format!("An {lower}")
    } else {
        format!("A {lower}")
    }
}

/// The `https://…` and `http://…` links in `text` (each once, in order), ending at whitespace, a
/// quote, a bracket or a backslash, without trailing punctuation. The delivery rule then refuses
/// plain http (only this module's tests allow it).
fn links(text: &str) -> Vec<&str> {
    let mut out: Vec<&str> = Vec::new();
    let mut rest = text;
    while let Some(at) = [rest.find("https://"), rest.find("http://")].into_iter().flatten().min() {
        let tail = &rest[at..];
        let end = tail
            .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>' | '`' | '\\' | ')' | ']' | '}'))
            .unwrap_or(tail.len());
        let link = tail[..end].trim_end_matches(['.', ',', ';', ':']);
        if !out.contains(&link) {
            out.push(link);
        }
        rest = &tail[end.max(7)..];
    }
    out
}

/// Downloads each allowlisted link in `text`, saves it, and puts the saved file in the link's
/// place; a link that can't be saved stays, with the reason added.
async fn save_links(mut text: String, rule: &DeliveryRule, saver: &mut Saver<'_>) -> String {
    let found: Vec<(String, reqwest::Url)> = links(&text)
        .into_iter()
        .filter_map(|l| reqwest::Url::parse(l).ok().filter(|u| rule.allows(u)).map(|u| (l.to_string(), u)))
        .collect();
    let mut notes = Vec::new();
    for (link, url) in found {
        if saver.full() {
            notes.push(format!(
                "Not saved: {link} (only the first {MAX_FILES_PER_CALL} files a tool returns are saved in the chat; the link expires)."
            ));
            continue;
        }
        let outcome = match saver.sink {
            None => Err("this turn has no chat to save it in".to_string()),
            Some(_) => match download(rule, &url).await {
                Ok(bytes) => {
                    let mime = real_mime(&bytes, "application/octet-stream").to_string();
                    saver.save(bytes, &mime).await.map(|s| (s, mime))
                }
                Err(why) => Err(why),
            },
        };
        match outcome {
            Ok((saved, mime)) => {
                text = text.replace(&link, &format!("kwnt-file:{} ({})", saved.id, saved.name));
                notes.push(saved_line(noun(&mime), &saved));
            }
            Err(why) => notes.push(format!("The file at {link} couldn't be saved in this chat: {why}. The link expires soon.")),
        }
    }
    if !notes.is_empty() {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&notes.join("\n"));
    }
    text
}

/// 26h D3: claude's PostToolUse hook for an MCP tool call (the claude engine calls the bot's
/// servers itself, so [`finish`] never sees the result). `tool_response` is the content claude
/// made of the result, recorded from claude 2.1.326 (Nov 17):
/// - an `image` part is `{"type":"image","source":{"type":"base64","media_type":…,"data":…}}`,
///   then claude's own `[Image: source: <its local copy>]` text;
/// - an `audio` part (or a blob claude can't show) is only a text line, `[Audio from <server>]
///   Binary content (<type>, <size>) saved to <path>`, with the bytes in claude's `tool-results`
///   folder for this session;
/// - a `resource_link` is `[Resource link: <name>] <uri>`, which the delivery rule reads as a link.
///
/// The parts come back in MCP's shape (`{"content": […]}`) so the same saving runs on them.
/// `tool_results` is that session's `tool-results` folder; a saved-to path is read only when it is
/// a plain file claude wrote there for an MCP tool (`mcp-…-blob-…`), never anywhere a result's
/// text points. Also says whether a part was turned into a note (a file over the cap).
pub fn from_claude(tool_response: &Value, tool_results: Option<&std::path::Path>) -> (Value, bool) {
    let blocks: Vec<Value> = match tool_response {
        Value::Array(a) => a.clone(),
        Value::Object(o) if o.get("content").is_some_and(Value::is_array) => o["content"].as_array().cloned().unwrap_or_default(),
        Value::String(s) => vec![serde_json::json!({"type": "text", "text": s})],
        _ => Vec::new(),
    };
    let mut content = Vec::new();
    let mut noted = false;
    let mut after_image = false;
    for block in blocks {
        let was_image = std::mem::replace(&mut after_image, false);
        let kind = block["type"].as_str().unwrap_or("").to_string();
        match kind.as_str() {
            "image" if block["source"]["type"] == "base64" => {
                if let Some(data) = block["source"]["data"].as_str() {
                    let mime = block["source"]["media_type"].as_str().unwrap_or("image/png");
                    content.push(serde_json::json!({"type": "image", "mimeType": mime, "data": data}));
                    after_image = true;
                    continue;
                }
                content.push(block);
            }
            "text" => {
                let text = block["text"].as_str().unwrap_or("");
                // claude's note of where it kept the picture it just showed: the saved line says it now
                if was_image && text.starts_with("[Image: source: ") && text.ends_with(']') && !text.contains('\n') {
                    continue;
                }
                match claude_blob(text).map(|(mime, path)| (mime, read_blob(path, tool_results))) {
                    Some((mime, Ok(bytes))) => {
                        let data = base64::engine::general_purpose::STANDARD.encode(&bytes);
                        let part = match mime.split('/').next() {
                            Some("image") => serde_json::json!({"type": "image", "mimeType": mime, "data": data}),
                            Some("audio") => serde_json::json!({"type": "audio", "mimeType": mime, "data": data}),
                            _ => serde_json::json!({"type": "resource", "resource": {"uri": "", "mimeType": mime, "blob": data}}),
                        };
                        content.push(part);
                    }
                    Some((mime, Err(BlobError::TooBig))) => {
                        noted = true;
                        let line = format!(
                            "[{}: {mime}] {} came back but couldn't be saved: it is larger than 100 MB.",
                            noun(&mime).to_lowercase(),
                            article(noun(&mime))
                        );
                        content.push(serde_json::json!({"type": "text", "text": line}));
                    }
                    // Not a file claude kept for this session: the text stays as it is
                    _ => content.push(block),
                }
            }
            _ => content.push(block),
        }
    }
    (serde_json::json!({ "content": content }), noted)
}

/// The type and path in claude's `[Audio from x] Binary content (audio/wav, 7.9KB) saved to /p`.
fn claude_blob(text: &str) -> Option<(String, &str)> {
    if !["[Audio from ", "[Image from ", "[Resource from "].iter().any(|p| text.starts_with(p)) || text.contains('\n') {
        return None;
    }
    let (_, rest) = text.split_once("] Binary content (")?;
    let (mime, rest) = rest.split_once(", ")?;
    let (_, path) = rest.split_once(") saved to ")?;
    let mime = mime.trim();
    let mime = if mime.contains('/') && !mime.contains(char::is_whitespace) { mime.to_string() } else { "application/octet-stream".to_string() };
    Some((mime, path.trim()))
}

enum BlobError {
    TooBig,
    NotAllowed,
}

/// Reads `path` only when it is a plain file (not a link) directly in `dir`, named the way claude
/// names an MCP tool's blob (`mcp-<server>-blob-…`), and at most [`MAX_FILE_BYTES`].
fn read_blob(path: &str, dir: Option<&std::path::Path>) -> Result<Vec<u8>, BlobError> {
    let dir = dir.and_then(|d| std::fs::canonicalize(d).ok()).ok_or(BlobError::NotAllowed)?;
    let path = std::path::Path::new(path);
    if !path.is_absolute() {
        return Err(BlobError::NotAllowed);
    }
    let meta = std::fs::symlink_metadata(path).map_err(|_| BlobError::NotAllowed)?;
    let real = std::fs::canonicalize(path).map_err(|_| BlobError::NotAllowed)?;
    let name = real.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if !meta.file_type().is_file() || real.parent() != Some(dir.as_path()) || !name.starts_with("mcp-") || !name.contains("-blob-") {
        return Err(BlobError::NotAllowed);
    }
    if meta.len() > MAX_FILE_BYTES as u64 {
        return Err(BlobError::TooBig);
    }
    std::fs::read(&real).map_err(|_| BlobError::NotAllowed)
}

/// 26h D3: [`finish`] for a claude PostToolUse payload's `tool_response` ([`from_claude`]): the
/// files are saved in the chat and the model's text names them. `None` when there is nothing to
/// save (no file part, and no allowlisted delivery link for this app): claude's own output stays.
pub async fn finish_claude(
    tool_response: &Value,
    server: &str,
    server_url: Option<&str>,
    tool: &str,
    sink: Option<&dyn MediaSink>,
    tool_results: Option<&std::path::Path>,
) -> Option<String> {
    let (raw, noted) = from_claude(tool_response, tool_results);
    let delivery = CreditApp::of_server(server, server_url).and_then(DeliveryRule::for_app);
    let text = result_text_with(&raw, |_| None);
    let files = raw["content"].as_array().into_iter().flatten().any(|p| file_part(p).is_some());
    let has_links = delivery
        .as_ref()
        .is_some_and(|rule| links(&text).into_iter().any(|l| reqwest::Url::parse(l).is_ok_and(|u| rule.allows(&u))));
    if !files && !has_links && !noted {
        return None;
    }
    let result = ToolCallResult { text, is_error: false, raw };
    Some(finish_with(&result, server, tool, sink, delivery.as_ref()).await)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Mutex;

    /// A 1×1 PNG.
    const PNG: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x01,
        0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x44, 0x41,
        0x54, 0x78, 0x9C, 0x63, 0xF8, 0xCF, 0xC0, 0xF0, 0x1F, 0x00, 0x05, 0x00, 0x01, 0xFF, 0x89, 0x99, 0x3D, 0x1D, 0x00, 0x00,
        0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];

    fn b64(bytes: &[u8]) -> String {
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }

    /// Keeps what it was given; refuses files over `max` bytes the way the store does.
    #[derive(Default)]
    struct Sink {
        files: Mutex<Vec<(String, Vec<u8>)>>,
        max: Option<usize>,
    }

    #[async_trait]
    impl MediaSink for Sink {
        async fn save(&self, bytes: Vec<u8>, name: &str, _server: &str) -> Result<SavedFile, String> {
            if self.max.is_some_and(|m| bytes.len() > m) {
                return Err("the image is larger than 20 MB".into());
            }
            let mut files = self.files.lock().unwrap();
            let already = files.iter().any(|(_, b)| *b == bytes);
            files.push((name.to_string(), bytes));
            Ok(SavedFile { id: format!("att{}", files.len()), name: name.to_string(), already })
        }
    }

    fn result(content: Value) -> ToolCallResult {
        let raw = json!({ "content": content });
        ToolCallResult { text: result_text_with(&raw, |_| None), is_error: false, raw }
    }

    async fn run(r: &ToolCallResult, sink: Option<&Sink>) -> String {
        finish_with(r, "flux", "generate_image", sink.map(|s| s as &dyn MediaSink), None).await
    }

    #[tokio::test]
    async fn a_png_part_is_saved_and_named_never_sent_as_base64() {
        let sink = Sink::default();
        let r = result(json!([{"type": "image", "mimeType": "image/png", "data": b64(PNG)}]));
        assert_eq!(r.text, "[image: image/png]", "the old stand-in");
        let text = run(&r, Some(&sink)).await;
        assert_eq!(text, "Image saved in this chat as flux-generate_image-1.png (id att1); the person sees it there.");
        assert!(!text.contains(&b64(PNG)[..12]));
        let files = sink.files.lock().unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].1, PNG);
    }

    #[tokio::test]
    async fn bad_base64_keeps_the_stand_in_with_the_reason() {
        let sink = Sink::default();
        let r = result(json!([{"type": "image", "mimeType": "image/png", "data": "not base64!!"}]));
        let text = run(&r, Some(&sink)).await;
        assert_eq!(text, "[image: image/png] An image came back but couldn't be saved: its data isn't valid base64.");
        assert!(sink.files.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn text_and_pictures_keep_their_order() {
        let sink = Sink::default();
        let r = result(json!([
            {"type": "text", "text": "Here you go"},
            {"type": "image", "mimeType": "image/png", "data": b64(PNG)},
            {"type": "text", "text": "seed 42"},
            {"type": "resource", "resource": {"uri": "file:///x.png", "mimeType": "image/png", "blob": b64(PNG)}},
            {"type": "resource", "resource": {"uri": "file:///notes.txt", "text": "a note"}},
        ]));
        let text = run(&r, Some(&sink)).await;
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "Here you go");
        assert!(lines[1].starts_with("Image saved in this chat as flux-generate_image-1.png (id att1)"), "{text}");
        assert_eq!(lines[2], "seed 42");
        assert!(lines[3].starts_with("Image already in this chat as flux-generate_image-2.png (id att2)"), "same bytes: {text}");
        assert_eq!(lines[4], "a note", "a text resource stays text");
    }

    #[tokio::test]
    async fn an_oversized_part_is_skipped_and_the_others_still_save() {
        let sink = Sink { max: Some(1000), ..Default::default() };
        let mut big = PNG.to_vec();
        big.resize(5000, 0);
        let r = result(json!([
            {"type": "image", "mimeType": "image/png", "data": b64(&big)},
            {"type": "image", "mimeType": "image/png", "data": b64(PNG)},
        ]));
        let text = run(&r, Some(&sink)).await;
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "[image: image/png] An image came back but couldn't be saved: the image is larger than 20 MB.");
        assert!(lines[1].starts_with("Image saved in this chat as flux-generate_image-2.png"), "{text}");
        assert_eq!(sink.files.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_part_over_100_mb_is_never_decoded() {
        let sink = Sink::default();
        // Only its length is read: 140 M base64 characters stand for 105 MB
        let huge = "A".repeat(140 * 1024 * 1024);
        let r = result(json!([{"type": "audio", "mimeType": "audio/wav", "data": huge}]));
        let text = run(&r, Some(&sink)).await;
        assert_eq!(text, "[audio file: audio/wav] An audio file came back but couldn't be saved: it is larger than 100 MB.");
    }

    #[tokio::test]
    async fn only_the_first_ten_files_are_saved() {
        let sink = Sink::default();
        let parts: Vec<Value> = (0..12u8)
            .map(|n| {
                let mut png = PNG.to_vec();
                png.push(n); // distinct bytes, so none is a repeat
                json!({"type": "image", "mimeType": "image/png", "data": b64(&png)})
            })
            .collect();
        let text = run(&result(json!(parts)), Some(&sink)).await;
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 12);
        assert!(lines[9].starts_with("Image saved in this chat as flux-generate_image-10.png"), "{text}");
        assert_eq!(lines[10], "[image: image/png] Not saved: only the first 10 files a tool returns are saved in the chat.");
        assert_eq!(lines[11], lines[10]);
        assert_eq!(sink.files.lock().unwrap().len(), 10);
    }

    #[tokio::test]
    async fn without_a_chat_the_model_is_told_it_wasnt_saved() {
        let r = result(json!([{"type": "image", "mimeType": "image/png", "data": b64(PNG)}]));
        let text = run(&r, None).await;
        assert_eq!(text, "[image: image/png] An image came back but couldn't be saved: this turn has no chat to save it in.");
    }

    #[tokio::test]
    async fn the_real_type_names_the_file() {
        let sink = Sink::default();
        // Declared as JPEG, but the bytes are a PNG
        let r = result(json!([{"type": "image", "mimeType": "image/jpeg", "data": b64(PNG)}]));
        let text = run(&r, Some(&sink)).await;
        assert!(text.starts_with("Image saved in this chat as flux-generate_image-1.png"), "{text}");
    }

    #[tokio::test]
    async fn text_only_results_are_untouched() {
        let r = result(json!([{"type": "text", "text": "https://delivery-eu1.bfl.ai/x.png"}]));
        assert_eq!(run(&r, Some(&Sink::default())).await, r.text, "no delivery rule for this server");
    }

    #[test]
    fn delivery_hosts_are_allowlisted() {
        let rule = DeliveryRule::for_app(CreditApp::Bfl).unwrap();
        let ok = |u: &str| rule.allows(&reqwest::Url::parse(u).unwrap());
        assert!(ok("https://delivery-eu1.bfl.ai/results/ab/cd/sample.jpeg?se=2026&sig=x"));
        assert!(ok("https://delivery-us1.bfl.ai/results/x.png"));
        assert!(ok("https://delivery.eu.bfl.ai/x.png"));
        assert!(!ok("http://delivery-eu1.bfl.ai/x.png"), "https only");
        assert!(!ok("https://delivery-eu1.bfl.ai:8443/x.png"), "the default port only");
        assert!(!ok("https://user:pw@delivery-eu1.bfl.ai/x.png"));
        assert!(!ok("https://api.bfl.ai/x.png"), "not a delivery host");
        assert!(!ok("https://mcp.bfl.ai/x.png"));
        assert!(!ok("https://delivery-eu1.bfl.ai.evil.com/x.png"));
        assert!(!ok("https://deliverybfl.ai/x.png"));
        assert!(!ok("https://.bfl.ai/x.png"));
        assert!(!ok("https://169.254.169.254/latest/meta-data"));
        assert!(!ok("https://127.0.0.1/x.png"));
        assert!(DeliveryRule::for_app(CreditApp::Picsart).is_none(), "Picsart's CDN isn't on any list yet");
    }

    #[test]
    fn links_are_found_whole_and_once() {
        let text = r#"{"image_url": "https://delivery-eu1.bfl.ai/r/a.jpeg?se=1&sig=a%2Fb", "again": "https://delivery-eu1.bfl.ai/r/a.jpeg?se=1&sig=a%2Fb"} see (https://x.example/p). https://y.example/q, end"#;
        assert_eq!(
            links(text),
            ["https://delivery-eu1.bfl.ai/r/a.jpeg?se=1&sig=a%2Fb", "https://x.example/p", "https://y.example/q"]
        );
        assert_eq!(links("a http://127.0.0.1:9/png, b"), ["http://127.0.0.1:9/png"]);
    }

    /// A local file host: `/png` is a picture, `/away` redirects off the list, `/hop` redirects
    /// to `/png`, `/missing` is a 404. [`big_host`] claims 200 MB.
    async fn file_host() -> String {
        use axum::{routing::get, Router};
        let app = Router::new()
            .route("/png", get(|| async { ([("content-type", "image/png")], PNG.to_vec()) }))
            .route("/hop", get(|| async { axum::response::Redirect::temporary("/png") }))
            .route("/away", get(|| async { axum::response::Redirect::temporary("http://localhost:1/png") }))
            .route("/missing", get(|| async { axum::http::StatusCode::NOT_FOUND }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.ok() });
        format!("http://{addr}")
    }

    /// A raw host whose every answer declares 200 MB but sends one byte. hyper refuses to serve a
    /// body shorter than its own content-length header (a debug-build panic that drops the
    /// connection), so this one writes the response by hand.
    async fn big_host() -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let mut request = [0u8; 4096];
                    let _ = socket.read(&mut request).await;
                    let head = format!("HTTP/1.1 200 OK\r\ncontent-type: image/png\r\ncontent-length: {}\r\n\r\nx", 200 * 1024 * 1024);
                    let _ = socket.write_all(head.as_bytes()).await;
                    // Held open: the client must refuse on the header, not on a short body
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                });
            }
        });
        format!("http://{addr}/big")
    }

    fn local_rule() -> DeliveryRule {
        DeliveryRule { hosts: vec![HostPattern { suffix: "127.0.0.1", label_prefix: "" }], https_only: false, public_only: false }
    }

    #[tokio::test]
    async fn a_delivery_link_is_downloaded_saved_and_replaced() {
        let base = file_host().await;
        let sink = Sink::default();
        let link = format!("{base}/png");
        let r = result(json!([{"type": "text", "text": format!("{{\"image_url\": \"{link}\", \"seed\": 7}}")}]));
        let text = finish_with(&r, "flux", "generate_image", Some(&sink), Some(&local_rule())).await;
        assert!(!text.contains(&link), "{text}");
        assert!(text.starts_with("{\"image_url\": \"kwnt-file:att1 (flux-generate_image-1.png)\", \"seed\": 7}"), "{text}");
        assert!(text.ends_with("\nImage saved in this chat as flux-generate_image-1.png (id att1); the person sees it there."), "{text}");
        assert_eq!(sink.files.lock().unwrap()[0].1, PNG);
        // A redirect within the list is followed
        let r = result(json!([{"type": "text", "text": format!("{base}/hop")}]));
        let text = finish_with(&r, "flux", "generate_image", Some(&sink), Some(&local_rule())).await;
        assert!(text.contains("Image already in this chat"), "{text}");
    }

    #[tokio::test]
    async fn a_delivery_link_that_cant_be_saved_stays_with_the_reason() {
        let base = file_host().await;
        let sink = Sink::default();
        for (link, why) in [
            (format!("{base}/away"), "it redirects off the app's file hosts"),
            (big_host().await, "it is larger than 100 MB"),
            (format!("{base}/missing"), "the file host answered 404"),
        ] {
            let r = result(json!([{"type": "text", "text": format!("image: {link}")}]));
            let text = finish_with(&r, "flux", "generate_image", Some(&sink), Some(&local_rule())).await;
            assert_eq!(text, format!("image: {link}\nThe file at {link} couldn't be saved in this chat: {why}. The link expires soon."));
        }
        // Links off the list are left alone and never fetched
        let r = result(json!([{"type": "text", "text": "see https://example.com/a.png"}]));
        let text = finish_with(&r, "flux", "generate_image", Some(&sink), Some(&local_rule())).await;
        assert_eq!(text, "see https://example.com/a.png");
        assert!(sink.files.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn the_real_rule_refuses_a_local_address_before_connecting() {
        let rule = DeliveryRule::for_app(CreditApp::Bfl).unwrap();
        let url = reqwest::Url::parse("https://127.0.0.1/x.png").unwrap();
        assert_eq!(download(&rule, &url).await, Err("its address isn't one of the app's file hosts".into()));
    }

    /// claude's PostToolUse `tool_response` for an image, as claude 2.1.326 sent it (Nov 17).
    fn claude_image(local_copy: &str) -> Value {
        json!([
            {"type": "text", "text": "Here is your picture."},
            {"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": b64(PNG)}},
            {"type": "text", "text": format!("[Image: source: {local_copy}]")}
        ])
    }

    async fn claude(resp: &Value, sink: Option<&Sink>, dir: Option<&std::path::Path>) -> Option<String> {
        finish_claude(resp, "flux", None, "generate_image", sink.map(|s| s as &dyn MediaSink), dir).await
    }

    #[tokio::test]
    async fn a_claude_hook_picture_is_saved_and_claudes_local_copy_line_goes() {
        let sink = Sink::default();
        let text = claude(&claude_image("/h/.claude/projects/-w/s1/tool-results/mcp-flux-blob-1.png"), Some(&sink), None).await.unwrap();
        assert_eq!(
            text,
            "Here is your picture.\nImage saved in this chat as flux-generate_image-1.png (id att1); the person sees it there."
        );
        assert!(!text.contains("tool-results") && !text.contains(&b64(PNG)[..12]), "{text}");
        assert_eq!(sink.files.lock().unwrap()[0].1, PNG);
        // Without a chat the model still hears why
        let none = claude(&claude_image("/x"), None, None).await.unwrap();
        assert!(none.contains("[image: image/png] An image came back but couldn't be saved: this turn has no chat to save it in."), "{none}");
        // Text only, or a link no rule allows: nothing to do, claude's own output stays
        assert_eq!(claude(&json!([{"type": "text", "text": "done"}]), Some(&sink), None).await, None);
        assert_eq!(claude(&json!("done"), Some(&sink), None).await, None);
        let link = json!([{"type": "text", "text": "[Resource link: y.png] https://example.com/y.png"}]);
        assert_eq!(claude(&link, Some(&sink), None).await, None);
        // A text that only looks like claude's copy line, not after a picture, stays
        let (raw, _) = from_claude(&json!([{"type": "text", "text": "[Image: source: /etc/x]"}]), None);
        assert_eq!(raw["content"][0]["text"], "[Image: source: /etc/x]");
    }

    #[tokio::test]
    async fn a_claude_hook_audio_file_is_read_only_from_the_sessions_tool_results() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("s1").join("tool-results");
        std::fs::create_dir_all(&dir).unwrap();
        let wav = b"RIFF\x24\x00\x00\x00WAVEfmt more".to_vec();
        let kept = dir.join("mcp-flux-blob-1794927836623-qzwmn2.wav");
        std::fs::write(&kept, &wav).unwrap();
        fn line(p: &std::path::Path) -> Value {
            json!([{"type": "text", "text": format!("[Audio from flux] Binary content (audio/wav, 7.9KB) saved to {}", p.display())}])
        }
        let sink = Sink::default();
        let text = claude(&line(&kept), Some(&sink), Some(&dir)).await.unwrap();
        assert_eq!(text, "Audio file saved in this chat as flux-generate_image-1.wav (id att1); the person sees it there.");
        assert_eq!(sink.files.lock().unwrap()[0].1, wav);

        // A path elsewhere, a file claude didn't name as a tool's blob, or no folder: not read
        let outside = tmp.path().join("mcp-flux-blob-2.wav");
        std::fs::write(&outside, &wav).unwrap();
        let other = dir.join("toolu_01.txt");
        std::fs::write(&other, b"secret").unwrap();
        let sink = Sink::default();
        for (path, folder) in [(&outside, Some(dir.as_path())), (&other, Some(dir.as_path())), (&kept, None)] {
            assert_eq!(claude(&line(path), Some(&sink), folder).await, None, "{}", path.display());
        }
        let walk = dir.join("..").join("..").join("mcp-flux-blob-2.wav");
        assert_eq!(claude(&line(&walk), Some(&sink), Some(&dir)).await, None, "a .. path out of the folder");
        assert!(sink.files.lock().unwrap().is_empty());
        #[cfg(unix)]
        {
            let link = dir.join("mcp-flux-blob-3.wav");
            std::os::unix::fs::symlink(&outside, &link).unwrap();
            assert_eq!(claude(&line(&link), Some(&sink), Some(&dir)).await, None, "a link in the folder isn't followed");
            // Over the cap: named, not read
            let big = dir.join("mcp-flux-blob-4.mp4");
            std::fs::File::create(&big).unwrap().set_len(MAX_FILE_BYTES as u64 + 1).unwrap();
            let msg = json!([{"type": "text", "text": format!("[Resource from flux at file:///v.mp4] Binary content (video/mp4, 100MB) saved to {}", big.display())}]);
            let text = claude(&msg, Some(&sink), Some(&dir)).await.unwrap();
            assert_eq!(text, "[video: video/mp4] A video came back but couldn't be saved: it is larger than 100 MB.");
            assert!(sink.files.lock().unwrap().is_empty());
        }
    }
}
