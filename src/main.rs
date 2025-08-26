mod ollama;
mod subtitle;

use crate::subtitle::get_video_data;
use crossbeam_channel::{bounded, Receiver, Sender};
use serde::{Deserialize, Serialize};
use std::env;
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::Duration;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Deserialize)]
struct SummarizeRequest {
    url: String,
    #[serde(default)]
    api_key: Option<String>,           // unused for Ollama; kept for UI compatibility
    #[serde(default)]
    model: Option<String>,             // ollama model name
    #[serde(default)]
    system_prompt: Option<String>,
    #[serde(default)]
    dry_run: bool,
    #[serde(default)]
    transcript_only: bool,
}

#[derive(Serialize, Clone)]
struct SummarizeResponse {
    summary: String,
    subtitles: String,
    video_name: String,
}

struct WorkItem {
    stream: TcpStream,
}

macro_rules! static_response {
    ($name:ident, $path:expr) => {
        static $name: &[u8] = include_bytes!(concat!("../static/", $path, ".gz"));
    };
}

static_response!(HTML_RESPONSE, "index.html");
static_response!(CSS_RESPONSE, "style.css");
static_response!(JS_RESPONSE, "script.js");

const READ_WRITE_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_HEADER_SIZE: usize = 8 * 1024; // 8 KB
const MAX_BODY_SIZE: usize = 10 * 1024 * 1024; // 10 MB

fn main() -> io::Result<()> {
    let ip = env::var("TLDR_IP").unwrap_or_else(|_| "0.0.0.0".into());
    let port = env::var("TLDR_PORT").unwrap_or_else(|_| "8001".into());
    let addr = format!("{}:{}", ip, port);

    let num_workers = env::var("TLDR_WORKERS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(4);

    let listener = TcpListener::bind(&addr)?;
    println!("✅ Ollama TLDR server at http://{}", addr);
    println!("✅ Spawning {} worker threads", num_workers);

    let (sender, receiver) = bounded(100);

    for id in 0..num_workers {
        let receiver = receiver.clone();
        thread::spawn(move || worker(id, receiver));
    }

    println!("▶️ Ready to accept requests");

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                if let Err(e) = handle_connection(stream, &sender) {
                    eprintln!("❌ Connection error: {}", e);
                }
            }
            Err(e) => eprintln!("❌ Accept failed: {}", e),
        }
    }
    Ok(())
}

fn handle_connection(stream: TcpStream, sender: &Sender<WorkItem>) -> io::Result<()> {
    stream.set_read_timeout(Some(READ_WRITE_TIMEOUT))?;
    stream.set_write_timeout(Some(READ_WRITE_TIMEOUT))?;

    let mut stream_clone = stream.try_clone()?;
    let work_item = WorkItem { stream };

    match sender.try_send(work_item) {
        Ok(()) => Ok(()),
        Err(crossbeam_channel::TrySendError::Full(_)) => {
            write_error_response(&mut stream_clone, "503 Service Unavailable", "Server is busy, please try again later.")
        }
        Err(crossbeam_channel::TrySendError::Disconnected(_)) => {
            write_error_response(&mut stream_clone, "500 Internal Server Error", "Worker pool has been disconnected.")
        }
    }
}

fn worker(id: usize, receiver: Receiver<WorkItem>) {
    println!("   Worker {} started", id);
    loop {
        match receiver.recv() {
            Ok(mut work_item) => {
                if let Err(e) = handle_request(&mut work_item.stream) {
                    eprintln!("❌ Worker {} error: {}", id, e);
                    let _ = write_error_response(&mut work_item.stream, "500 Internal Server Error", &e.to_string());
                }
            }
            Err(_) => {
                println!("   Worker {} shutting down", id);
                break;
            }
        }
    }
}

