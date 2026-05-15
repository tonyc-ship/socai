//! Tauri desktop entry — single scrolling page with three stacked
//! capability tests (cdp / tools / agent).

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import { agentPanel, cdpPanel, toolsPanel } from "./capability_tests";

export type Status =
  | { state: "disconnected"; reason: string }
  | { state: "connecting"; attempt: number }
  | { state: "connected"; endpoint: string; browser_version: string; page_count: number };

export interface TargetInfo {
  target_id: string;
  type: string;
  title: string;
  url: string;
}

export interface ModelInfo {
  provider: string;
  display_name: string;
  default_model: string;
  has_key: boolean;
}

export interface AgentEventPayload {
  kind:
    | "started"
    | "turn"
    | "assistant"
    | "reasoning"
    | "tool_call"
    | "tool_result"
    | "tool_error"
    | "api_error"
    | "done";
  text: string;
}

export interface ShellState {
  status: Status;
  pages: TargetInfo[];
  rerender: () => void;
}

const MARK_SVG = `
  <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32" width="24" height="24" fill="none" role="img" aria-label="socai">
    <rect x="2.5" y="2.5" width="27" height="27" rx="3" stroke="currentColor" stroke-width="1.6"></rect>
    <rect x="16" y="16" width="10" height="10" rx="1.2" fill="currentColor"></rect>
  </svg>
`;

interface PanelModule {
  label: string;
  render: (shell: ShellState) => string;
  bind: (shell: ShellState) => void;
}

const PANELS: PanelModule[] = [
  { label: "cdp", render: cdpPanel.render, bind: cdpPanel.bind },
  { label: "tools", render: toolsPanel.render, bind: toolsPanel.bind },
  { label: "agent", render: agentPanel.render, bind: agentPanel.bind },
];

let status: Status = { state: "disconnected", reason: "starting" };
let pages: TargetInfo[] = [];

function shell(): ShellState {
  return { status, pages, rerender: render };
}

function render(): void {
  const root = document.getElementById("app");
  if (!root) return;
  const state = shell();
  const sections = PANELS
    .map(
      (p) => `
      <section class="section">
        <p class="t-eyebrow section-label">${p.label}</p>
        ${p.render(state)}
      </section>`,
    )
    .join("");
  root.innerHTML = `
    <div class="shell">
      <header class="topbar">
        <div class="brand">${MARK_SVG}<span class="brand-name">socai</span></div>
        <span class="t-eyebrow connection-pill">${connectionEyebrow()}</span>
      </header>
      <main class="stack">${sections}</main>
    </div>
  `;
  for (const p of PANELS) p.bind(state);
}

function connectionEyebrow(): string {
  switch (status.state) {
    case "disconnected":
      return "chrome · disconnected";
    case "connecting":
      return `chrome · connecting · ${status.attempt}/3`;
    case "connected": {
      const n = pages.length;
      return `chrome · connected · ${n} tab${n === 1 ? "" : "s"}`;
    }
  }
}

async function main(): Promise<void> {
  try {
    status = await invoke<Status>("cdp_status");
    if (status.state === "connected") {
      try {
        pages = await invoke<TargetInfo[]>("cdp_list_pages");
      } catch (e) {
        console.error("initial cdp_list_pages failed:", e);
      }
    }
  } catch (e) {
    console.error("initial cdp_status failed:", e);
  }
  try {
    const models = await invoke<ModelInfo[]>("agent_list_models");
    agentPanel.setModels(models);
  } catch (e) {
    console.error("agent_list_models failed:", e);
  }
  render();

  await listen<Status>("cdp:status_changed", (event) => {
    const wasConnected = status.state === "connected";
    status = event.payload;
    if (status.state !== "connected") {
      pages = [];
      if (wasConnected) cdpPanel.reset();
    }
    render();
  });

  await listen<TargetInfo[]>("cdp:targets_changed", (event) => {
    pages = event.payload;
    render();
  });

  // Stream agent events incrementally — full re-render once per event would
  // also work but blows away the DOM unnecessarily and breaks the user's
  // scroll position. The panel appends a row and pins scroll-to-bottom.
  await listen<AgentEventPayload>("agent:event", (event) => {
    agentPanel.appendEvent(event.payload);
  });

  const refresh = (): void => {
    invoke("cdp_refresh").catch((e) => console.error("cdp_refresh failed:", e));
  };
  window.addEventListener("focus", refresh);
  document.addEventListener("visibilitychange", () => {
    if (document.visibilityState === "visible") refresh();
  });
}

main();
