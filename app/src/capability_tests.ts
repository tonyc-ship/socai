//! Three capability tests shipped with the desktop app — CDP connect,
//! XHS tool calls, agent task. All three live in this one file because
//! they're closely related and small enough that splitting them per panel
//! would add more navigation cost than it saves.
//!
//! The shell in `main.ts` calls into the `cdpPanel` / `toolsPanel` /
//! `agentPanel` namespaces below; each owns its own local state and
//! exposes `render(shell)` + `bind(shell)`.

import { invoke } from "@tauri-apps/api/core";
import type { AgentEventPayload, ModelInfo, ShellState, TargetInfo } from "./main";

interface AgentOutcome {
  run_id: string;
  run_dir: string;
  turns: number;
  final_text: string;
  input_tokens: number;
  output_tokens: number;
}

export function esc(s: string): string {
  return s.replace(/[<>&"']/g, (c) => {
    return (
      { "<": "&lt;", ">": "&gt;", "&": "&amp;", '"': "&quot;", "'": "&#39;" } as Record<string, string>
    )[c];
  });
}

function truncate(s: string, n: number): string {
  return s.length > n ? s.slice(0, n - 1) + "…" : s;
}

// ── CDP connect smoke test ────────────────────────────────────────────────

export namespace cdpPanel {
  let cdpQuery = "";
  let cdpStatusText = "";
  let cdpInFlight = false;

  export function reset(): void {
    cdpStatusText = "";
  }

  export function render(shell: ShellState): string {
    const tabList = shell.status.state === "connected" && shell.pages.length > 0
      ? `<ul class="tab-list">${shell.pages.map(renderTab).join("")}</ul>`
      : "";

    if (shell.status.state === "connected") {
      return `
        <form id="cdp-form" class="row-form">
          <input
            id="cdp-input"
            class="input-field"
            type="text"
            placeholder="google search…"
            value="${esc(cdpQuery)}"
            autocomplete="off"
            ${cdpInFlight ? "disabled" : ""}
          />
          <button type="submit" class="btn-primary" ${cdpInFlight ? "disabled" : ""}>
            ${cdpInFlight ? "running…" : "search"}
          </button>
          <button id="cdp-disconnect" type="button" class="btn-ghost">disconnect</button>
        </form>
        ${cdpStatusText ? `<p class="t-small status-line">${esc(cdpStatusText)}</p>` : ""}
        ${tabList}
      `;
    }

    return `
      <div class="row-form">
        <button id="cdp-connect" class="btn-primary" ${shell.status.state === "connecting" ? "disabled" : ""}>
          ${shell.status.state === "connecting" ? "connecting…" : "connect"}
        </button>
      </div>
    `;
  }

  export function bind(shell: ShellState): void {
    document.getElementById("cdp-connect")?.addEventListener("click", () => {
      invoke("cdp_connect").catch((e) => console.error("cdp_connect failed:", e));
    });
    document.getElementById("cdp-disconnect")?.addEventListener("click", () => {
      invoke("cdp_disconnect").catch((e) => console.error("cdp_disconnect failed:", e));
    });
    const input = document.getElementById("cdp-input") as HTMLInputElement | null;
    input?.addEventListener("input", () => { cdpQuery = input.value; });
    document.getElementById("cdp-form")?.addEventListener("submit", async (e) => {
      e.preventDefault();
      const q = cdpQuery.trim();
      if (!q || cdpInFlight) return;
      cdpInFlight = true;
      cdpStatusText = `opening tab and searching for "${q}"…`;
      shell.rerender();
      try {
        cdpStatusText = await invoke<string>("cdp_test_search", { query: q });
      } catch (err) {
        cdpStatusText = `error: ${err}`;
      } finally {
        cdpInFlight = false;
        shell.rerender();
      }
    });
  }

