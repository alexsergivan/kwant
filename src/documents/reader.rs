//! The one model call that reads a letter: no tools, the page images (and PDFs with no text) as
//! image and document blocks, the PDF text in the prompt. Claude only, like the zombie check: the
//! Messages API with the person's key (no `tools` field), or the signed-in `claude` CLI with every
//! tool, MCP server and customization off, in an empty folder, everything on stdin.
//!
//! Errors are kwant's own short reasons, never the letter's text.

use base64::Engine;
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(180);
/// The API model when the settings name none: it must see images.
pub const DEFAULT_API_MODEL: &str = "claude-sonnet-5-5";
const MAX_TOKENS: u32 = 4000;

/// One page handed to the model: an image, or a PDF with no text layer.
#[derive(Debug, Clone)]
pub struct Page {
    pub mime: String,
    pub bytes: Vec<u8>,
}

impl Page {
    /// The Messages API content block for the page.
    pub fn block(&self) -> serde_json::Value {
        let data = base64::engine::general_purpose::STANDARD.encode(&self.bytes);
        let kind = if self.mime == "application/pdf" { "document" } else { "image" };
        serde_json::json!({"type": kind, "source": {"type": "base64", "media_type": self.mime, "data": data}})
    }
}

/// Something that reads a letter with one toolless model call.
#[async_trait::async_trait]
pub trait LetterReader: Send + Sync {
    /// Who read it, for the record: `api:<model>`, `cli` or a test stub's name.
    fn name(&self) -> String;
    /// The model's raw answer. `Err` is a short reason, never the letter's text.
    async fn read(&self, system: &str, prompt: &str, pages: &[Page]) -> Result<String, String>;
}

/// The content of the one user message: the pages, then the prompt.
pub fn content(prompt: &str, pages: &[Page]) -> serde_json::Value {
    let mut blocks: Vec<serde_json::Value> = pages.iter().map(Page::block).collect();
    blocks.push(serde_json::json!({"type": "text", "text": prompt}));
    serde_json::Value::Array(blocks)
}

/// The Messages API with the person's key. The request has no `tools`.
pub struct ApiReader {
    http: reqwest::Client,
    url: String,
    key: zeroize::Zeroizing<String>,
    model: String,
}

impl ApiReader {
    pub fn new(base: &str, key: String, model: &str) -> Self {
        let http = reqwest::Client::builder()
            .timeout(TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap_or_default();
        let model = if model.trim().is_empty() || model == "claude" { DEFAULT_API_MODEL } else { model.trim() };
        Self { http, url: format!("{}/v1/messages", base.trim_end_matches('/')), key: key.into(), model: model.into() }
    }

    /// The request body (tested: it never has `tools`).
    pub fn body(&self, system: &str, prompt: &str, pages: &[Page]) -> serde_json::Value {
        serde_json::json!({
            "model": self.model,
            "max_tokens": MAX_TOKENS,
            "system": system,
            "messages": [{"role": "user", "content": content(prompt, pages)}],
        })
    }
}

#[async_trait::async_trait]
impl LetterReader for ApiReader {
    fn name(&self) -> String {
        format!("api:{}", self.model)
    }

    async fn read(&self, system: &str, prompt: &str, pages: &[Page]) -> Result<String, String> {
        let res = self
            .http
            .post(&self.url)
            .header("x-api-key", self.key.as_str())
            .header("anthropic-version", crate::drivers::api::ANTHROPIC_VERSION)
            .json(&self.body(system, prompt, pages))
            .send()
            .await
            .map_err(|e| format!("the model could not be reached: {}", e.without_url()))?;
        let status = res.status();
        if !status.is_success() {
            return Err(format!("the model API answered {}", status.as_u16()));
        }
        let v: serde_json::Value = res.json().await.map_err(|_| "the model API sent an unreadable answer".to_string())?;
        let text: String = v["content"]
            .as_array()
            .map(|parts| parts.iter().filter_map(|p| p["text"].as_str()).collect::<Vec<_>>().join(""))
            .unwrap_or_default();
        if text.is_empty() {
            return Err("the model API sent no text".into());
        }
        Ok(text)
    }
}

/// The signed-in `claude` CLI: one stream-json user message on stdin with the page blocks, every
/// tool, MCP server and customization off, run in an empty folder.
pub struct CliReader {
    bin: String,
    model: Option<String>,
}

impl CliReader {
    pub fn new(bin: &str, model: &str) -> Self {
        let model = model.trim();
        let model = (!model.is_empty() && model != "claude").then(|| model.to_string());
        Self { bin: bin.into(), model }
    }

    /// The arguments; the letter goes on stdin, never in argv (where `ps` would show it).
    pub fn args(&self, system: &str) -> Vec<String> {
        let mut a: Vec<String> = [
            "-p",
            "--tools",
            "",
            "--strict-mcp-config",
            "--safe-mode",
            "--no-session-persistence",
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--verbose",
            "--system-prompt",
            system,
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        if let Some(m) = &self.model {
            a.push("--model".into());
            a.push(m.clone());
        }
        a
    }

    /// The stdin line: one user message.
    pub fn stdin_line(prompt: &str, pages: &[Page]) -> String {
        let msg = serde_json::json!({"type": "user", "message": {"role": "user", "content": content(prompt, pages)}});
        format!("{msg}\n")
    }

    /// The model's text from the stream: the `result` event's `result`.
    pub fn result_of(stdout: &str) -> Result<String, String> {
        for line in stdout.lines().rev() {
            let Ok(v) = serde_json::from_str::<serde_json::Value>(line.trim()) else { continue };
            if v["type"] == "result" {
                if v["is_error"].as_bool() == Some(true) {
                    return Err("claude reported an error (signed out or out of quota?)".into());
                }
                return v["result"].as_str().map(str::to_string).ok_or_else(|| "claude sent no answer".to_string());
            }
        }
        Err("claude sent no answer".into())
    }
}

#[async_trait::async_trait]
impl LetterReader for CliReader {
    fn name(&self) -> String {
        "cli".into()
    }

