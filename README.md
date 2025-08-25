# YouTubeTLDR (Ollama)

A minimal, self-hosted YouTube video summarizer using your local Ollama models. It reuses the same tiny Rust HTTP server from the original project, swaps Gemini calls for the Ollama chat API, and serves a simple UI.

## Prerequisites
- Rust nightly (see `rust-toolchain.toml`)
- Ollama installed and running (default at http://127.0.0.1:11434)
- Pull at least one model, e.g.:
  - `ollama pull llama3:8b` (text summarization)
  - For vision (frames), integrate separately; this tool uses transcript-only summarization.

## Build
```
cargo build --release
```
Binary at `target/release/YouTubeTLDR-Ollama`.

## Run
Windows (PowerShell):
```
$env:TLDR_IP='127.0.0.1'
$env:TLDR_PORT='8001'
$env:TLDR_WORKERS='4'
$env:OLLAMA_BASE_URL='http://127.0.0.1:11434'
./target/release/YouTubeTLDR-Ollama.exe
```

macOS/Linux:
```
TLDR_IP=127.0.0.1 TLDR_PORT=8001 TLDR_WORKERS=4 OLLAMA_BASE_URL=http://127.0.0.1:11434 \
  ./target/release/YouTubeTLDR-Ollama
```
Open http://127.0.0.1:8001 and use the Settings to pick an Ollama model (default `llama3:8b`).

## Notes
- This version summarizes from the YouTube transcript only.
- To add frame-aware reasoning, extend with frame extraction and a vision model (e.g., `qwen2-vl:7b`).
