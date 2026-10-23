//! When Laya misses its 1.5s limit, kwant routes by keywords. That has to show in the log at info, with
//! how long it waited, or nobody can tell how often Laya actually decides (laya-eval: 31 of 33 model
//! queries timed out on a 2-vCPU server, and the old log line was at debug).
use kwnt::db::models::Kwant;
use kwnt::router::LayaRouter;
use std::io::Write;
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
struct Logs(Arc<Mutex<Vec<u8>>>);

impl Write for Logs {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn bot(id: &str, title: &str) -> Kwant {
    Kwant {
        space_id: kwnt::db::spaces::DEFAULT_SPACE_ID.into(),
        id: id.into(),
        name: id.into(),
        title: title.into(),
        persona_prompt: String::new(),
        soul: String::new(),
        avatar_sprite: String::new(),
        driver_type: "api".into(),
        preferred_model: "auto".into(),
        temperature: 0.7,
        max_tokens: 4096,
        level: 1,
        turns_completed: 0,
        created_at: None,
    }
}

/// The ms value of the first `elapsed_ms=` in `line`.
fn elapsed(line: &str) -> u64 {
    let rest = &line[line.find("elapsed_ms=").expect(line) + "elapsed_ms=".len()..];
    rest.chars().take_while(char::is_ascii_digit).collect::<String>().parse().unwrap()
}

#[tokio::test]
async fn a_laya_timeout_is_logged_at_info_with_the_wait() {
    let logs = Logs::default();
    let sink = logs.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_ansi(false)
        .with_writer(move || sink.clone())
        .finish();
    tracing::subscriber::set_global_default(subscriber).unwrap();

    // A Laya that accepts the connection and never answers
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((sock, _)) = listener.accept().await {
            held.push(sock);
        }
    });
    let router = LayaRouter::new(url);

    let decision = router.route_prompt("Write a regex for ISO dates", "chat").await.unwrap();
    assert!(decision.is_fallback);
    let text = String::from_utf8_lossy(&logs.0.lock().unwrap()).into_owned();
    let line = text.lines().find(|l| l.contains("Laya didn't pick the model")).unwrap_or_else(|| panic!("no info line:\n{text}"));
    assert!(line.contains(" INFO ") && line.contains("timeout"), "{line}");
    assert!((1400..10_000).contains(&elapsed(line)), "{line}");

    let members = [bot("a", "Backend Engineer"), bot("b", "Designer")];
    let route = router.route_channel_query_detailed("hello", "general", "a", &members).await;
    assert!(route.fallback);
    let text = String::from_utf8_lossy(&logs.0.lock().unwrap()).into_owned();
    let line = text.lines().find(|l| l.contains("Laya didn't pick the bot")).unwrap_or_else(|| panic!("no info line:\n{text}"));
    assert!(line.contains(" INFO "), "{line}");
    assert!((1400..10_000).contains(&elapsed(line)), "{line}");
}
