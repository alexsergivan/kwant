use super::ToolError;
use reqwest::Client;
use serde_json::{json, Value};
use std::time::Duration;

pub async fn web_fetch(args: &Value) -> Result<Value, ToolError> {
    let url = args["url"]
        .as_str()
        .ok_or_else(|| ToolError::InvalidArgs("Missing 'url' argument".into()))?;

    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| ToolError::Execution(e.to_string()))?;

    let resp = client
        .get(url)
        .header("User-Agent", "KwantsBot/1.0 (Multi-Agent-Platform)")
        .send()
        .await
        .map_err(|e| ToolError::Execution(format!("Web request failed: {}", e)))?;

    let status = resp.status().as_u16();
    let body = resp
        .text()
        .await
        .map_err(|e| ToolError::Execution(format!("Failed reading web response: {}", e)))?;

    // Basic HTML tag stripping and whitespace normalization
    let stripped = strip_html_tags(&body);
    let truncated = if stripped.len() > 8000 {
        format!("{}... [content truncated]", &stripped[..8000])
    } else {
        stripped
    };

    Ok(json!({
        "url": url,
        "status": status,
        "content": truncated
    }))
}

fn strip_html_tags(html: &str) -> String {
    let mut in_tag = false;
    let mut output = String::with_capacity(html.len());

    for ch in html.chars() {
        if ch == '<' {
            in_tag = true;
        } else if ch == '>' {
            in_tag = false;
            output.push(' ');
        } else if !in_tag {
            output.push(ch);
        }
    }

    // Collapse multiple whitespaces
    output
        .split_whitespace()
        .collect::<Vec<&str>>()
        .join(" ")
}