fn handle_request(stream: &mut TcpStream) -> io::Result<()> {
    let (headers, body_start_index) = read_headers_from_stream(stream)?;
    let request_data = &headers[..body_start_index];
    let initial_body = &headers[body_start_index..];

    let mut lines = request_data.split(|&b| b == b'\n').filter(|l| !l.is_empty());
    let request_line = lines.next().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Empty request"))?;

    if request_line.starts_with(b"GET ") {
        handle_get(request_line, stream)
    } else if request_line.starts_with(b"POST /api/summarize") {
        let content_length = get_content_length(request_data)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Content-Length header is required for POST"))?;

        if content_length > MAX_BODY_SIZE {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "Request body too large"));
        }

        let body = read_body(initial_body, content_length, stream)?;

        let req: SummarizeRequest = serde_json::from_slice(&body)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("JSON deserialization error: {}", e)))?;

        let response_payload = perform_summary_work(req)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("Processing error: {}", e)))?;

        let response_body = serde_json::to_string(&response_payload)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("JSON serialization error: {}", e)))?;

        write_response(stream, "200 OK", "application/json", response_body.as_bytes())
    } else if request_line.starts_with(b"POST /api/submit") {
        // background job submission
        let content_length = get_content_length(request_data)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Content-Length header is required for POST"))?;

        if content_length > MAX_BODY_SIZE { return Err(io::Error::new(io::ErrorKind::InvalidData, "Request body too large")); }

        let body = read_body(initial_body, content_length, stream)?;
        let req: SummarizeRequest = serde_json::from_slice(&body)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("JSON deserialization error: {}", e)))?;

    let job_id = new_job_id();
    insert_job_pending(&job_id);
    let job_id_for_thread = job_id.clone();
    thread::spawn(move || {
            let result = perform_summary_work(req);
            match result {
        Ok(resp) => {
            let val = serde_json::to_value(&resp).unwrap_or(serde_json::json!({"error":"serialization"}));
            set_job_done(&job_id_for_thread, val)
        },
        Err(err) => set_job_error(&job_id_for_thread, err),
            }
        });

    let body = serde_json::json!({"job_id": job_id}).to_string();
        write_response(stream, "200 OK", "application/json", body.as_bytes())
    } else if request_line.starts_with(b"POST /api/submit_script") {
        // background job to generate a YouTube script from an existing summary + transcript
        let content_length = get_content_length(request_data)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Content-Length header is required for POST"))?;
        if content_length > MAX_BODY_SIZE { return Err(io::Error::new(io::ErrorKind::InvalidData, "Request body too large")); }
        let body = read_body(initial_body, content_length, stream)?;
        let req: ScriptRequest = serde_json::from_slice(&body)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("JSON deserialization error: {}", e)))?;

        let job_id = new_job_id();
        insert_job_pending(&job_id);
        let job_id_for_thread = job_id.clone();
        thread::spawn(move || {
            let result = perform_script_work(req);
            match result {
                Ok(resp) => {
                    let val = serde_json::to_value(&resp).unwrap_or(serde_json::json!({"error":"serialization"}));
                    set_job_done(&job_id_for_thread, val)
                },
                Err(err) => set_job_error(&job_id_for_thread, err),
            }
        });

        let body = serde_json::json!({"job_id": job_id}).to_string();
        write_response(stream, "200 OK", "application/json", body.as_bytes())
    } else {
        write_error_response(stream, "404 Not Found", "Not Found")
    }
}

