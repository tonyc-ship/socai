//! Task workspace coordinator: shared state, agent configuration popover,
//! task event intake, and bindings. The tab bodies live in `task_new.ts` and
//! `task_history.ts`.

import { invoke } from "@tauri-apps/api/core";
import type {
  AgentTaskEventPayload,
  AgentTaskSnapshot,
  AgentTaskStatus,
  ModelInfo,
  ShellState,
} from "../main";
import { esc } from "../lib/html";
import { renderAgentEvent, renderHistoryPage } from "./task_history";
import { renderNewTaskPage } from "./task_new";

export type TaskMode = "agent" | "tools";
type TaskPage = "new" | "history";
export type ToolCommand = "search_notes" | "topic_scan" | "extract_note";
export type AgentTaskView = AgentTaskSnapshot & { events: AgentTaskEventPayload[] };

// ── Agent task workspace ──────────────────────────────────────────────────

export namespace agentPanel {
  let page: TaskPage = "new";
  let draft = "";
  let mode: TaskMode = "agent";
  let toolCommand: ToolCommand = "search_notes";
  let model = "";
  let submittingTask = false;
  let toolInFlight = false;
  let toolResult: unknown = null;
  let toolError = "";
  let submitError = "";
  let tasks: AgentTaskView[] = [];
  let selectedTaskId: string | null = null;
  let modelsCache: ModelInfo[] = [];

  // Key-entry sub-state — used by the header configuration popover.
  let pendingKey = "";
  let savingKey = false;
  let keyError = "";
  let configOpen = false;

  // Overlay key-entry sub-state — used by the inline new-task gate when
  // chrome is connected but the selected agent has no key.
  let overlayKey = "";
  let overlaySaving = false;
  let overlayError = "";

  export function setModels(models: ModelInfo[]): void {
    modelsCache = models;
    if (!model || !models.some((m) => m.default_model === model)) {
      const withKey = models.find((m) => m.has_key);
      model = (withKey ?? models[0])?.default_model ?? "";
    }
  }

  export function setTasks(snapshots: AgentTaskSnapshot[]): void {
    const eventsById = new Map(tasks.map((task) => [task.task_id, task.events]));
    tasks = snapshots.map((snapshot) => ({ ...snapshot, events: eventsById.get(snapshot.task_id) ?? [] }));
    if (!selectedTaskId && tasks.length > 0) {
      selectedTaskId = newestTask(tasks)?.task_id ?? null;
    }
  }

  export function renderHeader(): string {
    return `
      <div class="agent-status">
        ${renderAgentBadge()}
        ${configOpen ? renderConfigPopover() : ""}
      </div>
    `;
  }

  export function bindHeader(shell: ShellState): void {
    document.getElementById("agent-config-toggle")?.addEventListener("click", () => {
      configOpen = !configOpen;
      shell.rerender();
    });

    const modelEl = document.getElementById("agent-header-model") as HTMLSelectElement | null;
    modelEl?.addEventListener("change", () => {
      model = modelEl.value;
      keyError = "";
      pendingKey = "";
      shell.rerender();
    });

    const keyInput = document.getElementById("agent-header-key-input") as HTMLInputElement | null;
    keyInput?.addEventListener("input", () => { pendingKey = keyInput.value; });

    const saveBtn = document.getElementById("agent-header-key-save") as HTMLButtonElement | null;
    saveBtn?.addEventListener("click", async () => {
      const provider = saveBtn.dataset.provider;
      const key = pendingKey.trim();
      if (!provider || !key || savingKey) return;
      savingKey = true;
      keyError = "";
      shell.rerender();
      try {
        await invoke("agent_save_api_key", { provider, apiKey: key });
        setModels(await invoke<ModelInfo[]>("agent_list_models"));
        pendingKey = "";
      } catch (err) {
        keyError = `${err}`;
      } finally {
        savingKey = false;
        shell.rerender();
      }
    });
  }

  export function closeHeaderConfig(): boolean {
    if (!configOpen) return false;
    configOpen = false;
    return true;
  }

