import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

type Status =
  | { state: "disconnected"; reason: string }
  | { state: "connecting"; attempt: number }
  | { state: "connected"; endpoint: string; browser_version: string; page_count: number };

interface TargetInfo {
  target_id: string;
  type: string;
  title: string;
  url: string;
}

const MARK_SVG = `
  <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32" width="48" height="48" fill="none" role="img" aria-label="socai">
    <rect x="2.5" y="2.5" width="27" height="27" rx="3" stroke="currentColor" stroke-width="1.6"></rect>
    <rect x="16" y="16" width="10" height="10" rx="1.2" fill="currentColor"></rect>
  </svg>
`;

let status: Status = { state: "disconnected", reason: "starting" };
let pages: TargetInfo[] = [];
let searchQuery = "";
let actionStatus = "";
let actionInFlight = false;

function renderHero(): void {
  const root = document.getElementById("app");
  if (!root) return;
  root.innerHTML = `<main class="hero">${heroContent()}</main>`;
  bindActions();
  refocusInputIfRendered();
}

function heroContent(): string {
  switch (status.state) {
    case "disconnected":
      return `
        <div class="hero-mark">${MARK_SVG}</div>
        <p class="t-eyebrow">chrome · disconnected</p>
        <h1 class="t-display">connect chrome</h1>
        <p class="t-lede">${disconnectExplainer(status.reason)}</p>
        <button id="connect" class="btn-primary">connect</button>
      `;
    case "connecting":
      return `
        <div class="hero-mark">${MARK_SVG}</div>
        <p class="t-eyebrow">chrome · connecting · attempt ${status.attempt}/3</p>
        <h1 class="t-display">looking for chrome…</h1>
      `;
    case "connected":
      return `
        <div class="hero-mark">${MARK_SVG}</div>
        <p class="t-eyebrow" id="connected-eyebrow">${connectedEyebrowText()}</p>
        <h1 class="t-display">${esc(status.browser_version)}</h1>
        ${actionPanel()}
        <button id="disconnect" class="btn-ghost">disconnect</button>
        <ul class="tab-list">
          ${pages.map(renderTab).join("")}
        </ul>
      `;
  }
}

function connectedEyebrowText(): string {
  const n = pages.length;
  return `chrome · connected · ${n} tab${n === 1 ? "" : "s"}`;
}

function actionPanel(): string {
  return `
    <section class="action-panel">
      <p class="t-eyebrow">try an action</p>
      <form id="search-form" class="action-form">
        <input
          id="search-input"
          class="input-field"
          type="text"
          placeholder="search google for…"
          value="${esc(searchQuery)}"
          autocomplete="off"
          ${actionInFlight ? "disabled" : ""}
        />
        <button type="submit" class="btn-primary" ${actionInFlight ? "disabled" : ""}>
          ${actionInFlight ? "running…" : "search"}
        </button>
      </form>
      <p class="t-small action-status" id="action-status">${esc(actionStatus)}</p>
    </section>
  `;
}

function renderTab(t: TargetInfo): string {
  return `
    <li class="tab-row">
      <div class="tab-title t-body">${esc(t.title || "(untitled)")}</div>
      <div class="tab-url t-mono">${esc(truncate(t.url, 90))}</div>
    </li>
  `;
}

function disconnectExplainer(reason: string): string {
  if (reason === "not_yet_connected" || reason === "starting") {
    return "click connect to attach socai to your running chrome.";
  }
  if (reason === "user_disconnected") {
    return "disconnected. click to reconnect when you're ready.";
  }
  if (reason === "connection_lost") {
    return "chrome went away. click to reconnect when you're ready.";
  }
  return reason;
}

function bindActions(): void {
  document.getElementById("connect")?.addEventListener("click", () => {
    invoke("cdp_connect").catch((e) => console.error("cdp_connect failed:", e));
  });
  document.getElementById("disconnect")?.addEventListener("click", () => {
    invoke("cdp_disconnect").catch((e) => console.error("cdp_disconnect failed:", e));
  });

  const input = document.getElementById("search-input") as HTMLInputElement | null;
  if (input) {
    input.addEventListener("input", () => {
      searchQuery = input.value;
    });
  }

  document.getElementById("search-form")?.addEventListener("submit", async (e) => {
    e.preventDefault();
    const query = searchQuery.trim();
    if (!query || actionInFlight) return;

    actionInFlight = true;
    actionStatus = `opening tab and searching for "${query}"…`;
    renderHero();

    try {
      const result = await invoke<string>("cdp_test_search", { query });
      actionStatus = result;
    } catch (err) {
      actionStatus = `error: ${err}`;
    } finally {
      actionInFlight = false;
      renderHero();
    }
  });
}

function refocusInputIfRendered(): void {
  if (status.state !== "connected" || actionInFlight) return;
  const input = document.getElementById("search-input") as HTMLInputElement | null;
  if (input && document.activeElement !== input && searchQuery !== "") {
    // Keep focus only if the user was mid-typing; don't steal it on first paint.
    return;
  }
}

function updateTabsInPlace(): void {
  if (status.state !== "connected") return;
  const eyebrow = document.getElementById("connected-eyebrow");
  if (eyebrow) eyebrow.textContent = connectedEyebrowText();
  const list = document.querySelector(".tab-list");
  if (list) list.innerHTML = pages.map(renderTab).join("");
}

function esc(s: string): string {
  return s.replace(/[<>&"']/g, (c) => {
    return (
      { "<": "&lt;", ">": "&gt;", "&": "&amp;", '"': "&quot;", "'": "&#39;" } as Record<string, string>
    )[c];
  });
}

function truncate(s: string, n: number): string {
  return s.length > n ? s.slice(0, n - 1) + "…" : s;
}

async function main(): Promise<void> {
  try {
    status = await invoke<Status>("cdp_status");
    if (status.state === "connected") {
      // Initial cdp:targets_changed was emitted at connect time, which may
      // have happened before this webview started listening (e.g. tauri dev
      // HMR). Pull a fresh snapshot so the tab list isn't blank.
      try {
        pages = await invoke<TargetInfo[]>("cdp_list_pages");
      } catch (e) {
        console.error("initial cdp_list_pages failed:", e);
      }
    }
  } catch (e) {
    console.error("initial cdp_status failed:", e);
  }
  renderHero();

  await listen<Status>("cdp:status_changed", (event) => {
    const wasConnected = status.state === "connected";
    status = event.payload;
    if (status.state !== "connected") {
      pages = [];
      if (wasConnected) {
        actionStatus = "";
      }
    }
    renderHero();
  });

  await listen<TargetInfo[]>("cdp:targets_changed", (event) => {
    pages = event.payload;
    updateTabsInPlace();
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
