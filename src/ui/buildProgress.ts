import { errorMessage } from "../api";
import { queryRequired } from "./dom";
import { formatBytes } from "./format";
import { formatOption, platformOption } from "./targetSelector";
import type { BuildSelection } from "../types";

export interface BuildProgressView {
  element: HTMLElement;
  setRunning(running: boolean): void;
  setProgress(percent: number, status: string): void;
  appendLog(line: string): void;
  setCompleted(outputPath: string): void;
  setFailed(message: string): void;
}

export interface BuildProgressOptions {
  selection: BuildSelection;
  onReset: () => void;
  onOpenFolder: (path: string) => Promise<void> | void;
}

/**
 * Step 3 UI. The imperative API is driven by Tauri `build://event` messages so
 * live logs and progress never rebuild the DOM while a build is running.
 */
export function createBuildProgress({
  selection,
  onReset,
  onOpenFolder,
}: BuildProgressOptions): BuildProgressView {
  const option = platformOption(selection.platform);
  const formatLabel = formatOption(selection.platform, selection.format)?.label ?? selection.format;
  const runtimeNote =
    selection.platform === "macos"
      ? "macOS builds run natively with Xcode on this host."
      : "Compilation runs in an ephemeral Docker container.";
  const section = document.createElement("section");

  section.innerHTML = `
    <div class="mb-5">
      <h2 class="text-2xl font-semibold tracking-tight">Building your target</h2>
      <p class="mt-1 text-sm text-slate-400">${runtimeNote}</p>
    </div>

    <div class="rounded-2xl border border-slate-800 bg-slate-900/50 p-5">
      <div class="flex flex-wrap items-center justify-between gap-2 text-sm">
        <span class="font-medium">${option.label} · ${selection.architecture} · ${formatLabel}</span>
        <span class="text-xs text-slate-400">${
          selection.asset ? formatBytes(selection.asset.sizeBytes) : "no asset selected"
        }</span>
      </div>

      <div data-role="progress" role="progressbar" aria-valuemin="0" aria-valuemax="100" aria-valuenow="0"
           class="mt-5 h-2 w-full overflow-hidden rounded-full bg-slate-800">
        <div data-role="bar" class="h-full w-0 rounded-full bg-sky-400 transition-[width] duration-300"></div>
      </div>
      <p data-role="status" aria-live="polite" class="mt-3 text-sm text-slate-300">
        Waiting for the build backend...
      </p>
    </div>

    <details data-role="logs" class="log-drawer mt-5 rounded-2xl border border-slate-800 bg-slate-950/70">
      <summary class="flex cursor-pointer items-center justify-between px-5 py-4 text-sm font-medium">
        <span>Terminal logs</span>
        <span data-role="log-count" class="text-xs text-slate-500">0 lines</span>
      </summary>
      <pre data-role="log-output" class="max-h-64 overflow-auto border-t border-slate-800 px-5 py-4 text-xs leading-relaxed text-slate-300">No build output yet.</pre>
    </details>

    <div class="mt-6 flex flex-wrap gap-3">
      <button type="button" data-role="open-folder" disabled
              class="rounded-xl bg-sky-500 px-5 py-2.5 text-sm font-semibold text-slate-950 transition hover:bg-sky-400 disabled:cursor-not-allowed disabled:bg-slate-800 disabled:text-slate-500">
        Open Output Folder
      </button>
      <button type="button" data-role="reset"
              class="rounded-xl border border-slate-700 px-5 py-2.5 text-sm font-medium text-slate-200 transition hover:border-slate-500 disabled:cursor-not-allowed disabled:opacity-50">
        Compile for Another Platform
      </button>
    </div>
  `;

  const progress = queryRequired<HTMLElement>(section, "[data-role='progress']");
  const bar = queryRequired<HTMLElement>(section, "[data-role='bar']");
  const statusText = queryRequired<HTMLElement>(section, "[data-role='status']");
  const logDrawer = queryRequired<HTMLDetailsElement>(section, "[data-role='logs']");
  const logOutput = queryRequired<HTMLElement>(section, "[data-role='log-output']");
  const logCount = queryRequired<HTMLElement>(section, "[data-role='log-count']");
  const openFolderButton = queryRequired<HTMLButtonElement>(section, "[data-role='open-folder']");
  const resetButton = queryRequired<HTMLButtonElement>(section, "[data-role='reset']");

  let lineCount = 0;

  function appendLog(line: string): void {
    if (lineCount === 0) {
      logOutput.textContent = "";
    }
    logOutput.append(`${line}\n`);
    lineCount += 1;
    logCount.textContent = `${lineCount} ${lineCount === 1 ? "line" : "lines"}`;
    if (logDrawer.open) {
      logOutput.scrollTop = logOutput.scrollHeight;
    }
  }

  resetButton.addEventListener("click", onReset);
  openFolderButton.addEventListener("click", () => {
    const outputPath = openFolderButton.dataset.outputPath;
    if (!outputPath || openFolderButton.disabled) {
      return;
    }
    void Promise.resolve(onOpenFolder(outputPath)).catch((error) => {
      appendLog(`Could not open output folder: ${errorMessage(error)}`);
    });
  });

  return {
    element: section,

    setRunning(running) {
      resetButton.disabled = running;
      if (running) {
        bar.classList.remove("bg-emerald-400", "bg-rose-500");
        bar.classList.add("bg-sky-400");
      }
    },

    setProgress(percent, status) {
      const clamped = Math.min(100, Math.max(0, Math.round(percent)));
      bar.style.width = `${clamped}%`;
      progress.setAttribute("aria-valuenow", String(clamped));
      statusText.textContent = status;
    },

    appendLog,

    setCompleted(outputPath) {
      bar.classList.remove("bg-sky-400", "bg-rose-500");
      bar.classList.add("bg-emerald-400");
      statusText.textContent = "Build complete.";
      openFolderButton.dataset.outputPath = outputPath;
      openFolderButton.disabled = false;
      resetButton.disabled = false;
    },

    setFailed(message) {
      bar.classList.remove("bg-sky-400", "bg-emerald-400");
      bar.classList.add("bg-rose-500");
      statusText.textContent = message;
      resetButton.disabled = false;
    },
  };
}