  function renderAgentBadge(): string {
    const selected = selectedModel();
    const expanded = configOpen ? "true" : "false";
    if (!selected) {
      return `<button id="agent-config-toggle" type="button" class="badge badge-button" aria-expanded="${expanded}"><i class="badge-dot badge-dot-muted" aria-hidden="true"></i>agent · loading</button>`;
    }
    if (!selected.has_key) {
      return `<button id="agent-config-toggle" type="button" class="badge badge-button" aria-expanded="${expanded}"><i class="badge-dot badge-dot-hollow" aria-hidden="true"></i>agent · ${esc(selected.display_name)} · key needed</button>`;
    }
    return `<button id="agent-config-toggle" type="button" class="badge badge-button" aria-expanded="${expanded}"><i class="badge-dot badge-dot-ink" aria-hidden="true"></i>agent · ${esc(selected.display_name)}</button>`;
  }

  function renderConfigPopover(): string {
    const selected = selectedModel();
    const modelOpts = modelsCache
      .map((m) => {
        const sel = model === m.default_model ? "selected" : "";
        const flag = m.has_key ? "" : " · no key";
        return `<option value="${esc(m.default_model)}" ${sel}>${esc(m.display_name)} — ${esc(m.default_model)}${flag}</option>`;
      })
      .join("");

    return `
      <div class="topbar-popover agent-config-popover" role="dialog" aria-label="agent configuration">
        <p class="t-eyebrow agent-config-title">agent</p>
        <label class="agent-config-field">
          <span class="t-small">model</span>
          <select id="agent-header-model" class="input-field" ${savingKey || submittingTask ? "disabled" : ""}>
            ${modelOpts || `<option value="">loading…</option>`}
          </select>
        </label>
        ${
          selected
            ? `<p class="t-small subtle">${esc(selected.display_name)} · <span class="t-mono">${esc(selected.default_model)}</span></p>`
            : `<p class="t-small subtle">loading available models…</p>`
        }
        ${selected ? selected.has_key ? `<p class="t-small subtle">api key configured.</p>` : renderHeaderKeyEntry(selected) : ""}
      </div>
    `;
  }

  function renderHeaderKeyEntry(selected: ModelInfo): string {
    return `
      <div class="agent-config-key">
        <p class="t-small subtle">${esc(selected.display_name)} needs an api key.</p>
        <input
          id="agent-header-key-input"
          class="input-field"
          type="password"
          placeholder="paste api key"
          value="${esc(pendingKey)}"
          autocomplete="off"
          ${savingKey ? "disabled" : ""}
        />
        <div class="agent-config-actions">
          <button id="agent-header-key-save" type="button" data-provider="${esc(selected.provider)}" class="btn-primary btn-compact" ${savingKey ? "disabled" : ""}>
            ${savingKey ? "saving…" : "save"}
          </button>
          ${keyError ? `<span class="t-small result-error">${esc(keyError)}</span>` : ""}
        </div>
      </div>
    `;
  }

  function selectedModel(): ModelInfo | undefined {
    return modelsCache.find((m) => m.default_model === model);
  }

  // Append a streamed event and update state. Returns true when the shell
  // should re-render because a task snapshot/status changed.
  export function appendTaskEvent(payload: AgentTaskEventPayload): boolean {
    if (payload.snapshot) upsertTask(payload.snapshot);

    let task = tasks.find((item) => item.task_id === payload.task_id);
    if (!task && payload.snapshot) {
      task = upsertTask(payload.snapshot);
    }
    if (!task) return !!payload.snapshot;

    if (payload.text.trim()) {
      task.events = [...task.events, payload];
      appendEventRowIfSelected(payload);
    }

    return !!payload.snapshot;
  }

  export function render(shell: ShellState): string {
    return `
      <div class="task-interface">
        ${renderPageTabs()}
        ${page === "new"
          ? renderNewTaskPage({
              shell,
              mode,
              toolCommand,
              draft,
              submittingTask,
              toolInFlight,
              toolResult,
              toolError,
              submitError,
              tasks,
              selectedModel: selectedModel(),
              overlayKeyDraft: overlayKey,
              overlayKeySaving: overlaySaving,
              overlayKeyError: overlayError,
            })
          : renderHistoryPage({ tasks, selectedTask: selectedTask(), selectedTaskId })}
      </div>
    `;
  }

