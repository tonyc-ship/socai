import type { ModelInfo, Status, ShellState } from "../main";
import { esc } from "../lib/html";
import type { AgentTaskView, TaskMode, ToolCommand } from "./tasks";

export interface NewTaskPageProps {
  shell: ShellState;
  mode: TaskMode;
  toolCommand: ToolCommand;
  draft: string;
  submittingTask: boolean;
  toolInFlight: boolean;
  toolResult: unknown;
  toolError: string;
  submitError: string;
  tasks: AgentTaskView[];
  selectedModel: ModelInfo | undefined;
  overlayKeyDraft: string;
  overlayKeySaving: boolean;
  overlayKeyError: string;
}

export function renderNewTaskPage(props: NewTaskPageProps): string {
  const connected = props.shell.status.state === "connected";
  const modelReady = !!props.selectedModel && props.selectedModel.has_key;
  const agentMode = props.mode === "agent";
  const running = agentMode ? props.submittingTask : props.toolInFlight;
  const runDisabled = running || !props.draft.trim() || !connected || (agentMode && !modelReady);
  const needsKey = connected && agentMode && !!props.selectedModel && !props.selectedModel.has_key;
  const gated = !connected || needsKey;

  return `
    <div class="new-task-page">
      <div class="new-task-compose">
        <div class="new-task-copy">
          <h2 class="t-h2">what should socai research?</h2>
          <p class="t-small subtle">start a one-shot browser task. socai opens a temporary chrome tab, runs the agent, saves the result, then closes the tab.</p>
        </div>
        <div class="compose-form-stack ${gated ? "is-masked" : ""}">
          <div class="compose-form-inner" aria-hidden="${gated ? "true" : "false"}">
            ${renderTaskForm(props, agentMode, running, runDisabled)}
            ${renderInlineGuard(props.mode, props.toolCommand, props.selectedModel)}
            ${props.submitError ? `<pre class="result-pre result-error">${esc(props.submitError)}</pre>` : ""}
            ${agentMode ? "" : renderToolResult(props)}
          </div>
          ${!connected ? renderConnectOverlay(props.shell.status) : ""}
          ${connected && needsKey ? renderApiKeyOverlay(props.selectedModel!, props) : ""}
        </div>
      </div>
      ${renderRunningChip(props.tasks)}
      ${renderTaskGlance(props.tasks)}
    </div>
  `;
}

function renderTaskForm(
  props: NewTaskPageProps,
  agentMode: boolean,
  running: boolean,
  runDisabled: boolean,
): string {
  return `
    <form id="task-form" class="task-form task-form-centered">
      <textarea
        id="task-input"
        class="task-input"
        rows="5"
        placeholder="${esc(taskPlaceholder(props.mode, props.toolCommand))}"
        ${running ? "disabled" : ""}
      >${esc(props.draft)}</textarea>

      <div class="task-controls">
        <div class="mode-switch" aria-label="task mode">
          <button id="mode-agent" type="button" class="mode-button ${agentMode ? "mode-button-active" : ""}">agent tasks</button>
          <button id="mode-tools" type="button" class="mode-button ${!agentMode ? "mode-button-active" : ""}">tool tests</button>
        </div>
        ${agentMode ? renderAgentSummary(props.selectedModel) : renderToolPicker(props.toolCommand)}
        <button id="task-submit" type="submit" class="btn-primary" ${runDisabled ? "disabled" : ""}>
          ${running ? "starting…" : agentMode ? "new task" : "run test"}
        </button>
      </div>
    </form>
  `;
}

function renderInlineGuard(mode: TaskMode, toolCommand: ToolCommand, selected: ModelInfo | undefined): string {
  if (mode !== "agent") return renderToolHint(toolCommand);
  if (!selected) return `<p class="t-small subtle">loading agent models…</p>`;
  return "";
}

function renderAgentSummary(selected: ModelInfo | undefined): string {
  const summary = selected
    ? `agent · ${esc(selected.display_name)} · <span class="t-mono">${esc(selected.default_model)}</span>`
    : "agent · loading";
  return `<p class="t-small subtle task-context">${summary}</p>`;
}

function renderToolPicker(toolCommand: ToolCommand): string {
  const tools: Array<[ToolCommand, string]> = [
    ["search_notes", "search notes"],
    ["topic_scan", "topic scan"],
    ["extract_note", "extract note"],
  ];
  return `
    <div class="tool-picker" aria-label="tool">
      ${tools.map(([cmd, label]) => `
        <button type="button" data-tool="${cmd}" class="tool-choice ${toolCommand === cmd ? "tool-choice-active" : ""}">
          ${label}
        </button>
      `).join("")}
    </div>
  `;
}

function renderToolHint(toolCommand: ToolCommand): string {
  const hint = {
    search_notes: "test search_notes on a fresh temporary xiaohongshu tab.",
    topic_scan: "test topic_scan on a fresh temporary xiaohongshu tab.",
    extract_note: "paste a note id or url; socai opens a fresh temporary page and extracts it.",
  }[toolCommand];
  return `<p class="t-small subtle">${hint}</p>`;
}

function taskPlaceholder(mode: TaskMode, toolCommand: ToolCommand): string {
  if (mode === "agent") return "tell socai what you want researched…\neach task opens its own temporary chrome tab.";
  switch (toolCommand) {
    case "search_notes": return "search query…";
    case "topic_scan": return "topic to scan…";
    case "extract_note": return "note id or url…";
  }
}

