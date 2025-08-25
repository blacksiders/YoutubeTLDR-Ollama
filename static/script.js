document.addEventListener('DOMContentLoaded', () => {
    const config = {
        baseURL: `${location.protocol}//${location.hostname}${location.port ? ':' + location.port : ''}`,
        storageKeys: {
            model: 'ollama-tldr-model',
            systemPrompt: 'ollama-tldr-system-prompt',
            dryRun: 'ollama-tldr-dry-run',
            transcriptOnly: 'ollama-tldr-transcript-only',
            summaries: 'ollama-tldr-summaries'
        },
          defaults: {
                model: 'gpt-oss:20b',
                systemPrompt: `You are an expert video summarizer. Given a raw YouTube transcript (and optionally the video title), produce a debate‑ready Markdown summary that captures the speaker’s core thesis, structure, and evidence without adding facts that aren’t in the transcript.

Tone and perspective:
- Use a neutral narrator voice: refer to the narrator as “the speaker” (e.g., “The speaker argues…”).
- Preserve the speaker’s stance and rhetoric, but do not editorialize or inject new claims.
- If something is not mentioned, say “Not mentioned” instead of guessing.

Output format (Markdown only):
1) Start with a punchy H2 title that captures the thesis.
    - Format: “## {Concise, compelling title reflecting the main claim}”
2) One short opening paragraph (2–3 sentences) that frames the overall argument.
3) 3–6 H3 sections with clear, descriptive headings that organize the content.
    - For each section:
      - 1–2 concise paragraphs.
      - Follow with bullet points using “* ”. Bold key terms and claims like **Bitcoin**, **employment**, **risk**, **status**, **leverage**, etc.
      - Where helpful, add a short numbered list (1.–3.) for steps/frameworks.
4) If the transcript includes critiques of alternatives or comparisons, include a separate section summarizing them (e.g., “### Critique of {X}”).
5) If practical steps are given, include a short “### Actionable Steps” section.
6) If risks, caveats, timelines, metrics, or quotes appear, preserve them verbatim (use inline quotes for short lines, blockquotes for longer).
7) End cleanly without a generic conclusion if it repeats content.

Style constraints:
- Use bold to highlight crucial terms and takeaways (not entire sentences).
- Keep factual fidelity: do not add numbers, timelines, or names that aren’t in the transcript.
- Prefer concrete details (figures, dates, specific names) when present.
- Remove ads/sponsors, filler, repeated phrases, and irrelevant tangents.
- Length target: ~300–700 words for typical videos; go longer only if the transcript is dense.

Safety/accuracy:
- If the transcript is incomplete or ambiguous, note “Not mentioned,” “Unclear,” or “Ambiguous” where appropriate.
- Do not invent references, links, or sources.`
          }
    };

    const dom = {
        model: document.getElementById('model'),
        systemPrompt: document.getElementById('system-prompt'),
        dryRun: document.getElementById('dry-run'),
        transcriptOnly: document.getElementById('transcript-only'),
        sidebar: document.getElementById('sidebar'),
        newSummaryBtn: document.getElementById('new-summary-btn'),
        savedSummariesList: document.getElementById('saved-summaries-list'),
        clearSummariesBtn: document.getElementById('clear-summaries-btn'),
        menuToggleBtn: document.getElementById('menu-toggle-btn'),
        closeSidebarBtn: document.getElementById('close-sidebar-btn'),
        sidebarOverlay: document.getElementById('sidebar-overlay'),
        mainContent: document.getElementById('main-content'),
        welcomeView: document.getElementById('welcome-view'),
        summaryView: document.getElementById('summary-view'),
        form: document.getElementById('summary-form'),
        urlInput: document.getElementById('youtube-url'),
        statusContainer: document.getElementById('status-container'),
        loader: document.getElementById('loader'),
        errorMessage: document.getElementById('error-message'),
        summaryContainer: document.getElementById('summary-container'),
        summaryTitleText: document.getElementById('summary-title-text'),
        summaryOutput: document.getElementById('summary-output'),
        transcriptSection: document.getElementById('transcript-section'),
        transcriptText: document.getElementById('transcript-text'),
        copySummaryBtn: document.getElementById('copy-summary-btn'),
        copyTranscriptBtn: document.getElementById('copy-transcript-btn'),
        videoLink: document.getElementById('video-link'),
        apiKeyContainer: document.getElementById('api-key-container')
    };

    const state = { summaries: [], activeSummaryIndex: -1, isLoading: false, error: null };

    const app = {
        init() {
            this.loadSettings();
            this.loadSummaries();
            this.addEventListeners();
            this.render();
            // Hide API key UI if present
            const ak = document.getElementById('api-key');
            if (ak && dom.apiKeyContainer) dom.apiKeyContainer.style.display = 'none';
            const akLabel = document.querySelector("label[for='api-key']");
            if (akLabel) akLabel.parentElement.style.display = 'none';
        },
        addEventListeners() {
            dom.form.addEventListener('submit', this.handleFormSubmit.bind(this));
            // Populate models datalist on load
            this.populateModels();
            dom.clearSummariesBtn.addEventListener('click', this.handleClearSummaries.bind(this));
            dom.newSummaryBtn.addEventListener('click', this.handleNewSummary.bind(this));
            dom.savedSummariesList.addEventListener('click', this.handleSidebarClick.bind(this));
            dom.copySummaryBtn.addEventListener('click', (e) => this.handleCopyClick(e, dom.summaryOutput.mdContent, dom.copySummaryBtn));
            dom.copyTranscriptBtn.addEventListener('click', (e) => this.handleCopyClick(e, dom.transcriptText.textContent, dom.copyTranscriptBtn));
            [dom.menuToggleBtn, dom.closeSidebarBtn, dom.sidebarOverlay].forEach(el => el && el.addEventListener('click', () => this.toggleSidebar()));
            [dom.model, dom.systemPrompt].forEach(el => el.addEventListener('change', this.saveSettings));
            [dom.dryRun, dom.transcriptOnly].forEach(el => el.addEventListener('change', this.saveSettings));
        },
        loadSummaries() {
            state.summaries = JSON.parse(localStorage.getItem(config.storageKeys.summaries)) || [];
            if (state.summaries.length > 0) state.activeSummaryIndex = 0;
        },
        saveSummaries() {
            localStorage.setItem(config.storageKeys.summaries, JSON.stringify(state.summaries));
            this.render();
        },
        loadSettings() {
            dom.model.value = localStorage.getItem(config.storageKeys.model) || config.defaults.model;
            dom.systemPrompt.value = localStorage.getItem(config.storageKeys.systemPrompt) || config.defaults.systemPrompt;
            dom.dryRun.checked = localStorage.getItem(config.storageKeys.dryRun) === 'true';
            dom.transcriptOnly.checked = localStorage.getItem(config.storageKeys.transcriptOnly) === 'true';
        },
        async populateModels() {
            try {
                const res = await fetch(`${config.baseURL}/api/models`);
                if (!res.ok) return;
                const data = await res.json();
                const models = Array.isArray(data.models) ? data.models : [];
                const dl = document.getElementById('models-list');
                if (!dl) return;
                dl.innerHTML = models.map(m => `<option value="${this.escapeHtml(m)}"></option>`).join('');
            } catch {}
        },
        saveSettings() {
            localStorage.setItem(config.storageKeys.model, dom.model.value);
            localStorage.setItem(config.storageKeys.systemPrompt, dom.systemPrompt.value);
            localStorage.setItem(config.storageKeys.dryRun, dom.dryRun.checked);
            localStorage.setItem(config.storageKeys.transcriptOnly, dom.transcriptOnly.checked);
        },
        async handleFormSubmit(event) {
            event.preventDefault();
            const url = dom.urlInput.value.trim();
            if (!url) { state.error = 'Please enter a YouTube URL.'; this.render(); return; }
            this.saveSettings();
            state.isLoading = true; state.error = null; state.activeSummaryIndex = -1; this.render();
            try {
                const response = await fetch(`${config.baseURL}/api/summarize`, {
                    method: 'POST', headers: { 'Content-Type': 'application/json' },
                    body: JSON.stringify({
                        url,
                        model: dom.model.value,
                        system_prompt: dom.systemPrompt.value,
                        dry_run: dom.dryRun.checked,
                        transcript_only: dom.transcriptOnly.checked,
                    }),
                });
                const txt = await response.text();
                if (!response.ok) throw new Error(txt || `Server error: ${response.status}`);
                const data = JSON.parse(txt);
                const newSummary = { name: data.video_name, summary: data.summary, transcript: data.subtitles, url };
                state.summaries.unshift(newSummary);
                state.activeSummaryIndex = 0;
            } catch (err) {
                console.error('Summarization failed:', err);
                state.error = err.message || String(err);
            } finally {
                state.isLoading = false; this.saveSummaries();
            }
        },
        handleNewSummary() { state.activeSummaryIndex = -1; state.error = null; dom.urlInput.value = ''; this.render(); if (this.isMobile()) this.toggleSidebar(false); },
        handleClearSummaries() { if (confirm('Clear all saved summaries?')) { state.summaries = []; state.activeSummaryIndex = -1; state.error = null; this.saveSummaries(); } },
        handleSidebarClick(e) {
            const link = e.target.closest('a[data-index]');
            const del = e.target.closest('button[data-index]');
            if (del) { e.preventDefault(); const i = parseInt(del.dataset.index, 10); this.deleteSummary(i); return; }
            if (link) { e.preventDefault(); state.activeSummaryIndex = parseInt(link.dataset.index, 10); state.error = null; this.render(); if (this.isMobile()) this.toggleSidebar(false); }
        },
        deleteSummary(i) {
            const s = state.summaries[i]; if (!s) return;
            if (confirm(`Delete summary for "${s.name}"?`)) {
                state.summaries.splice(i,1);
                if (state.activeSummaryIndex === i) { state.activeSummaryIndex = -1; state.error = null; }
                else if (state.activeSummaryIndex > i) { state.activeSummaryIndex--; }
                this.saveSummaries();
            }
        },
        render() {
            const hasActive = state.activeSummaryIndex > -1;
            const cur = hasActive ? state.summaries[state.activeSummaryIndex] : null;
            const showSummary = state.isLoading || hasActive || state.error;
            document.getElementById('welcome-view').classList.toggle('hidden', showSummary);
            document.getElementById('summary-view').classList.toggle('hidden', !showSummary);
            const hasStatus = state.isLoading || state.error;
            document.getElementById('status-container').classList.toggle('hidden', !hasStatus);
            document.getElementById('loader').style.display = state.isLoading ? 'flex' : 'none';
            const err = document.getElementById('error-message');
            err.style.display = state.error ? 'block' : 'none';
            err.textContent = state.error || '';
            document.getElementById('summary-container').classList.toggle('hidden', !cur || hasStatus);
            document.getElementById('transcript-section').classList.toggle('hidden', true);
            if (cur) {
                document.getElementById('summary-title-text').textContent = cur.name;
                document.getElementById('video-link').href = cur.url;
                document.getElementById('summary-output').mdContent = cur.summary;
                if (cur.transcript && cur.transcript.trim()) {
                    document.getElementById('transcript-text').textContent = cur.transcript;
                    document.getElementById('transcript-section').classList.remove('hidden');
                }
            }
            this.renderSidebarList();
        },
        renderSidebarList() {
            const list = document.getElementById('saved-summaries-list');
            list.innerHTML = state.summaries.map((s,i) => `
                <li class="${i===state.activeSummaryIndex?'active':''}">
                    <a href="#" data-index="${i}" title="${this.escapeHtml(s.name)}">
                        <i class="fas fa-file-alt"></i><span>${this.escapeHtml(s.name)}</span>
                    </a>
                    <button class="delete-summary-btn" data-index="${i}" title="Delete summary"><i class="fas fa-trash-alt"></i></button>
                </li>`).join('');
        },
        async handleCopyClick(e, text, btn) {
            e.preventDefault(); e.stopPropagation(); if (!text) return;
            const orig = btn.innerHTML, title = btn.title;
            try { await navigator.clipboard.writeText(text); btn.innerHTML = '<i class="fas fa-check"></i>'; btn.title = 'Copied!'; }
            catch { btn.title = 'Failed to copy'; }
            finally { setTimeout(()=>{ btn.innerHTML = orig; btn.title = title; }, 2000); }
        },
        isMobile: () => window.innerWidth <= 800,
        toggleSidebar(force) { document.body.classList.toggle('sidebar-open', force); document.getElementById('menu-toggle-btn').setAttribute('aria-expanded', document.body.classList.contains('sidebar-open')); },
        escapeHtml(str) { const p = document.createElement('p'); p.textContent = str; return p.innerHTML; }
    };

    app.init();
});

window.addEventListener('unhandledrejection', e => console.error('Unhandled rejection:', e.reason));
window.addEventListener('error', e => console.error('Uncaught error:', e.error));