  function renderPageTabs(): string {
    return `
      <div class="page-switch" aria-label="task pages">
        <button id="page-new" type="button" class="page-switch-button ${page === "new" ? "page-switch-button-active" : ""}">new task</button>
        <button id="page-history" type="button" class="page-switch-button ${page === "history" ? "page-switch-button-active" : ""}">history</button>
      </div>
    `;
  }

  export function bind(shell: ShellState): void {
    document.getElementById("page-new")?.addEventListener("click", () => {
      page = "new";
      shell.rerender();
    });
    document.getElementById("page-history")?.addEventListener("click", () => {
      page = "history";
      shell.rerender();
    });
    document.getElementById("history-new-task")?.addEventListener("click", () => {
      page = "new";
      shell.rerender();
    });
    document.getElementById("recent-history-link")?.addEventListener("click", () => {
      page = "history";
      shell.rerender();
    });

    const taskEl = document.getElementById("task-input") as HTMLTextAreaElement | null;
    taskEl?.addEventListener("input", () => {
      draft = taskEl.value;
      updateSubmitButton(shell);
    });

    document.getElementById("mode-agent")?.addEventListener("click", () => {
      mode = "agent";
      submitError = "";
      shell.rerender();
    });
    document.getElementById("mode-tools")?.addEventListener("click", () => {
      mode = "tools";
      submitError = "";
      shell.rerender();
    });
    document.querySelectorAll<HTMLButtonElement>("[data-tool]").forEach((btn) => {
      btn.addEventListener("click", () => {
        toolCommand = btn.dataset.tool as ToolCommand;
        toolError = "";
        toolResult = null;
        shell.rerender();
      });
    });
    document.querySelectorAll<HTMLButtonElement>("[data-task-id]").forEach((btn) => {
      btn.addEventListener("click", () => {
        selectedTaskId = btn.dataset.taskId ?? null;
        page = "history";
        shell.rerender();
      });
    });
    document.querySelectorAll<HTMLButtonElement>("[data-cancel-task]").forEach((btn) => {
      btn.addEventListener("click", async () => {
        const taskId = btn.dataset.cancelTask;
        if (!taskId) return;
        btn.disabled = true;
        try {
          const snapshot = await invoke<AgentTaskSnapshot>("agent_task_cancel", { taskId });
          upsertTask(snapshot);
        } catch (err) {
          submitError = `${err}`;
        } finally {
          shell.rerender();
        }
      });
    });

    document.getElementById("task-form")?.addEventListener("submit", async (e) => {
      e.preventDefault();
      if (mode === "agent") await startAgentTask(shell);
      else await runDedicatedTool(shell);
    });

    document.getElementById("overlay-chrome-connect")?.addEventListener("click", () => {
      invoke("cdp_connect").catch((e) => console.error("cdp_connect failed:", e));
    });

    const overlayKeyInput = document.getElementById("overlay-key-input") as HTMLInputElement | null;
    overlayKeyInput?.addEventListener("input", () => {
      overlayKey = overlayKeyInput.value;
      const submit = document.getElementById("overlay-key-save") as HTMLButtonElement | null;
      if (submit) submit.disabled = overlaySaving || !overlayKey.trim();
    });
    if (overlayKeyInput && document.activeElement === document.body) {
      overlayKeyInput.focus();
    }

    document.getElementById("overlay-key-form")?.addEventListener("submit", async (e) => {
      e.preventDefault();
      await saveOverlayKey(shell);
    });
  }

  async function saveOverlayKey(shell: ShellState): Promise<void> {
    const form = document.getElementById("overlay-key-form") as HTMLFormElement | null;
    const provider = form?.dataset.provider;
    const key = overlayKey.trim();
    if (!provider || !key || overlaySaving) return;
    overlaySaving = true;
    overlayError = "";
    shell.rerender();
    try {
      await invoke("agent_save_api_key", { provider, apiKey: key });
      setModels(await invoke<ModelInfo[]>("agent_list_models"));
      overlayKey = "";
    } catch (err) {
      overlayError = `${err}`;
    } finally {
      overlaySaving = false;
      shell.rerender();
    }
  }