fn handle_get(request_line: &[u8], stream: &mut TcpStream) -> io::Result<()> {
    let path = request_line.split(|&b| b == b' ').nth(1).unwrap_or(b"/");
    // split off query string if present
    let (route, query) = match path.split(|&b| b == b'?').collect::<Vec<_>>().as_slice() {
        [r] => (*r, b"".as_slice()),
        [r, q, ..] => (*r, *q),
        _ => (path, b"".as_slice()),
    };

    match route {
        b"/" | b"/index.html" => write_static_response(stream, "text/html", HTML_RESPONSE),
        b"/style.css" => write_static_response(stream, "text/css", CSS_RESPONSE),
        b"/script.js" => write_static_response(stream, "application/javascript", JS_RESPONSE),
        b"/api/models" => {
            let body = get_ollama_models_json();
            write_response(stream, "200 OK", "application/json", body.as_bytes())
        }
        b"/api/job" => {
            let query_str = std::str::from_utf8(query).unwrap_or("");
            // accept either id=... or job_id=...
            let job_id = query_str
                .strip_prefix("id=")
                .or_else(|| query_str.strip_prefix("job_id="))
                .unwrap_or("");
            if job_id.is_empty() {
                return write_error_response(stream, "400 Bad Request", "Missing id parameter");
            }
            let body = get_job_status_json(job_id);
            write_response(stream, "200 OK", "application/json", body.as_bytes())
        }
        _ => write_error_response(stream, "404 Not Found", "Not Found"),
    }
}

fn perform_summary_work(req: SummarizeRequest) -> Result<SummarizeResponse, String> {
    if req.dry_run {
        let test_md = include_str!("./markdown_test.md").to_string();
        return Ok(SummarizeResponse {
            summary: test_md.clone(),
            subtitles: test_md,
            video_name: "Dry Run".into(),
        });
    }

    let (transcript, video_name) = get_video_data(&req.url, "en")
        .map_err(|e| format!("Transcript error: {}", e))?;

    if req.transcript_only {
        return Ok(SummarizeResponse {
            summary: transcript.clone(),
            subtitles: transcript,
            video_name,
        });
    }

    let model = req
        .model
        .filter(|m| !m.trim().is_empty())
        .unwrap_or_else(|| "gpt-oss:20b".to_string());
    let system_prompt = req.system_prompt.unwrap_or_else(|| r####"You are an expert video summarizer. Given a raw YouTube transcript (and optionally the video title), produce a debate-ready Markdown summary that captures the speaker's core thesis, structure, and evidence without adding facts that aren't in the transcript.

Tone and perspective:
- Use a neutral narrator voice: refer to the narrator as "the speaker" (e.g., "The speaker argues...").
- Preserve the speaker's stance and rhetoric, but do not editorialize or inject new claims.
- If something is not mentioned, say "Not mentioned" instead of guessing.

Output format (Markdown only):
1) Start with a punchy H2 title that captures the thesis.
    - Format: "## {Concise, compelling title reflecting the main claim}"
2) One short opening paragraph (2–3 sentences) that frames the overall argument.
3) 3–6 H3 sections with clear, descriptive headings that organize the content.
    - For each section:
      - 1–2 concise paragraphs.
      - Follow with bullet points using "* ". Bold key terms and claims like **Bitcoin**, **employment**, **risk**, **status**, **leverage**, etc.
      - Where helpful, add a short numbered list (1.–3.) for steps/frameworks.
4) If the transcript includes critiques of alternatives or comparisons, include a separate section summarizing them (e.g., "### Critique of {X}").
5) If practical steps are given, include a short "### Actionable Steps" section.
6) If risks, caveats, timelines, metrics, or quotes appear, preserve them verbatim (use inline quotes for short lines, blockquotes for longer).
7) End cleanly without a generic conclusion if it repeats content.

Style constraints:
- Do not use tables. Use headings, paragraphs, and bullet lists only.
- Use bold to highlight crucial terms and takeaways (not entire sentences).
- Keep factual fidelity: do not add numbers, timelines, or names that aren't in the transcript.
- Prefer concrete details (figures, dates, specific names) when present.
- Remove ads/sponsors, filler, repeated phrases, and irrelevant tangents.
- Length target: ~300–700 words for typical videos; go longer only if the transcript is dense.

Safety/accuracy:
- If the transcript is incomplete or ambiguous, note "Not mentioned," "Unclear," or "Ambiguous" where appropriate.
- Do not invent references, links, or sources.
- Do not give prescriptive financial, medical, or legal advice; only summarize what the speaker says."####
          .to_string());

    let base_url = env::var("OLLAMA_BASE_URL").unwrap_or_else(|_| "http://127.0.0.1:11434".to_string());

    let user_content = format!("Title: {}\n\nTranscript:\n{}", video_name, transcript);
    let summary = ollama::summarize(&base_url, &model, &system_prompt, &user_content)
        .map_err(|e| format!("Ollama error: {}", e))?;

    Ok(SummarizeResponse {
        summary,
        subtitles: transcript,
        video_name,
    })
}