  function renderTab(t: TargetInfo): string {
    return `
      <li class="tab-row">
        <div class="tab-title t-body">${esc(t.title || "(untitled)")}</div>
        <div class="tab-url t-mono">${esc(truncate(t.url, 90))}</div>
      </li>
    `;
  }
}

// ── XHS tool calls (search_notes / topic_scan / extract_note) ─────────────

export namespace toolsPanel {
  type ToolCommand = "search_notes" | "topic_scan" | "extract_note";

  interface RunningState {
    cmd: ToolCommand;
    preview: string;
  }

  let searchQuery = "";
  let topicQuery = "";
  let extractNoteId = "";
  let active: RunningState | null = null;
  let result: unknown = null;
  let errorText = "";

  export function render(shell: ShellState): string {
    const connected = shell.status.state === "connected";
    const disabled = (cmd: ToolCommand): string => {
      if (!connected) return "disabled";
      return active && active.cmd !== cmd ? "disabled" : "";
    };
    const running = (cmd: ToolCommand): boolean => active?.cmd === cmd;
    const guard = connected ? "" : `<p class="t-small subtle">connect chrome first.</p>`;

    return `
      ${guard}
      <div class="tool-grid">
        <form id="tool-search-form" class="row-form">
          <span class="tool-name t-mono">search_notes</span>
          <input id="tool-search-input" class="input-field" placeholder="query" value="${esc(searchQuery)}" ${disabled("search_notes")} />
          <button type="submit" class="btn-primary" ${disabled("search_notes")}>
            ${running("search_notes") ? "running…" : "run"}
          </button>
        </form>

        <form id="tool-topic-form" class="row-form">
          <span class="tool-name t-mono">topic_scan</span>
          <input id="tool-topic-input" class="input-field" placeholder="query" value="${esc(topicQuery)}" ${disabled("topic_scan")} />
          <button type="submit" class="btn-primary" ${disabled("topic_scan")}>
            ${running("topic_scan") ? "running…" : "run"}
          </button>
        </form>

        <form id="tool-extract-form" class="row-form">
          <span class="tool-name t-mono">extract_note</span>
          <input id="tool-extract-input" class="input-field" placeholder="note_id" value="${esc(extractNoteId)}" ${disabled("extract_note")} />
          <button type="submit" class="btn-primary" ${disabled("extract_note")}>
            ${running("extract_note") ? "running…" : "run"}
          </button>
        </form>
      </div>

      <div class="result-block">
        ${renderResult()}
      </div>
    `;
  }

  function renderResult(): string {
    if (active) {
      return `<pre class="result-pre result-running">running: ${esc(active.preview)}</pre>`;
    }
    if (errorText) {
      return `<pre class="result-pre result-error">${esc(errorText)}</pre>`;
    }
    if (result) {
      return `<pre class="result-pre">${esc(JSON.stringify(result, null, 2))}</pre>`;
    }
    return `<p class="t-small placeholder">no result yet.</p>`;
  }

  export function bind(shell: ShellState): void {
    const sInput = document.getElementById("tool-search-input") as HTMLInputElement | null;
    sInput?.addEventListener("input", () => { searchQuery = sInput.value; });
    document.getElementById("tool-search-form")?.addEventListener("submit", async (e) => {
      e.preventDefault();
      const q = searchQuery.trim();
      if (!q) return;
      await runTool(shell, {
        cmd: "search_notes",
        preview: `search_notes(query=${JSON.stringify(q)})`,
      }, () => invoke("tool_search_notes", { query: q }));
    });

    const tInput = document.getElementById("tool-topic-input") as HTMLInputElement | null;
    tInput?.addEventListener("input", () => { topicQuery = tInput.value; });
    document.getElementById("tool-topic-form")?.addEventListener("submit", async (e) => {
      e.preventDefault();
      const q = topicQuery.trim();
      if (!q) return;
      await runTool(shell, {
        cmd: "topic_scan",
        preview: `topic_scan(query=${JSON.stringify(q)})`,
      }, () => invoke("tool_topic_scan", { query: q }));
    });

    const eInput = document.getElementById("tool-extract-input") as HTMLInputElement | null;
    eInput?.addEventListener("input", () => { extractNoteId = eInput.value; });
    document.getElementById("tool-extract-form")?.addEventListener("submit", async (e) => {
      e.preventDefault();
      const id = extractNoteId.trim();
      if (!id) return;
      await runTool(shell, {
        cmd: "extract_note",
        preview: `extract_note(note_id=${JSON.stringify(id)})`,
      }, () => invoke("tool_extract_note", { noteId: id }));
    });
  }