  function updateSubmitButton(shell: ShellState): void {
    const button = document.getElementById("task-submit") as HTMLButtonElement | null;
    if (!button) return;
    const connected = shell.status.state === "connected";
    const agentMode = mode === "agent";
    const selected = selectedModel();
    const modelReady = !!selected && selected.has_key;
    const running = agentMode ? submittingTask : toolInFlight;
    button.disabled = running || !draft.trim() || !connected || (agentMode && !modelReady);
  }

  async function startAgentTask(shell: ShellState): Promise<void> {
    const value = draft.trim();
    if (!value || submittingTask) return;
    submittingTask = true;
    submitError = "";
    shell.rerender();
    try {
      const snapshot = await invoke<AgentTaskSnapshot>("agent_task_start", {
        task: value,
        model: model || null,
      });
      upsertTask(snapshot);
      selectedTaskId = snapshot.task_id;
      page = "history";
      draft = "";
    } catch (err) {
      submitError = `${err}`;
    } finally {
      submittingTask = false;
      shell.rerender();
    }
  }

  async function runDedicatedTool(shell: ShellState): Promise<void> {
    const value = draft.trim();
    if (!value || toolInFlight) return;
    toolInFlight = true;
    toolError = "";
    toolResult = null;
    shell.rerender();
    try {
      if (toolCommand === "extract_note") {
        toolResult = await invoke("tool_extract_note", { noteId: value });
      } else if (toolCommand === "topic_scan") {
        toolResult = await invoke("tool_topic_scan", { query: value });
      } else {
        toolResult = await invoke("tool_search_notes", { query: value });
      }
    } catch (err) {
      toolError = `${err}`;
    } finally {
      toolInFlight = false;
      shell.rerender();
    }
  }

  function upsertTask(snapshot: AgentTaskSnapshot): AgentTaskView {
    const existing = tasks.find((task) => task.task_id === snapshot.task_id);
    if (existing) {
      const merged = mergeSnapshot(existing, snapshot);
      Object.assign(existing, merged, { events: existing.events });
      return existing;
    }
    const created = { ...snapshot, events: [] };
    tasks = [...tasks, created];
    if (!selectedTaskId) selectedTaskId = snapshot.task_id;
    return created;
  }

  function mergeSnapshot(existing: AgentTaskView, incoming: AgentTaskSnapshot): AgentTaskSnapshot {
    const status = statusRank(existing.status) > statusRank(incoming.status) ? existing.status : incoming.status;
    const terminalIncoming = statusRank(incoming.status) >= 2;
    return {
      ...incoming,
      status,
      started_at: incoming.started_at ?? existing.started_at,
      finished_at: incoming.finished_at ?? existing.finished_at,
      run_id: incoming.run_id ?? existing.run_id,
      run_dir: incoming.run_dir ?? existing.run_dir,
      target_id: terminalIncoming ? incoming.target_id : incoming.target_id ?? existing.target_id,
      final_text: incoming.final_text ?? existing.final_text,
      error: incoming.error ?? existing.error,
      turns: incoming.turns ?? existing.turns,
      input_tokens: incoming.input_tokens ?? existing.input_tokens,
      output_tokens: incoming.output_tokens ?? existing.output_tokens,
    };
  }

  function statusRank(status: AgentTaskStatus): number {
    switch (status) {
      case "queued": return 0;
      case "running": return 1;
      case "completed": return 2;
      case "failed": return 2;
      case "cancelled": return 2;
      case "interrupted": return 2;
    }
  }

  function selectedTask(): AgentTaskView | undefined {
    if (selectedTaskId) {
      const selected = tasks.find((task) => task.task_id === selectedTaskId);
      if (selected) return selected;
    }
    return newestTask(tasks);
  }

  function newestTask(items: AgentTaskView[]): AgentTaskView | undefined {
    return [...items].sort((a, b) => b.created_at - a.created_at)[0];
  }

  function appendEventRowIfSelected(payload: AgentTaskEventPayload): void {
    if (payload.task_id !== selectedTaskId) return;
    const stream = document.querySelector<HTMLDivElement>(`[data-agent-events="${payload.task_id}"]`);
    if (!stream) return;

    const placeholder = stream.querySelector("[data-events-placeholder]");
    if (placeholder) placeholder.remove();

    stream.insertAdjacentHTML("beforeend", renderAgentEvent(payload));
    stream.scrollTop = stream.scrollHeight;
  }
}
