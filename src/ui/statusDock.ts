import { queryRequired } from "./dom";

export type ActivityTone = "info" | "success" | "error" | "warning";

export interface StatusDock {
  element: HTMLElement;
  setPhase(label: string, detail: string): void;
  setProgress(percent: number | null, status: string): void;
  setError(message: string): void;
  addActivity(label: string, detail?: string, tone?: ActivityTone): void;
  appendLog(line: string): void;
  clearLog(): void;
  setBusy(busy: boolean): void;
}

const MAX_LOG_LINES = 600;
const MAX_ACTIVITY_ITEMS = 40;

const TONE_CLASSES: Record<ActivityTone, string> = {
  info: "border-sky-500/40 bg-sky-500/10 text-sky-200",
  success: "border-emerald-500/40 bg-emerald-500/10 text-emerald-200",
  error: "border-rose-500/40 bg-rose-500/10 text-rose-200",
  warning: "border-amber-500/40 bg-amber-500/10 text-amber-200",
};

function timestamp(): string {
  return new Date().toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}

/** Persistent bottom dock: one work line, progress, activity feed, live logs. */
export function createStatusDock(): StatusDock {
  const section = document.createElement("section");
  section.className = "status-dock";
  section.innerHTML = `
    <div class="dock-head">
      <div class="min-w-0">
        <div class="flex items-center gap-2 text-[11px] uppercase tracking-widest text-slate-500">
          <span data-role="phase" class="font-semibold text-sky-300">Idle</span>
          <span class="text-slate-700">/</span>
          <span>Live activity</span>
        </div>
        <p data-role="work-line" aria-live="polite" class="mt-1 truncate text-sm text-slate-200">
          Waiting for an operation.
        </p>
      </div>
      <div class="flex items-center gap-2">
        <span data-role="busy" class="hidden text-xs text-sky-300">Working…</span>
        <button type="button" data-role="clear-log"
                class="dock-button">Clear logs</button>
      </div>
    </div>

    <div data-role="progress" role="progressbar" aria-valuemin="0" aria-valuemax="100" aria-valuenow="0"
         class="dock-progress">
      <div data-role="bar" class="h-full w-0 rounded-full bg-sky-400 transition-[width] duration-300"></div>
    </div>

    <div data-role="activity" class="activity-feed" aria-label="Activity feed"></div>

    <details data-role="logs" class="dock-logs">
      <summary class="flex cursor-pointer items-center justify-between gap-3 px-4 py-2 text-xs">
        <span class="font-semibold uppercase tracking-widest text-slate-400">Live logs</span>
        <span data-role="log-count" class="text-slate-500">0 lines</span>
      </summary>
      <pre data-role="log-output" class="log-output">No output yet.</pre>
    </details>
  `;

  const phaseText = queryRequired<HTMLElement>(section, "[data-role='phase']");
  const workLine = queryRequired<HTMLElement>(section, "[data-role='work-line']");
  const busyText = queryRequired<HTMLElement>(section, "[data-role='busy']");
  const progress = queryRequired<HTMLElement>(section, "[data-role='progress']");
  const bar = queryRequired<HTMLElement>(section, "[data-role='bar']");
  const activity = queryRequired<HTMLElement>(section, "[data-role='activity']");
  const logDetails = queryRequired<HTMLDetailsElement>(section, "[data-role='logs']");
  const logOutput = queryRequired<HTMLElement>(section, "[data-role='log-output']");
  const logCount = queryRequired<HTMLElement>(section, "[data-role='log-count']");
  const clearButton = queryRequired<HTMLButtonElement>(section, "[data-role='clear-log']");

  let logLines: string[] = [];

  function renderLogs(): void {
    logOutput.textContent = logLines.length ? logLines.join("\n") : "No output yet.";
    logCount.textContent = `${logLines.length} lines`;
    if (logDetails.open) {
      logOutput.scrollTop = logOutput.scrollHeight;
    }
  }

  clearButton.addEventListener("click", () => {
    logLines = [];
    renderLogs();
  });

  function setBarTone(tone: "sky" | "emerald" | "rose"): void {
    bar.classList.remove("bg-sky-400", "bg-emerald-400", "bg-rose-500");
    bar.classList.add(
      tone === "sky" ? "bg-sky-400" : tone === "emerald" ? "bg-emerald-400" : "bg-rose-500",
    );
  }

  return {
    element: section,

    setPhase(label, detail) {
      phaseText.textContent = label;
      workLine.textContent = detail;
    },

    setProgress(percent, status) {
      if (percent === null) {
        progress.removeAttribute("aria-valuenow");
        return;
      }
      const clamped = Math.min(100, Math.max(0, Math.round(percent)));
      bar.style.width = `${clamped}%`;
      progress.setAttribute("aria-valuenow", String(clamped));
      if (status) {
        workLine.textContent = status;
      }
      if (clamped === 100) {
        setBarTone("emerald");
      }
    },

    setError(message) {
      setBarTone("rose");
      workLine.textContent = message;
    },

    addActivity(label, detail = "", tone = "info") {
      const item = document.createElement("div");
      item.className = `activity-item ${TONE_CLASSES[tone]}`;
      const time = document.createElement("span");
      time.className = "activity-time";
      time.textContent = timestamp();
      const text = document.createElement("span");
      text.className = "min-w-0 truncate";
      text.textContent = detail ? `${label} — ${detail}` : label;
      item.append(time, text);
      activity.prepend(item);
      while (activity.childElementCount > MAX_ACTIVITY_ITEMS) {
        activity.lastElementChild?.remove();
      }
    },

    appendLog(line) {
      const trimmed = line.trimEnd();
      if (!trimmed) {
        return;
      }
      logLines.push(trimmed);
      if (logLines.length > MAX_LOG_LINES) {
        logLines = logLines.slice(-MAX_LOG_LINES);
      }
      renderLogs();
    },

    clearLog() {
      logLines = [];
      renderLogs();
    },

    setBusy(busy) {
      busyText.classList.toggle("hidden", !busy);
      section.classList.toggle("is-busy", busy);
      if (busy) {
        setBarTone("sky");
      }
    },
  };
}
