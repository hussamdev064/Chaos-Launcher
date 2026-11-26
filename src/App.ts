import {
  errorMessage,
  fetchDockerStatus,
  fetchHostPlatform,
  listenBuildEvents,
  listenDockerStatus,
  openOutputFolder,
  startBuild as invokeStartBuild,
  type BuildEventPayload,
  type DockerStatus,
} from "./api";
import { queryRequired } from "./ui/dom";
import { createBuildProgress, type BuildProgressView } from "./ui/buildProgress";
import { renderDropZone } from "./ui/dropZone";
import { renderTargetSelector } from "./ui/targetSelector";
import type { BuildSelection, StepIndex } from "./types";

const STEP_LABELS = ["Source asset", "Target", "Build"] as const;

interface AppState {
  step: StepIndex;
  selection: BuildSelection;
  building: boolean;
  isMacHost: boolean | null;
  dockerStatus: DockerStatus | null;
}

export function mountApp(root: HTMLElement): void {
  const state: AppState = {
    step: 0,
    selection: {
      asset: null,
      platform: "windows",
      architecture: "x86_64",
      format: "exe",
    },
    building: false,
    isMacHost: null,
    dockerStatus: null,
  };

  root.innerHTML = `
    <div class="mx-auto flex min-h-screen max-w-5xl flex-col px-5 py-8 sm:px-8">
      <header class="flex flex-wrap items-center justify-between gap-4 border-b border-slate-800 pb-6">
        <div>
          <h1 class="text-2xl font-bold tracking-tight">Chaos<span class="text-sky-400">Launcher</span></h1>
          <p class="mt-1 text-sm text-slate-400">Build Sonic Mania for any target, with no local toolchains.</p>
        </div>
        <span data-role="docker-status"
              class="rounded-full border border-slate-700 bg-slate-800/60 px-3 py-1 text-xs text-slate-300">
          Docker: checking…
        </span>
      </header>

      <nav data-role="steps" aria-label="Build steps" class="mt-6 flex flex-wrap gap-2"></nav>

      <main class="flex-1 py-8">
        <div data-role="step-content"></div>
      </main>

      <footer class="flex items-center justify-between border-t border-slate-800 pt-5">
        <button type="button" data-role="back"
                class="rounded-xl px-5 py-2.5 text-sm font-medium text-slate-300 transition hover:bg-slate-800 disabled:cursor-not-allowed disabled:opacity-50">Back</button>
        <button type="button" data-role="next"
                class="rounded-xl bg-sky-500 px-6 py-2.5 text-sm font-semibold text-slate-950 transition hover:bg-sky-400 disabled:cursor-not-allowed disabled:bg-slate-800 disabled:text-slate-500">
          Continue
        </button>
      </footer>
    </div>
  `;

  const stepsNav = queryRequired<HTMLElement>(root, "[data-role='steps']");
  const stepContent = queryRequired<HTMLElement>(root, "[data-role='step-content']");
  const backButton = queryRequired<HTMLButtonElement>(root, "[data-role='back']");
  const nextButton = queryRequired<HTMLButtonElement>(root, "[data-role='next']");
  const dockerBadge = queryRequired<HTMLElement>(root, "[data-role='docker-status']");

  let progressView: BuildProgressView | null = null;

  function hasAssetPath(): boolean {
    return typeof state.selection.asset?.path === "string" && state.selection.asset.path.length > 0;
  }

  function updateFooter(): void {
    backButton.classList.toggle("invisible", state.step === 0);
    backButton.disabled = state.building;
    backButton.textContent = state.building ? "Building…" : "Back";
    nextButton.classList.toggle("hidden", state.step === 2);
    nextButton.disabled = state.building || (state.step === 0 && !hasAssetPath());
    nextButton.textContent = state.step === 1 ? "Start build" : "Continue";
  }

  function applyDockerStatus(status: DockerStatus): void {
    state.dockerStatus = status;
    const tones: Record<DockerStatus["stage"], string> = {
      checking: "border-slate-700 bg-slate-800/60 text-slate-300",
      building: "border-amber-500/30 bg-amber-500/10 text-amber-200",
      ready: "border-emerald-500/30 bg-emerald-500/10 text-emerald-200",
      failed: "border-rose-500/30 bg-rose-500/10 text-rose-200",
    };
    dockerBadge.className = `rounded-full border px-3 py-1 text-xs ${tones[status.stage]}`;
    dockerBadge.textContent = `Docker: ${status.message}`;
  }

  function render(): void {
    progressView = null;
    stepsNav.innerHTML = STEP_LABELS.map((label, index) => {
      const isActive = index === state.step;
      const isDone = index < state.step;
      return `
        <span class="flex items-center gap-2 rounded-full px-3 py-1.5 text-xs ${
          isActive
            ? "bg-sky-500/15 text-sky-300"
            : isDone
              ? "text-emerald-300"
              : "text-slate-500"
        }">
          <span class="flex h-5 w-5 items-center justify-center rounded-full border text-[10px] ${
            isActive ? "border-sky-400" : isDone ? "border-emerald-500" : "border-slate-700"
          }">${index + 1}</span>
          ${label}
        </span>`;
    }).join("");

    updateFooter();

    switch (state.step) {
      case 0:
        stepContent.replaceChildren(
          renderDropZone({
            asset: state.selection.asset,
            onAssetChange: (asset) => {
              state.selection.asset = asset;
              render();
            },
          }),
        );
        break;
      case 1:
        stepContent.replaceChildren(
          renderTargetSelector({
            platform: state.selection.platform,
            architecture: state.selection.architecture,
            format: state.selection.format,
            isMacHost: state.isMacHost ?? undefined,
            onChange: (platform, architecture, format) => {
              state.selection.platform = platform;
              state.selection.architecture = architecture;
              state.selection.format = format;
              render();
            },
          }),
        );
        break;
      case 2: {
        const view = createBuildProgress({
          selection: state.selection,
          onReset: () => {
            if (state.building) {
              return;
            }
            // Keep the validated asset; only the target needs changing.
            state.step = 1;
            render();
          },
          onOpenFolder: openOutputFolder,
        });
        progressView = view;
        stepContent.replaceChildren(view.element);
        break;
      }
    }
  }

  async function startBuild(): Promise<void> {
    if (state.building || !hasAssetPath()) {
      return;
    }

    state.building = true;
    state.step = 2;
    render();

    const view = progressView;
    if (!view) {
      state.building = false;
      updateFooter();
      return;
    }

    view.setRunning(true);
    view.setProgress(0, "Preparing source checkout...");
    view.appendLog("Starting build...");

    let percent = 0;
    let status = "Preparing source checkout...";

    const applyEvent = (event: BuildEventPayload): void => {
      if (event.percent !== null) {
        percent = event.percent;
        if (event.message) {
          status = event.message;
          view.appendLog(event.message);
        }
        view.setProgress(percent, status);
        return;
      }
      if (event.message) {
        status = event.message;
        view.appendLog(event.message);
        view.setProgress(percent, status);
      }
    };

    let unlisten: (() => void) | null = null;
    try {
      unlisten = await listenBuildEvents(applyEvent);
    } catch (error) {
      view.appendLog(`Live logs unavailable: ${errorMessage(error)}`);
    }

    try {
      const asset = state.selection.asset;
      if (!asset?.path) {
        throw new Error("Select Data.rsdk before building.");
      }
      const response = await invokeStartBuild({
        assetPath: asset.path,
        target: {
          platform: state.selection.platform,
          architecture: state.selection.architecture,
          format: state.selection.format,
        },
      });
      view.setProgress(100, "Build complete.");
      view.setCompleted(response.outputDirectory);
      view.appendLog(`Artifacts: ${response.artifacts.join(", ") || "(none)"}`);
      if (response.assetSha256) {
        view.appendLog(`Data.rsdk SHA-256: ${response.assetSha256}`);
      }
    } catch (error) {
      const message = errorMessage(error);
      view.setFailed(`Build failed: ${message}`);
      view.appendLog(message);
    } finally {
      if (unlisten) {
        unlisten();
      }
      state.building = false;
      view.setRunning(false);
      updateFooter();
    }
  }

  backButton.addEventListener("click", () => {
    if (state.building || state.step === 0) {
      return;
    }
    state.step = (state.step - 1) as StepIndex;
    render();
  });

  nextButton.addEventListener("click", () => {
    if (state.building) {
      return;
    }
    if (state.step === 0 && hasAssetPath()) {
      state.step = 1;
      render();
      return;
    }
    if (state.step === 1) {
      void startBuild();
    }
  });

  render();

  void fetchHostPlatform()
    .then((platform) => {
      state.isMacHost = platform === "macos";
      if (!state.building && state.step === 1) {
        render();
      }
    })
    .catch(() => {
      // Keep the user-agent fallback from targetSelector.ts.
    });

  void fetchDockerStatus()
    .then(applyDockerStatus)
    .catch(() =>
      applyDockerStatus({ stage: "failed", message: "Docker status unavailable in this browser" }),
    );

  void listenDockerStatus(applyDockerStatus).catch(() => {
    // The initial fetch above still covers the common startup case.
  });
}