function renderConnectOverlay(status: Status): string {
  const connecting = status.state === "connecting";
  const label = connecting
    ? `chrome · connecting · ${(status as Extract<Status, { state: "connecting" }>).attempt}/3`
    : "chrome · disconnected";
  const heading = connecting ? "looking for chrome…" : "connect chrome to start";
  const cta = connecting ? "connecting…" : "connect chrome →";
  const dotClass = connecting ? "badge-dot-ink badge-dot-pulse" : "badge-dot-hollow";
  return `
    <div class="connect-overlay" role="dialog" aria-label="chrome required">
      <span class="connect-overlay-pill">
        <i class="badge-dot ${dotClass}" aria-hidden="true"></i>${label}
      </span>
      <h3 class="connect-overlay-head">${heading}</h3>
      <button
        id="overlay-chrome-connect"
        type="button"
        class="btn-primary connect-overlay-cta"
        ${connecting ? "disabled" : ""}
      >${cta}</button>
      <a
        class="connect-overlay-link t-small"
        href="https://developer.chrome.com/docs/devtools/remote-debugging"
        target="_blank"
        rel="noopener noreferrer"
      >how do i enable remote debugging? ↗</a>
    </div>
  `;
}

function renderApiKeyOverlay(selected: ModelInfo, props: NewTaskPageProps): string {
  const placeholder = {
    anthropic: "sk-ant-...",
    openai: "sk-...",
  }[selected.provider] ?? "paste api key";
  const saving = props.overlayKeySaving;
  const cta = saving ? "saving…" : "save & continue →";
  const disableSubmit = saving || !props.overlayKeyDraft.trim();
  return `
    <form
      id="overlay-key-form"
      class="connect-overlay"
      role="dialog"
      aria-label="api key required"
      data-provider="${esc(selected.provider)}"
    >
      <span class="connect-overlay-pill">
        <i class="badge-dot badge-dot-hollow" aria-hidden="true"></i>
        agent · ${esc(selected.display_name)} · key needed
      </span>
      <h3 class="connect-overlay-head">
        add your ${esc(selected.display_name.toLowerCase())} api key
      </h3>
      <input
        id="overlay-key-input"
        type="password"
        class="input-field connect-overlay-input"
        placeholder="${esc(placeholder)}"
        autocomplete="off"
        value="${esc(props.overlayKeyDraft)}"
        ${saving ? "disabled" : ""}
      />
      <button
        id="overlay-key-save"
        type="submit"
        class="btn-primary connect-overlay-cta"
        ${disableSubmit ? "disabled" : ""}
      >${cta}</button>
      ${props.overlayKeyError ? `<p class="t-small result-error connect-overlay-error">${esc(props.overlayKeyError)}</p>` : ""}
    </form>
  `;
}

function renderRunningChip(tasks: AgentTaskView[]): string {
  const running = [...tasks]
    .filter((t) => t.status === "running" || t.status === "queued")
    .sort((a, b) => b.created_at - a.created_at);
  if (running.length === 0) return "";
  const first = running[0];
  const isOne = running.length === 1;
  const count = isOne ? "1 task running" : `${running.length} tasks running`;
  const taskLabel = isOne
    ? `<span class="running-chip-dot" aria-hidden="true">·</span><span class="running-chip-task">${esc(first.task)}</span>`
    : "";
  return `
    <button type="button" class="running-chip" data-task-id="${esc(first.task_id)}">
      <i class="badge-dot badge-dot-ink badge-dot-pulse" aria-hidden="true"></i>
      <span class="running-chip-count">${count}</span>
      ${taskLabel}
      <span class="running-chip-arrow" aria-hidden="true">→</span>
    </button>
  `;
}

function renderTaskGlance(tasks: AgentTaskView[]): string {
  const recent = [...tasks]
    .filter((task) => task.status !== "running" && task.status !== "queued")
    .sort((a, b) => b.created_at - a.created_at)
    .slice(0, 5);

  return `
    <div class="task-glance">
      <section class="task-glance-card">
        <div class="task-glance-head">
          <p class="t-eyebrow result-label">recent</p>
          <button id="recent-history-link" type="button" class="btn-ghost btn-compact">view history</button>
        </div>
        ${renderTaskSummaryRows(recent, "no recent tasks yet.")}
      </section>
    </div>
  `;
}

function renderTaskSummaryRows(items: AgentTaskView[], emptyText: string): string {
  if (items.length === 0) {
    return `<p class="t-small placeholder task-summary-empty">${emptyText}</p>`;
  }
  return `
    <div class="task-summary-list">
      ${items.map((task) => `
        <button type="button" class="task-summary-row" data-task-id="${esc(task.task_id)}">
          <span class="task-row-glyph task-row-glyph-${esc(task.status)}" aria-hidden="true">${taskStatusGlyph(task.status)}</span>
          <span class="task-row-main">
            <span class="task-row-title">${esc(task.task)}</span>
            <span class="task-row-meta">${esc(task.status)} · ${esc(formatTime(task.created_at))}</span>
          </span>
        </button>
      `).join("")}
    </div>
  `;
}

function renderToolResult(props: NewTaskPageProps): string {
  if (props.toolInFlight) {
    return `<pre class="result-pre result-running">running: ${esc(props.toolCommand)}(${esc(JSON.stringify(props.draft.trim()))})</pre>`;
  }
  if (props.toolError) {
    return `<pre class="result-pre result-error">${esc(props.toolError)}</pre>`;
  }
  if (props.toolResult) {
    return `<pre class="result-pre">${esc(JSON.stringify(props.toolResult, null, 2))}</pre>`;
  }
  return `<p class="t-small placeholder">no tool test result yet.</p>`;
}

function taskStatusGlyph(status: AgentTaskView["status"]): string {
  switch (status) {
    case "queued": return "○";
    case "running": return "●";
    case "completed": return "✓";
    case "failed": return "×";
    case "cancelled": return "−";
    case "interrupted": return "!";
  }
}

function formatTime(ms: number): string {
  if (!ms) return "";
  return new Date(ms).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}
