//! The export prompt: what a person sends their bot in another app, so the bot replies with a
//! `kwant.bundle/1` they can paste back. This is the one source of truth for the text; the
//! Import dialog gets it from `GET /api/import/prompts`.

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct ExportPrompt {
    /// `grok`, `chatgpt`, `claude`, `other`.
    pub id: &'static str,
    /// "Bring a bot from …"
    pub label: &'static str,
    /// Where the person sends the prompt, in one sentence.
    pub where_to_send: &'static str,
    pub prompt: String,
}

/// In the order the "Bring a bot from…" list shows them.
const SOURCES: [(&str, &str, &str, &str); 4] = [
    (
        "grok",
        "Grok",
        "Open the Grok Project or Grok bot you want to bring over and send it this message in a new chat.",
        "grok",
    ),
    (
        "chatgpt",
        "ChatGPT",
        "Open your custom GPT (or the ChatGPT Project) and send it this message in a new chat. For a custom GPT you edit yourself, you can instead copy its Instructions field from the GPT editor and paste that here; it becomes one bot's instructions.",
        "chatgpt",
    ),
    (
        "claude",
        "Claude",
        "Open the Claude Project you want to bring over and send it this message in a new chat in that project.",
        "claude",
    ),
    ("other", "Another app", "Open the bot or assistant you want to bring over and send it this message.", "other"),
];

const TEMPLATE: &str = r#"I'm moving you to another app called kwant. Please describe yourself so I can recreate you there.

Reply with exactly one JSON code block and nothing else, in this shape:

{
  "format": "kwant.bundle/1",
  "source": { "kind": "{{KIND}}", "label": "<your name in {{APP}}>" },
  "bots": [
    {
      "ref": "main",
      "name": "<your name, one word>",
      "title": "<your role in a few words>",
      "persona_prompt": "<your full instructions / system prompt, WORD FOR WORD, not a summary>",
      "soul": "<your personality and tone, if it isn't already in the instructions>",
      "starters": ["<conversation starters, if you have any>"],
      "memories": ["<each thing you remember about me, one fact per item>"],
      "skills": [
        { "name": "<short-name>", "description": "<one line: when to use it>", "body": "<the steps, word for word>" }
      ],
      "routines": [
        { "name": "<scheduled task name>", "instructions": "<what to do each time>", "schedule": { "type": "cron", "expression": "<five-field cron line like 0 9 * * 1-5>", "tz": "<time zone like Europe/Berlin, or UTC>" }, "schedule_text": "<the schedule in your own words, only if it can't be written as cron>" }
      ],
      "tool_hints": ["<tools, actions, plugins or connected apps you use>"],
      "knowledge_files": ["<names of files you were given>"]
    }
  ],
  "omitted": ["<anything you have that doesn't fit above>"]
}

Rules:
- Copy your instructions exactly as written. Don't shorten, paraphrase or improve them.
- Leave out empty lists rather than inventing content.
- Write each routine's schedule as a cron line with its time zone (an IANA name like Europe/Berlin), not as a sentence.
- Never include passwords, API keys, tokens or other secrets. Write [secret] instead.
- Don't include our past conversations.
- Use straight JSON: double quotes, no comments, no trailing commas."#;

fn app_name(id: &str) -> &'static str {
    match id {
        "grok" => "Grok",
        "chatgpt" => "ChatGPT",
        "claude" => "Claude",
        _ => "this app",
    }
}

/// Every source's prompt, in display order.
pub fn export_prompts() -> Vec<ExportPrompt> {
    SOURCES
        .iter()
        .map(|(id, label, where_to_send, kind)| ExportPrompt {
            id,
            label,
            where_to_send,
            prompt: TEMPLATE.replace("{{KIND}}", kind).replace("{{APP}}", app_name(id)),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompts_start_with_grok_chatgpt_claude() {
        let ids: Vec<&str> = export_prompts().iter().map(|p| p.id).collect();
        assert_eq!(ids, ["grok", "chatgpt", "claude", "other"]);
        assert!(export_prompts().iter().all(|p| !p.prompt.contains("{{")));
    }

    #[test]
    fn prompt_example_is_a_valid_bundle_shape() {
        let p = &export_prompts()[0].prompt;
        let json = crate::import::parsers::json_block(p).unwrap();
        let b: crate::import::Bundle = serde_json::from_str(json).unwrap();
        assert_eq!(b.format, crate::import::BUNDLE_FORMAT);
        assert_eq!(b.source.kind, "grok");
    }
}