fn read_headers_from_stream(stream: &mut TcpStream) -> io::Result<(Vec<u8>, usize)> {
    let mut buffer = Vec::with_capacity(1024);
    let mut chunk = [0; 256];
    loop {
        let bytes_read = stream.read(&mut chunk)?;
        if bytes_read == 0 {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "Connection closed while reading headers"));
        }
        buffer.extend_from_slice(&chunk[..bytes_read]);

        if let Some(pos) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
            let body_start_index = pos + 4;
            return Ok((buffer, body_start_index));
        }

        if buffer.len() > MAX_HEADER_SIZE {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "Headers too large"));
        }
    }
}

fn write_response(stream: &mut TcpStream, status: &str, content_type: &str, content: &[u8]) -> io::Result<()> {
    let headers = format!(
        "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        status,
        content_type,
        content.len()
    );
    stream.write_all(headers.as_bytes())?;
    stream.write_all(content)?;
    stream.flush()
}

fn write_static_response(stream: &mut TcpStream, content_type: &str, content: &[u8]) -> io::Result<()> {
    let headers = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Encoding: gzip\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        content_type,
        content.len()
    );
    stream.write_all(headers.as_bytes())?;
    stream.write_all(content)?;
    stream.flush()
}

fn write_error_response(stream: &mut TcpStream, status: &str, msg: &str) -> io::Result<()> {
    write_response(stream, status, "text/plain; charset=utf-8", msg.as_bytes())
}

fn get_content_length(headers: &[u8]) -> Option<usize> {
    let headers_str = std::str::from_utf8(headers).ok()?;
    for line in headers_str.lines() {
        if line.to_ascii_lowercase().starts_with("content-length:") {
            return line.split(':').nth(1)?.trim().parse().ok();
        }
    }
    None
}

fn read_body(
    initial_data: &[u8],
    content_length: usize,
    stream: &mut TcpStream,
) -> io::Result<Vec<u8>> {
    let mut body = Vec::with_capacity(content_length);
    body.extend_from_slice(initial_data);

    let remaining_bytes = content_length.saturating_sub(initial_data.len());

    if remaining_bytes > 0 {
        let mut remaining_body_reader = stream.take(remaining_bytes as u64);
        remaining_body_reader.read_to_end(&mut body)?;
    }

    Ok(body)
}

fn get_ollama_models_json() -> String {
    let base_url = env::var("OLLAMA_BASE_URL").unwrap_or_else(|_| "http://127.0.0.1:11434".to_string());
    let tags_url = format!("{}/api/tags", base_url.trim_end_matches('/'));
    let mut names: Vec<String> = Vec::new();
    if let Ok(resp) = minreq::get(tags_url).with_timeout(5).send() {
        if resp.status_code >= 200 && resp.status_code < 300 {
            if let Ok(v) = resp.json::<serde_json::Value>() {
                if let Some(arr) = v.get("models").and_then(|m| m.as_array()) {
                    for item in arr {
                        if let Some(name) = item.get("name").and_then(|n| n.as_str()) {
                            names.push(name.to_string());
                        }
                    }
                }
            }
        }
    }
    serde_json::to_string(&serde_json::json!({ "models": names })).unwrap_or_else(|_| "{\"models\":[]}".to_string())
}

// ---------- Background Jobs ----------

#[derive(Serialize, Clone)]
#[serde(tag = "status", rename_all = "lowercase")]
enum JobStatus {
    Pending,
    Done { result: serde_json::Value },
    Error { error: String },
}