  async function runTool(
    shell: ShellState,
    state: RunningState,
    action: () => Promise<unknown>,
  ): Promise<void> {
    if (active) return;
    active = state;
    errorText = "";
    result = null;
    shell.rerender();
    try {
      result = await action();
    } catch (err) {
      errorText = `${err}`;
    } finally {
      active = null;
      shell.rerender();
    }
  }
}

// ── Agent task ─────────────────────────────────────────────────────────────

export namespace agentPanel {
  let task = "";
  let model = "";
  let inFlight = false;
  let events: AgentEventPayload[] = [];
  let outcome: AgentOutcome | null = null;
  let errorText = "";
  let modelsCache: ModelInfo[] = [];

  // Key-entry sub-state — only visible when the active model lacks a key.
  let pendingKey = "";
  let savingKey = false;
  let keyError = "";

  export function setModels(models: ModelInfo[]): void {
    modelsCache = models;
    if (!model) {
      const withKey = models.find((m) => m.has_key);
      model = (withKey ?? models[0])?.default_model ?? "";
    }
  }

  /// Append a streamed event AND incrementally update the DOM so we don't
  /// re-render the entire page on every chunk. Pins scroll-to-bottom.
  export function appendEvent(payload: AgentEventPayload): void {
    events = [...events, payload];

    const stream = document.querySelector<HTMLDivElement>("[data-agent-events]");
    if (!stream) return;

    const placeholder = stream.querySelector("[data-events-placeholder]");
    if (placeholder) placeholder.remove();

    stream.insertAdjacentHTML("beforeend", renderAgentEvent(payload));
    stream.scrollTop = stream.scrollHeight;
  }

  export function render(shell: ShellState): string {
    const connected = shell.status.state === "connected";
    const selected = modelsCache.find((m) => m.default_model === model);
    const keylessSelected = !!selected && !selected.has_key;
    const formDisabled = inFlight || !connected;
    const runDisabled = formDisabled || keylessSelected;

    const modelOpts = modelsCache
      .map((m) => {
        const sel = model === m.default_model ? "selected" : "";
        const flag = m.has_key ? "" : " · no key";
        return `<option value="${esc(m.default_model)}" ${sel}>${esc(m.display_name)} — ${esc(m.default_model)}${flag}</option>`;
      })
      .join("");

    const guard = connected ? "" : `<p class="t-small subtle">connect chrome first.</p>`;

    return `
      ${guard}
      <form id="agent-form" class="agent-form">
        <textarea
          id="agent-task"
          class="input-field input-textarea"
          rows="3"
          placeholder="task"
          ${formDisabled ? "disabled" : ""}
        >${esc(task)}</textarea>

        <div class="row-form agent-row">
          <select id="agent-model" class="input-field" ${inFlight ? "disabled" : ""}>
            ${modelOpts || `<option value="">(loading…)</option>`}
          </select>
          <button type="submit" class="btn-primary" ${runDisabled ? "disabled" : ""}>
            ${inFlight ? "running…" : "run"}
          </button>
        </div>

        ${keylessSelected ? renderKeyEntry(selected!.provider, selected!.display_name) : ""}
      </form>

      <p class="t-eyebrow result-label">result</p>
      <div class="result-block">
        ${
          errorText
            ? `<pre class="result-pre result-error">${esc(errorText)}</pre>`
            : `<div class="event-stream" data-agent-events>${
                events.length === 0
                  ? `<p class="t-small placeholder" data-events-placeholder>no run yet.</p>`
                  : events.map(renderAgentEvent).join("")
              }</div>`
        }
      </div>

      ${
        outcome
          ? `
        <div class="agent-outcome">
          <p class="t-eyebrow result-label">final answer</p>
          <pre class="result-pre">${esc(outcome.final_text.trim())}</pre>
          <p class="t-small subtle">run ${esc(outcome.run_id)} · ${outcome.turns} turns · in ${outcome.input_tokens} / out ${outcome.output_tokens} tokens</p>
          <p class="t-small subtle">run_dir: <span class="t-mono">${esc(outcome.run_dir)}</span></p>
        </div>`
          : ""
      }
    `;
  }

