use serde::{Deserialize, Serialize};
use serde_json::json;
use std::env;

#[derive(Debug)]
pub enum Error {
    Request(minreq::Error),
    StatusNotOk(String),
    NoTextInResponse,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for Error {}

#[derive(Serialize)]
struct Message {
    role: &'static str,
    content: String,
}

#[derive(Deserialize)]
struct ChatResponse {
    message: Option<ChatMessage>,
    // When non-streaming, Ollama includes whether the generation stopped due to length, stop token, etc.
    #[serde(default)]
    done_reason: Option<String>,
    // streaming responses have many chunks; we expect a single final response in our simple call
}

#[derive(Deserialize)]
struct ChatMessage {
    role: String,
    content: String,
}

pub fn summarize(base_url: &str, model: &str, system_prompt: &str, transcript: &str) -> Result<String, Error> {
    // Use Ollama chat API with a system + user content. If output is truncated due to length,
    // automatically issue continuation turns and concatenate results.
    let url = format!("{}/api/chat", base_url.trim_end_matches('/'));

    // Tunables via env vars with sensible defaults
    let num_predict: i64 = env::var("OLLAMA_NUM_PREDICT").ok().and_then(|s| s.parse().ok()).unwrap_or(1200);
    let num_ctx: i64 = env::var("OLLAMA_NUM_CTX").ok().and_then(|s| s.parse().ok()).unwrap_or(8192);
    let temperature: f64 = env::var("OLLAMA_TEMPERATURE").ok().and_then(|s| s.parse().ok()).unwrap_or(0.2);
    let repeat_penalty: f64 = env::var("OLLAMA_REPEAT_PENALTY").ok().and_then(|s| s.parse().ok()).unwrap_or(1.1);
    let max_cont: u32 = env::var("OLLAMA_AUTO_CONT_MAX").ok().and_then(|s| s.parse().ok()).unwrap_or(2);

    // Timeout handling: OLLAMA_TIMEOUT_SECS
    // - unset or 0 => no timeout (wait indefinitely)
    // - >0 => apply that many seconds
    let timeout_opt: Option<u64> = env::var("OLLAMA_TIMEOUT_SECS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .filter(|&v| v > 0);

    // Message history for chat
    let mut messages = vec![
        Message { role: "system", content: system_prompt.to_string() },
        Message { role: "user", content: transcript.to_string() },
    ];

    let mut accumulated = String::new();
    let mut turns = 0u32;
    loop {
        turns += 1;
        let body = json!({
            "model": model,
            "messages": messages,
            "options": {
                "temperature": temperature,
                "repeat_penalty": repeat_penalty,
                "num_ctx": num_ctx,
                "num_predict": num_predict
            },
            "stream": false
        });

        let mut req = minreq::post(&url).with_header("Content-Type", "application/json");
        if let Some(secs) = timeout_opt { req = req.with_timeout(secs); }

        let response = req
            .with_json(&body)
            .map_err(Error::Request)?
            .send()
            .map_err(Error::Request)?;

        if response.status_code < 200 || response.status_code > 299 {
            let text = response.as_str().unwrap_or("").to_string();
            // Special-case common error: model not found. Try to suggest installed models.
            if text.contains("not found") || response.status_code == 404 {
                // Query /api/tags for installed models to help the user pick a valid one.
                let tags_url = format!("{}/api/tags", base_url.trim_end_matches('/'));
                if let Ok(tags_resp) = minreq::get(tags_url).with_timeout(5).send() {
                    if tags_resp.status_code >= 200 && tags_resp.status_code <= 299 {
                        if let Ok(v) = tags_resp.json::<serde_json::Value>() {
                            let names: Vec<String> = v
                                .get("models")
                                .and_then(|m| m.as_array())
                                .map(|arr| {
                                    arr.iter()
                                        .filter_map(|item| item.get("name").and_then(|n| n.as_str()).map(|s| s.to_string()))
                                        .collect::<Vec<_>>()
                                })
                                .unwrap_or_default();
                            let suggestion = if names.is_empty() {
                                String::from("No local models found. Pull one, e.g.: ollama pull llama3:8b")
                            } else {
                                format!("Installed models: {}", names.join(", "))
                            };
                            let friendly = format!(
                                "Model '{model}' not found. Pull it with: ollama pull {model}. {suggestion}",
                                model = model,
                                suggestion = suggestion
                            );
                            return Err(Error::StatusNotOk(friendly));
                        }
                    }
                }
            }
            return Err(Error::StatusNotOk(text));
        }

        let reply: ChatResponse = response.json().map_err(Error::Request)?;
        let chunk = reply
            .message
            .map(|m| m.content)
            .filter(|s| !s.is_empty())
            .ok_or(Error::NoTextInResponse)?;
        accumulated.push_str(&chunk);

        let truncated = reply.done_reason.as_deref() == Some("length");
        if !truncated || turns > max_cont { break; }

        // Add assistant content and request a continuation
        messages.push(Message { role: "assistant", content: chunk });
        messages.push(Message { role: "user", content: "Continue. Finish any unfinished sections, bullets, and examples. Maintain the same formatting.".to_string() });
    }

    Ok(accumulated)
}