static JOBS: OnceLock<Mutex<HashMap<String, JobStatus>>> = OnceLock::new();
static JOB_COUNTER: AtomicU64 = AtomicU64::new(1);

fn jobs_map() -> &'static Mutex<HashMap<String, JobStatus>> {
    JOBS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn new_job_id() -> String {
    let ts = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis();
    let n = JOB_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("job-{}-{}", ts, n)
}

fn insert_job_pending(id: &str) {
    if let Ok(mut map) = jobs_map().lock() {
        map.insert(id.to_string(), JobStatus::Pending);
    }
}

fn set_job_done(id: &str, result: serde_json::Value) {
    if let Ok(mut map) = jobs_map().lock() {
        map.insert(id.to_string(), JobStatus::Done { result });
    }
}

fn set_job_error(id: &str, error: String) {
    if let Ok(mut map) = jobs_map().lock() {
        map.insert(id.to_string(), JobStatus::Error { error });
    }
}

// ---------- Script generation ----------

#[derive(Deserialize)]
struct ScriptRequest {
    summary: String,
    #[serde(default)]
    transcript: String,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    system_prompt: Option<String>,
    #[serde(default)]
    video_name: Option<String>,
}

#[derive(Serialize, Clone)]
struct ScriptResponse {
    script: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    video_name: Option<String>,
}

fn default_script_system_prompt() -> String {
    r####"You are a professional YouTube scriptwriter. Using ONLY the provided summary (and transcript for fidelity), write a complete voiceover script that could be read verbatim. Do not invent facts; if details are missing, indicate "Not mentioned". Keep the speaker's stance neutral unless the transcript clearly states otherwise.

Output constraints:
- Markdown only. No tables.
- Structure the script with clear sections and smooth transitions:
  1) ## Hook — a compelling opener (2–4 sentences) that frames the core claim.
  2) ## Setup — brief context and what viewers will get.
  3) 3–6 sections (### {short heading}) delivering the content step by step.
     - Use short, spoken-language sentences.
     - Where relevant, include on-screen text cues in [brackets] and quick B‑roll suggestions in (parentheses).
  4) ## Recap — summarize the key ideas without adding new claims.
  5) ## CTA (if present in transcript) — reflect the speaker’s authentic call-to-action; otherwise omit.

Style rules:
- Concise, conversational voice; avoid fluff and filler.
- Do not add financial, medical, or legal advice; only reflect what’s in the transcript.
- Prefer concrete details; if ambiguous or missing, say "Not mentioned".
"####
        .to_string()
}

fn perform_script_work(req: ScriptRequest) -> Result<ScriptResponse, String> {
    let model = req
        .model
        .filter(|m| !m.trim().is_empty())
        .unwrap_or_else(|| "gpt-oss:20b".to_string());
    let system_prompt = req
        .system_prompt
        .unwrap_or_else(default_script_system_prompt);

    let base_url = env::var("OLLAMA_BASE_URL").unwrap_or_else(|_| "http://127.0.0.1:11434".to_string());

    // Provide both the summary and transcript for fidelity; the model is instructed to prioritize summary structure but not invent facts.
    let user_content = format!(
        "Video Title: {title}\n\nSummary:\n{summary}\n\nTranscript (for fidelity):\n{transcript}",
        title = req.video_name.as_deref().unwrap_or("(Unknown)"),
        summary = req.summary,
        transcript = req.transcript
    );

    let script = crate::ollama::summarize(&base_url, &model, &system_prompt, &user_content)
        .map_err(|e| format!("Ollama error: {}", e))?;

    Ok(ScriptResponse { script, video_name: req.video_name })
}

fn get_job_status_json(id: &str) -> String {
    let status = if let Ok(map) = jobs_map().lock() { map.get(id).cloned() } else { None };
    match status {
        Some(s) => serde_json::to_string(&s).unwrap_or_else(|_| "{\"status\":\"error\",\"error\":\"serialization\"}".to_string()),
        None => "{\"status\":\"error\",\"error\":\"not_found\"}".to_string(),
    }
}