  function renderKeyEntry(provider: string, displayName: string): string {
    return `
      <div class="key-entry">
        <span class="t-small">${esc(displayName)} needs an API key:</span>
        <input
          id="agent-key-input"
          class="input-field"
          type="password"
          placeholder="paste api key"
          value="${esc(pendingKey)}"
          autocomplete="off"
          ${savingKey ? "disabled" : ""}
        />
        <button id="agent-key-save" type="button" data-provider="${esc(provider)}" class="btn-primary" ${savingKey ? "disabled" : ""}>
          ${savingKey ? "saving…" : "save"}
        </button>
        ${keyError ? `<span class="t-small result-error">${esc(keyError)}</span>` : ""}
      </div>
    `;
  }

  export function bind(shell: ShellState): void {
    const taskEl = document.getElementById("agent-task") as HTMLTextAreaElement | null;
    taskEl?.addEventListener("input", () => { task = taskEl.value; });
    const modelEl = document.getElementById("agent-model") as HTMLSelectElement | null;
    modelEl?.addEventListener("change", () => {
      model = modelEl.value;
      keyError = "";
      pendingKey = "";
      shell.rerender();
    });

    document.getElementById("agent-form")?.addEventListener("submit", async (e) => {
      e.preventDefault();
      const t = task.trim();
      if (!t || inFlight) return;
      inFlight = true;
      events = [];
      outcome = null;
      errorText = "";
      shell.rerender();
      try {
        outcome = await invoke<AgentOutcome>("agent_run", {
          task: t,
          model: model || null,
        });
      } catch (err) {
        errorText = `${err}`;
      } finally {
        inFlight = false;
        shell.rerender();
      }
    });

    const keyInput = document.getElementById("agent-key-input") as HTMLInputElement | null;
    keyInput?.addEventListener("input", () => { pendingKey = keyInput.value; });
    const saveBtn = document.getElementById("agent-key-save") as HTMLButtonElement | null;
    saveBtn?.addEventListener("click", async () => {
      const provider = saveBtn.dataset.provider;
      const key = pendingKey.trim();
      if (!provider || !key || savingKey) return;
      savingKey = true;
      keyError = "";
      shell.rerender();
      try {
        await invoke("agent_save_api_key", { provider, apiKey: key });
        const models = await invoke<ModelInfo[]>("agent_list_models");
        modelsCache = models;
        pendingKey = "";
      } catch (err) {
        keyError = `${err}`;
      } finally {
        savingKey = false;
        shell.rerender();
      }
    });
  }

  function renderAgentEvent(ev: AgentEventPayload): string {
    const glyph = eventGlyph(ev.kind);
    return `<div class="event event-${ev.kind}"><span class="event-glyph">${glyph}</span><span class="event-text">${esc(ev.text)}</span></div>`;
  }

  function eventGlyph(kind: AgentEventPayload["kind"]): string {
    switch (kind) {
      case "started": return "▸";
      case "turn": return "──";
      case "assistant": return " ";
      case "reasoning": return "·";
      case "tool_call": return "→";
      case "tool_result": return "←";
      case "tool_error": return "✗";
      case "api_error": return "✗";
      case "done": return "✓";
    }
  }
}