    async fn read(&self, system: &str, prompt: &str, pages: &[Page]) -> Result<String, String> {
        use tokio::io::AsyncWriteExt;
        let dir = ScratchDir::new().map_err(|_| "could not make a scratch folder for reading".to_string())?;
        let mut child = tokio::process::Command::new(&self.bin)
            .args(self.args(system))
            .current_dir(&dir.0)
            .env("BROWSER", "none")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|_| "the claude command could not start".to_string())?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(Self::stdin_line(prompt, pages).as_bytes())
                .await
                .map_err(|_| "could not hand the letter to claude".to_string())?;
            // Dropping stdin closes it: the CLI answers and exits
        }
        let out = tokio::time::timeout(TIMEOUT, child.wait_with_output())
            .await
            .map_err(|_| "the claude command took too long".to_string())?
            .map_err(|_| "the claude command failed".to_string())?;
        if !out.status.success() {
            return Err(format!("the claude command exited with {}", out.status.code().unwrap_or(-1)));
        }
        Self::result_of(&String::from_utf8_lossy(&out.stdout))
    }
}

/// An empty folder for the CLI to run in, removed when dropped.
struct ScratchDir(std::path::PathBuf);

impl ScratchDir {
    fn new() -> std::io::Result<Self> {
        let p = std::env::temp_dir().join(format!("kwnt-letter-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir(&p)?;
        Ok(Self(p))
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The reader for this server: Claude through the API (key set) or the signed-in CLI, on the
/// Loose ends model setting when it names a Claude model. `Err` says why none can run.
pub async fn reader_for(state: &crate::server::AppState) -> Result<std::sync::Arc<dyn LetterReader>, String> {
    if let Some(r) = state.documents.reader.clone() {
        return Ok(r);
    }
    let settings = crate::zombie::settings::Settings::load(&state.db).await.unwrap_or_default();
    let model = settings.llm_model.trim();
    let model = if model.is_empty() || crate::profiles::Engine::from_model(model) != Some(crate::profiles::Engine::Claude) { "claude" } else { model };
    let key = state.db.get_setting(crate::profiles::Engine::Claude.key_setting()).await.ok().flatten();
    use crate::drivers::engines::SystemRoute;
    match crate::drivers::engines::system_route(&state.engines, model, key, crate::drivers::find_cli_binary)? {
        SystemRoute::Api { key, .. } => Ok(std::sync::Arc::new(ApiReader::new("https://api.anthropic.com", key, model))),
        SystemRoute::Cli { bin, .. } => Ok(std::sync::Arc::new(CliReader::new(&bin, model))),
        SystemRoute::Ollama => Err("reading letters needs Claude, which can see images".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png() -> Page {
        Page { mime: "image/png".into(), bytes: vec![0x89, b'P', b'N', b'G'] }
    }

    #[test]
    fn the_api_request_has_no_tools_and_carries_the_pages() {
        let r = ApiReader::new("https://api.example", "k".into(), "");
        let b = r.body("sys", "read it", &[png(), Page { mime: "application/pdf".into(), bytes: b"%PDF".to_vec() }]);
        assert!(b.get("tools").is_none());
        assert_eq!(b["model"], DEFAULT_API_MODEL);
        let content = b["messages"][0]["content"].as_array().unwrap();
        assert_eq!(content[0]["type"], "image");
        assert_eq!(content[0]["source"]["media_type"], "image/png");
        assert_eq!(content[1]["type"], "document");
        assert_eq!(content[2]["text"], "read it");
        assert_eq!(r.name(), format!("api:{DEFAULT_API_MODEL}"));
    }

    #[test]
    fn the_cli_runs_toolless_and_takes_the_letter_on_stdin() {
        let r = CliReader::new("/bin/claude", "claude-opus-5-5");
        let a = r.args("sys");
        let tools = a.iter().position(|x| x == "--tools").unwrap();
        assert_eq!(a[tools + 1], "");
        for flag in ["--strict-mcp-config", "--safe-mode", "--no-session-persistence"] {
            assert!(a.iter().any(|x| x == flag), "{flag}");
        }
        assert!(!a.iter().any(|x| x.contains("read it")));
        let line = CliReader::stdin_line("read it", &[png()]);
        let v: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(v["message"]["content"][0]["type"], "image");
        assert_eq!(v["message"]["content"][1]["text"], "read it");
    }

    #[test]
    fn the_cli_result_comes_from_the_result_event() {
        let out = "{\"type\":\"system\"}\n{\"type\":\"result\",\"is_error\":false,\"result\":\"{}\"}\n";
        assert_eq!(CliReader::result_of(out).unwrap(), "{}");
        assert!(CliReader::result_of("{\"type\":\"result\",\"is_error\":true}").is_err());
        assert!(CliReader::result_of("garbage").is_err());
    }
}
