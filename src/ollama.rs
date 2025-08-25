use serde::{Deserialize, Serialize};
use serde_json::json;

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
    // streaming responses have many chunks; we expect a single final response in our simple call
}

#[derive(Deserialize)]
struct ChatMessage {
    role: String,
    content: String,
}

pub fn summarize(base_url: &str, model: &str, system_prompt: &str, transcript: &str) -> Result<String, Error> {
    // Use Ollama chat API with a system + user content
    // Reference: POST /api/chat { model, messages: [{role: "system"|"user", content}] }
    let url = format!("{}/api/chat", base_url.trim_end_matches('/'));

    let messages = vec![
        Message { role: "system", content: system_prompt.to_string() },
        Message { role: "user", content: transcript.to_string() },
    ];

    let body = json!({
        "model": model,
        "messages": messages,
        // non-streaming for simplicity
        "stream": false
    });

    let response = minreq::post(url)
        .with_header("Content-Type", "application/json")
        .with_timeout(45)
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
    reply
        .message
        .map(|m| m.content)
        .filter(|s| !s.is_empty())
        .ok_or(Error::NoTextInResponse)
}
