import {
  errorMessage,
  fetchDockerStatus,
  fetchHostPlatform,
  fullClean,
  listenBuildEvents,
  listenDockerStatus,
  loadSettings,
  openOutputFolder,
  pickOutputFolder,
  previewClean,
  saveSettings,
  startBuild as invokeStartBuild,
  type BuildEventPayload,
  type CleanPlan,
  type CleanReport,
  type DockerStage,
  type DockerStatus,
} from "./api";
import packageInfo from "../package.json";
import { queryRequired } from "./ui/dom";
import { formatBytes } from "./ui/format";
import { renderDropZone } from "./ui/dropZone";
import { formatOption, platformOption, renderTargetSelector } from "./ui/targetSelector";
import { createStatusDock, type ActivityTone, type StatusDock } from "./ui/statusDock";
import type { BuildSelection } from "./types";

const PHASE_LABELS: Record<string, string> = {
  validating: "Validating asset",
  environment: "Environment",
  configuring: "Configuring",
  compiling: "Compiling",
  packaging: "Packaging",
  "local-data": "Local Data.rsdk copy",
  complete: "Complete",
  cleaning: "Cleaning",
};

const PHASE_TONES: Record<string, ActivityTone> = {
  complete: "success",
  cleaning: "warning",
  "local-data": "warning",
};

const DOCKER_BADGE_TONES: Record<DockerStage, string> = {
  checking: "border-slate-700 bg-slate-800/60 text-slate-300",
  building: "border-amber-500/30 bg-amber-500/10 text-amber-200",
  ready: "border-emerald-500/30 bg-emerald-500/10 text-emerald-200",
  failed: "border-rose-500/30 bg-rose-500/10 text-rose-200",
};

interface AppState {
  selection: BuildSelection;
  building: boolean;
  cleaning: boolean;
  isMacHost: boolean | null;
  dockerStatus: DockerStatus | null;
  /** Persisted custom output root; empty means use the managed cache. */
  outputPath: string;
  /** Persisted opt-in; false keeps `-DRETRO_DISABLE_PLUS=ON`. */
  includePlus: boolean;
  /** Persisted opt-in; false keeps game data out of generated outputs. */
  includeDataRsdk: boolean;
  lastOutputPath: string | null;
  lastArtifacts: string[];
  lastDataRsdkPath: string | null;
}

export function mountApp(root: HTMLElement): void {
  const state: AppState = {
    selection: {
      asset: null,
      platform: "windows",
      architecture: "x86_64",
      format: "exe",
    },
    building: false,
    cleaning: false,
    isMacHost: null,
    dockerStatus: null,
    outputPath: "",
    includePlus: false,
    includeDataRsdk: false,
    lastOutputPath: null,
    lastArtifacts: [],
    lastDataRsdkPath: null,
  };

  root.innerHTML = `
    <div class="app-shell">
      <aside class="app-sidebar">
        <div class="side-brand">
          <span class="brand-mark" aria-hidden="true">CL</span>
          <div class="min-w-0">
            <p class="truncate font-semibold tracking-tight">ChaosLauncher</p>
            <span class="version-badge">v${packageInfo.version}</span>
          </div>
        </div>
        <p class="side-tagline">Sonic Mania builds with no local toolchains.</p>

        <p class="side-section-label">Workspace</p>
        <nav class="side-nav" aria-label="Launcher sections">
          <button type="button" data-nav="build" class="nav-item is-active">
            <span class="nav-dot" aria-hidden="true"></span><span>Build setup</span>
          </button>
          <button type="button" data-nav="destination" class="nav-item">
            <span class="nav-dot" aria-hidden="true"></span><span>Destination</span>
          </button>
          <button type="button" data-nav="settings" class="nav-item">
            <span class="nav-dot" aria-hidden="true"></span><span>Settings</span>
          </button>
          <button type="button" data-nav="maintenance" class="nav-item">
            <span class="nav-dot" aria-hidden="true"></span><span>Maintenance</span>
          </button>
        </nav>

        <div class="sidebar-foot">
          <p class="side-section-label !mt-0">Runtime</p>
          <div data-role="docker-badge" class="side-status">Docker: checking…</div>
          <p class="mt-3 text-[11px] leading-relaxed text-slate-600">
            GPLv3 launcher · game assets stay on your machine.
          </p>
        </div>
      </aside>

      <div class="workspace-column">
        <header class="app-toolbar">
          <div class="min-w-0">
            <p class="toolbar-kicker">Sonic Mania builder</p>
            <h1>Build dashboard</h1>
            <p class="toolbar-subtitle">Pick your data, choose a target, and watch the whole build live.</p>
          </div>
          <div class="toolbar-actions">
            <button type="button" data-action="build" class="toolbar-button toolbar-button-primary">Build</button>
            <button type="button" data-action="open" class="toolbar-button">Open Output</button>
            <button type="button" data-action="clean" class="toolbar-button toolbar-button-danger">Full Clean</button>
          </div>
        </header>

        <main class="workspace">
          <section class="panel" data-panel="build" aria-labelledby="build-heading">
            <div class="panel-head">
              <div>
                <p class="panel-kicker">01 / Configuration</p>
                <h2 id="build-heading">Build setup</h2>
              </div>
              <span data-role="asset-chip" class="stat-chip">Asset required</span>
            </div>

            <div class="summary-grid">
              <div class="summary-stat">
                <span>Source data</span>
                <strong data-role="summary-asset">Not selected</strong>
              </div>
              <div class="summary-stat">
                <span>Target</span>
                <strong data-role="summary-target">Windows · x86-64 · Standalone .exe</strong>
              </div>
              <div class="summary-stat">
                <span>Destination</span>
                <strong data-role="summary-output">Managed cache</strong>
              </div>
              <div class="summary-stat">
                <span>Build runtime</span>
                <strong data-role="summary-runtime">Docker container</strong>
              </div>
            </div>

            <div class="config-grid">
              <article class="panel-card">
                <div class="panel-card-head">
                  <div>
                    <h3>Game data</h3>
                    <p>Your own legally obtained Data.rsdk; never downloaded or bundled.</p>
                  </div>
                  <span class="card-index">01</span>
                </div>
                <div data-role="source-panel"></div>
              </article>

              <article class="panel-card">
                <div class="panel-card-head">
                  <div>
                    <h3>Build target</h3>
                    <p>Platform, architecture, and output format.</p>
                  </div>
                  <span class="card-index">02</span>
                </div>
                <div data-role="target-panel"></div>
              </article>
            </div>
          </section>

          <section class="panel" data-panel="destination" aria-labelledby="destination-heading">
            <div class="panel-head">
              <div>
                <p class="panel-kicker">02 / Output</p>
                <h2 id="destination-heading">Destination</h2>
              </div>
              <span class="stat-chip">Optional</span>
            </div>
            <div class="panel-card">
              <div class="panel-card-head">
                <div>
                  <h3>Custom output folder</h3>
                  <p>Leave empty to use the launcher cache; builds always create a timestamped child folder.</p>
                </div>
                <span class="card-index">03</span>
              </div>
              <label class="field-label" for="output-path">Folder path</label>
              <div class="input-row">
                <input id="output-path" data-role="output-input" type="text" spellcheck="false"
                       autocomplete="off" placeholder="~/SonicMania-builds (or leave empty)" />
                <button type="button" data-role="browse-output" class="toolbar-button">Browse…</button>
                <button type="button" data-role="clear-output" class="toolbar-button">Use default</button>
              </div>
              <p data-role="output-hint" class="field-hint">Choose a folder or keep the managed default.</p>
            </div>
          </section>

          <section class="panel" data-panel="settings" aria-labelledby="settings-heading">
            <div class="panel-head">
              <div>
                <p class="panel-kicker">03 / Preferences</p>
                <h2 id="settings-heading">Settings</h2>
              </div>
              <span data-role="plus-chip" class="stat-chip">Plus disabled</span>
            </div>
            <div class="config-grid">
              <article class="panel-card">
                <div class="panel-card-head">
                  <div>
                    <h3>Compilation</h3>
                    <p>Optional Plus/DLC content for locally owned installations.</p>
                  </div>
                  <span class="card-index">04</span>
                </div>
                <label class="checkbox-row">
                  <input type="checkbox" data-role="plus-toggle" />
                  <span>
                    <span class="block text-[13px] font-semibold text-slate-100">Include Plus/DLC content</span>
                    <span class="mt-1 block text-[11px] leading-relaxed text-slate-400">
                      Off by default keeps <code class="text-sky-200">-DRETRO_DISABLE_PLUS=ON</code>.
                      Enable only with legitimate Plus data; builds then use
                      <code class="text-sky-200">-DRETRO_DISABLE_PLUS=OFF</code>.
                    </span>
                  </span>
                </label>
                <p data-role="plus-hint" class="field-hint">Loading local settings…</p>
              </article>

              <article class="panel-card">
                <div class="panel-card-head">
                  <div>
                    <h3>Local game data</h3>
                    <p>Optional host-only copy of the Data.rsdk you selected.</p>
                  </div>
                  <span data-role="data-chip" class="stat-chip">Data copy off</span>
                </div>
                <label class="checkbox-row">
                  <input type="checkbox" data-role="data-toggle" />
                  <span>
                    <span class="block text-[13px] font-semibold text-slate-100">Copy my Data.rsdk into the build output</span>
                    <span class="mt-1 block text-[11px] leading-relaxed text-slate-400">
                      Off by default, so generated outputs stay clean. The copy is made on this host
                      after the build; your original file is never modified or moved.
                    </span>
                  </span>
                </label>
                <p class="warning-box">
                  <strong>Local use only:</strong> enabling this bundles your own local Data.rsdk into
                  the built game copy for personal local use only. The resulting file must not be
                  shared, distributed, or uploaded publicly. ChaosLauncher never downloads, embeds,
                  or ships game data in its own source, releases, or AppImage.
                </p>
                <p data-role="data-hint" class="field-hint">Loading local settings…</p>
              </article>

              <article class="panel-card">
                <div class="panel-card-head">
                  <div>
                    <h3>Pinned defaults</h3>
                    <p>Compilation rules this launcher will not change.</p>
                  </div>
                  <span class="card-index">06</span>
                </div>
                <ul class="clean-list">
                  <li><span class="text-emerald-300">Always pinned:</span> <code class="text-sky-200">-DRETRO_REVISION=5</code>.</li>
                  <li><span class="text-sky-300">Android:</span> mod loader v2, shared <code class="text-sky-200">/storage/emulated/0/RSDK/v5</code>, and SAF storage access.</li>
                  <li><span class="text-sky-300">Destination:</span> managed cache, or the folder chosen under Destination.</li>
                  <li><span class="text-amber-300">Legal use:</span> supply your own Data.rsdk and any Plus data; nothing is downloaded, and launcher releases contain no game data.</li>
                </ul>
              </article>
            </div>
          </section>

          <section class="panel" data-panel="maintenance" aria-labelledby="maintenance-heading">
            <div class="panel-head">
              <div>
                <p class="panel-kicker">04 / Maintenance</p>
                <h2 id="maintenance-heading">Full Clean</h2>
              </div>
              <span class="stat-chip stat-chip-danger">Confirmation required</span>
            </div>
            <div class="clean-layout">
              <div>
                <p class="text-sm leading-relaxed text-slate-300">
                  Removes only launcher-created caches, the cached source checkout, generated build
                  folders (including Android RSDK/v5 staging), build containers, and the builder
                  image. Nothing is deleted until you review the exact target list and confirm.
                </p>
                <ul class="clean-list">
                  <li><span class="text-emerald-300">Never removed:</span> your Data.rsdk, ~/Downloads/Sonic, Android shared storage, system files, unrelated folders, Docker sockets, or your custom output folder itself.</li>
                  <li><span class="text-amber-300">Re-downloaded later:</span> the source checkout and builder image are recreated on the next build.</li>
                </ul>
              </div>
              <div class="clean-controls">
                <label class="checkbox-row">
                  <input type="checkbox" data-role="include-custom" />
                  <span>Also delete generated build folders inside my custom output folder</span>
                </label>
                <p class="field-hint">Only timestamped folders ChaosLauncher recorded are eligible; the chosen folder itself is kept.</p>
                <button type="button" data-role="clean-panel-button" class="toolbar-button toolbar-button-danger">Review &amp; clean</button>
              </div>
            </div>
            <pre data-role="clean-report" class="clean-report" hidden></pre>
          </section>

          <section class="panel result-panel" data-role="result-card" hidden>
            <div class="panel-head">
              <div>
                <p class="panel-kicker">Latest build</p>
                <h2>Artifacts ready</h2>
              </div>
              <span class="stat-chip stat-chip-success">Complete</span>
            </div>
            <p data-role="result-path" class="result-path"></p>
            <p data-role="result-data-note" class="result-note" hidden></p>
            <ul data-role="result-artifacts" class="artifact-list"></ul>
            <div class="result-actions">
              <button type="button" data-role="open-result" class="toolbar-button toolbar-button-primary">Open build folder</button>
              <button type="button" data-role="compile-another" class="toolbar-button">Change target</button>
            </div>
          </section>
        </main>

        <div data-role="dock-host"></div>
      </div>
    </div>
  `;

  const dockerBadge = queryRequired<HTMLElement>(root, "[data-role='docker-badge']");
  const sourceHost = queryRequired<HTMLElement>(root, "[data-role='source-panel']");
  const targetHost = queryRequired<HTMLElement>(root, "[data-role='target-panel']");
  const assetChip = queryRequired<HTMLElement>(root, "[data-role='asset-chip']");
  const summaryAsset = queryRequired<HTMLElement>(root, "[data-role='summary-asset']");
  const summaryTarget = queryRequired<HTMLElement>(root, "[data-role='summary-target']");
  const summaryOutput = queryRequired<HTMLElement>(root, "[data-role='summary-output']");
  const summaryRuntime = queryRequired<HTMLElement>(root, "[data-role='summary-runtime']");
  const outputInput = queryRequired<HTMLInputElement>(root, "[data-role='output-input']");
  const outputHint = queryRequired<HTMLElement>(root, "[data-role='output-hint']");
  const plusToggle = queryRequired<HTMLInputElement>(root, "[data-role='plus-toggle']");
  const plusHint = queryRequired<HTMLElement>(root, "[data-role='plus-hint']");
  const plusChip = queryRequired<HTMLElement>(root, "[data-role='plus-chip']");
  const dataToggle = queryRequired<HTMLInputElement>(root, "[data-role='data-toggle']");
  const dataHint = queryRequired<HTMLElement>(root, "[data-role='data-hint']");
  const dataChip = queryRequired<HTMLElement>(root, "[data-role='data-chip']");
  const includeCustomInput = queryRequired<HTMLInputElement>(root, "[data-role='include-custom']");
  const cleanReport = queryRequired<HTMLPreElement>(root, "[data-role='clean-report']");
  const resultCard = queryRequired<HTMLElement>(root, "[data-role='result-card']");
  const resultPath = queryRequired<HTMLElement>(root, "[data-role='result-path']");
  const resultDataNote = queryRequired<HTMLElement>(root, "[data-role='result-data-note']");
  const resultArtifacts = queryRequired<HTMLUListElement>(root, "[data-role='result-artifacts']");
  const buildButton = queryRequired<HTMLButtonElement>(root, "[data-action='build']");
  const openButton = queryRequired<HTMLButtonElement>(root, "[data-action='open']");
  const cleanButton = queryRequired<HTMLButtonElement>(root, "[data-action='clean']");
  const cleanPanelButton = queryRequired<HTMLButtonElement>(root, "[data-role='clean-panel-button']");
  const browseOutputButton = queryRequired<HTMLButtonElement>(root, "[data-role='browse-output']");
  const clearOutputButton = queryRequired<HTMLButtonElement>(root, "[data-role='clear-output']");
  const openResultButton = queryRequired<HTMLButtonElement>(root, "[data-role='open-result']");
  const compileAnotherButton = queryRequired<HTMLButtonElement>(root, "[data-role='compile-another']");

  const dock: StatusDock = createStatusDock();
  queryRequired<HTMLElement>(root, "[data-role='dock-host']").replaceChildren(dock.element);

  let currentPhaseLabel = "Idle";
  let lastActivityKey = "";
  let lastDockerStage: DockerStage | null = null;
  let persistedOutput = "";
  let persistedIncludePlus = false;
  let persistedIncludeDataRsdk = false;
  let settingsLoad: Promise<void> = Promise.resolve();
  let settingsSave: Promise<void> = Promise.resolve();

  function targetLabel(): string {
    const platform = platformOption(state.selection.platform).label;
    const format = formatOption(state.selection.platform, state.selection.format)?.label ?? state.selection.format;
    return `${platform} · ${state.selection.architecture} · ${format}`;
  }

  function busy(): boolean {
    return state.building || state.cleaning;
  }

  function updateToolbar(): void {
    const locked = busy();
    buildButton.disabled = locked || !state.selection.asset?.path;
    buildButton.textContent = state.building ? "Building…" : "Build";
    openButton.disabled = locked;
    cleanButton.disabled = locked;
    cleanButton.textContent = state.cleaning ? "Cleaning…" : "Full Clean";
    cleanPanelButton.disabled = locked;
    cleanPanelButton.textContent = state.cleaning ? "Cleaning…" : "Review & clean";
    plusToggle.disabled = locked;
    dataToggle.disabled = locked;
    includeCustomInput.disabled = locked;
  }

  function updateSummary(): void {
    const asset = state.selection.asset;
    assetChip.textContent = asset ? "Asset ready" : "Asset required";
    assetChip.classList.toggle("stat-chip-success", Boolean(asset));
    summaryAsset.textContent = asset ? `${asset.name} · ${formatBytes(asset.sizeBytes)}` : "Not selected";
    summaryTarget.textContent = targetLabel();
    summaryOutput.textContent = state.outputPath.trim() || "Managed cache";
    const runtime =
      state.selection.platform === "macos" ? "Native Xcode host" : "Docker container";
    const dockerSuffix = state.dockerStatus ? ` · ${state.dockerStatus.stage}` : "";
    summaryRuntime.textContent = `${runtime}${dockerSuffix}`;
    updateToolbar();
  }

  function setOutputHint(message: string, isError = false): void {
    outputHint.textContent = message;
    outputHint.classList.toggle("is-error", isError);
  }

  function updateSettingsUi(): void {
    plusToggle.checked = state.includePlus;
    plusChip.textContent = state.includePlus ? "Plus included" : "Plus disabled";
    plusChip.classList.toggle("stat-chip-warning", state.includePlus);
    plusHint.classList.remove("is-error");
    plusHint.classList.toggle("is-warning", state.includePlus);
    plusHint.textContent = state.includePlus
      ? "Saved locally · builds pass -DRETRO_DISABLE_PLUS=OFF and -DRSDK_AUTOBUILD=0."
      : "Saved locally · builds keep -DRETRO_DISABLE_PLUS=ON and -DRSDK_AUTOBUILD=1.";

    dataToggle.checked = state.includeDataRsdk;
    dataChip.textContent = state.includeDataRsdk ? "Local copy on" : "Data copy off";
    dataChip.classList.toggle("stat-chip-warning", state.includeDataRsdk);
    dataHint.classList.remove("is-error");
    dataHint.classList.toggle("is-warning", state.includeDataRsdk);
    dataHint.textContent = state.includeDataRsdk
      ? "Saved locally · Data.rsdk is copied beside desktop outputs and staged as RSDK/v5 for Android. Copy it to shared storage on the device yourself; do not share, distribute, or upload it."
      : "Saved locally · generated outputs contain no game-data copy.";
  }

  function renderSource(): void {
    sourceHost.replaceChildren(
      renderDropZone({
        asset: state.selection.asset,
        variant: "compact",
        onAssetChange: (asset) => {
          state.selection.asset = asset;
          dock.addActivity("Data.rsdk selected", asset.name, "success");
          renderSource();
          updateSummary();
        },
      }),
    );
  }

  function renderTarget(): void {
    targetHost.replaceChildren(
      renderTargetSelector({
        platform: state.selection.platform,
        architecture: state.selection.architecture,
        format: state.selection.format,
        isMacHost: state.isMacHost ?? undefined,
        variant: "compact",
        onChange: (platform, architecture, format) => {
          state.selection.platform = platform;
          state.selection.architecture = architecture;
          state.selection.format = format;
          dock.addActivity("Target updated", targetLabel());
          renderTarget();
          updateSummary();
        },
      }),
    );
  }

  function renderResult(): void {
    const path = state.lastOutputPath;
    resultCard.hidden = !path;
    if (!path) {
      return;
    }
    resultPath.textContent = path;
    resultDataNote.hidden = !state.lastDataRsdkPath;
    resultDataNote.textContent = state.lastDataRsdkPath
      ? `Local Data.rsdk included: ${state.lastDataRsdkPath} · personal local use only; do not share, distribute, or upload.`
      : "";
    resultArtifacts.replaceChildren(
      ...state.lastArtifacts.map((artifact) => {
        const item = document.createElement("li");
        item.textContent = artifact;
        return item;
      }),
    );
  }

  function applyBuildEvent(event: BuildEventPayload): void {
    const message = event.message ?? "";
    if (event.phase) {
      const label = PHASE_LABELS[event.phase] ?? event.phase;
      currentPhaseLabel = label;
      dock.setPhase(label, message || `${label}…`);
      const key = `${event.phase}:${message}`;
      if (key !== lastActivityKey) {
        lastActivityKey = key;
        dock.addActivity(label, message, PHASE_TONES[event.phase] ?? "info");
      }
      if (message) {
        dock.appendLog(message);
      }
      return;
    }

    if (event.percent !== null) {
      dock.setProgress(event.percent, message);
      if (message) {
        dock.appendLog(message);
      }
      return;
    }

    if (message) {
      dock.setPhase(currentPhaseLabel, message);
      dock.appendLog(message);
    }
  }

  const listenerReady: Promise<void> = listenBuildEvents(applyBuildEvent)
    .then(() => undefined)
    .catch((error: unknown) => {
      dock.addActivity("Live build events unavailable", errorMessage(error), "warning");
    });

  function applyDockerStatus(status: DockerStatus): void {
    state.dockerStatus = status;
    dockerBadge.className = `side-status ${DOCKER_BADGE_TONES[status.stage]}`;
    dockerBadge.textContent = `Docker: ${status.message}`;
    if (status.stage !== lastDockerStage) {
      lastDockerStage = status.stage;
      const tone: ActivityTone =
        status.stage === "ready"
          ? "success"
          : status.stage === "failed"
            ? "warning"
            : "info";
      dock.addActivity(`Docker ${status.stage}`, status.message, tone);
      updateSummary();
    }
  }

  async function persistSettings(): Promise<void> {
    // Local picker/load always resolves first, so a fast toggle cannot overwrite
    // the output path before it has been read from disk.
    await settingsLoad;
    const value = outputInput.value.trim();
    if (
      value === persistedOutput &&
      state.includePlus === persistedIncludePlus &&
      state.includeDataRsdk === persistedIncludeDataRsdk
    ) {
      return;
    }
    try {
      await saveSettings({
        outputPath: value || null,
        includePlus: state.includePlus,
        includeDataRsdk: state.includeDataRsdk,
      });
      persistedOutput = value;
      persistedIncludePlus = state.includePlus;
      persistedIncludeDataRsdk = state.includeDataRsdk;
      state.outputPath = value;
      setOutputHint(
        value
          ? `Saved locally · builds create timestamped folders in ${value}`
          : "Saved locally · using managed cache outputs",
      );
      updateSettingsUi();
      dock.addActivity(
        "Settings saved",
        [
          value || "managed cache",
          state.includePlus ? "Plus/DLC enabled" : "Plus/DLC disabled",
          state.includeDataRsdk ? "local Data.rsdk copy on" : "local Data.rsdk copy off",
        ].join(" · "),
        state.includeDataRsdk ? "warning" : "success",
      );
      updateSummary();
    } catch (error) {
      const message = errorMessage(error);
      state.includePlus = persistedIncludePlus;
      state.includeDataRsdk = persistedIncludeDataRsdk;
      updateSettingsUi();
      setOutputHint(message, true);
      plusHint.textContent = `Could not save settings: ${message}`;
      plusHint.classList.add("is-error");
      dataHint.textContent = `Could not save settings: ${message}`;
      dataHint.classList.add("is-error");
      dock.addActivity("Could not save settings", message, "error");
    }
  }

  function queueSettingsSave(): void {
    settingsSave = persistSettings();
  }

  async function openPath(path: string | null): Promise<void> {
    if (!path) {
      dock.addActivity("No output folder yet", "Run a build or choose a destination first", "warning");
      return;
    }
    try {
      await openOutputFolder(path);
      dock.addActivity("Opened output folder", path, "success");
    } catch (error) {
      const message = errorMessage(error);
      dock.addActivity("Could not open output folder", message, "error");
    }
  }

  async function runBuild(): Promise<void> {
    if (busy()) {
      return;
    }
    const asset = state.selection.asset;
    if (!asset?.path) {
      dock.addActivity("Build needs Data.rsdk", "Select your local game data first", "warning");
      return;
    }

    await listenerReady;
    // The Rust command reads the persisted setting, so wait for any pending
    // local save before starting the build.
    await settingsLoad;
    await settingsSave;
    state.building = true;
    updateToolbar();
    dock.setBusy(true);
    dock.clearLog();
    dock.setPhase("Starting", "Preparing the build...");
    dock.setProgress(0, "Starting build...");
    dock.addActivity("Build started", targetLabel());

    try {
      const response = await invokeStartBuild({
        assetPath: asset.path,
        target: {
          platform: state.selection.platform,
          architecture: state.selection.architecture,
          format: state.selection.format,
        },
        outputPath: state.outputPath.trim() || null,
      });

      state.lastOutputPath = response.outputDirectory;
      state.lastArtifacts = response.artifacts;
      state.lastDataRsdkPath = response.dataRsdkOutputPath ?? null;
      renderResult();
      dock.setPhase("Complete", "Build complete.");
      dock.setProgress(100, "Build complete.");
      dock.addActivity("Build complete", response.outputDirectory, "success");
      dock.appendLog(`Artifacts: ${response.artifacts.join(", ") || "(none)"}`);
      if (response.assetSha256) {
        dock.appendLog(`Data.rsdk SHA-256: ${response.assetSha256}`);
      }
      if (response.dataRsdkIncluded && response.dataRsdkOutputPath) {
        dock.addActivity(
          "Local Data.rsdk included",
          `${response.dataRsdkOutputPath} · personal local use only; do not share or upload`,
          "warning",
        );
        dock.appendLog(
          "Data.rsdk included for personal local use only — do not share, distribute, or upload.",
        );
        if (state.selection.platform === "android") {
          dock.appendLog(
            "Android: copy the staged RSDK folder to /storage/emulated/0/RSDK on the device; the APK contains no game data.",
          );
        }
      } else {
        dock.addActivity("Data.rsdk copy skipped", "Build output contains no game-data copy", "info");
      }
    } catch (error) {
      const message = errorMessage(error);
      dock.setPhase("Failed", message);
      dock.setError(message);
      dock.addActivity("Build failed", message, "error");
      dock.appendLog(message);
    } finally {
      state.building = false;
      dock.setBusy(false);
      updateToolbar();
    }
  }

  function planSection(title: string, items: string[]): string {
    const body = items.length
      ? items.map((item) => `  • ${item}`).join("\n")
      : "  (none)";
    return `${title}:\n${body}`;
  }

  function describePlan(plan: CleanPlan, includeCustomPaths: boolean): string {
    const customPaths = plan.customOutputPaths.map((path) =>
      includeCustomPaths ? `${path} (will be removed)` : `${path} (kept)`,
    );
    return [
      "ChaosLauncher Full Clean will target only these launcher-created items:",
      planSection("Filesystem roots", plan.filesystemPaths),
      planSection("Generated folders in custom output roots", customPaths),
      planSection("Build containers", plan.containers),
      planSection("Builder image", plan.images),
      planSection("Protected references", plan.protectedPaths),
      planSection("Warnings", plan.warnings),
      "",
      "Never deleted: your original Data.rsdk, system files, unrelated directories, Docker sockets, and the custom output folder itself.",
      includeCustomPaths
        ? "The custom output folder itself is preserved; only recorded timestamped children are removed."
        : "Generated builds inside custom output folders are kept because the option is unchecked.",
    ].join("\n");
  }

  function renderCleanReport(report: CleanReport): void {
    cleanReport.textContent = [
      "Full Clean finished",
      planSection("Removed filesystem paths", report.removedPaths),
      planSection("Kept custom build folders", report.keptPaths),
      planSection("Removed containers", report.removedContainers),
      planSection("Removed images", report.removedImages),
      planSection("Protected references", report.protectedPaths),
      planSection("Warnings", report.warnings),
    ].join("\n");
    cleanReport.hidden = false;
  }

  async function runFullClean(): Promise<void> {
    if (busy()) {
      return;
    }
    await listenerReady;
    const includeCustom = includeCustomInput.checked;

    try {
      dock.addActivity("Preparing Full Clean", "Reading launcher-created targets...");
      const plan = await previewClean();
      if (!window.confirm(describePlan(plan, includeCustom))) {
        dock.addActivity("Full Clean cancelled", "Nothing was removed", "warning");
        return;
      }

      state.cleaning = true;
      updateToolbar();
      dock.setBusy(true);
      dock.clearLog();
      dock.setPhase("Cleaning", "Full Clean started.");
      dock.addActivity(
        "Full Clean started",
        includeCustom ? "including custom build folders" : "keeping custom build folders",
        "warning",
      );

      const report = await fullClean(includeCustom);
      renderCleanReport(report);
      if (state.lastOutputPath && report.removedPaths.includes(state.lastOutputPath)) {
        state.lastOutputPath = null;
        state.lastArtifacts = [];
        state.lastDataRsdkPath = null;
        renderResult();
      }

      dock.setPhase("Complete", "Full Clean finished.");
      dock.setProgress(100, "Full Clean finished.");
      dock.addActivity(
        "Full Clean finished",
        `${report.removedPaths.length} path(s), ${report.removedContainers.length} container(s), ${report.removedImages.length} image(s)`,
        report.warnings.length ? "warning" : "success",
      );
      for (const path of report.removedPaths) {
        dock.appendLog(`Removed: ${path}`);
      }
      for (const path of report.keptPaths) {
        dock.appendLog(`Kept: ${path}`);
      }
      for (const name of report.removedContainers) {
        dock.appendLog(`Removed container: ${name}`);
      }
      for (const name of report.removedImages) {
        dock.appendLog(`Removed image: ${name}`);
      }
      for (const warning of report.warnings) {
        dock.appendLog(`Warning: ${warning}`);
      }
    } catch (error) {
      const message = errorMessage(error);
      dock.setPhase("Failed", message);
      dock.setError(message);
      dock.addActivity("Full Clean failed", message, "error");
      dock.appendLog(message);
    } finally {
      state.cleaning = false;
      dock.setBusy(false);
      updateToolbar();
    }
  }

  buildButton.addEventListener("click", () => void runBuild());
  openButton.addEventListener("click", () => void openPath(state.outputPath.trim() || state.lastOutputPath));
  cleanButton.addEventListener("click", () => void runFullClean());
  cleanPanelButton.addEventListener("click", () => void runFullClean());
  openResultButton.addEventListener("click", () => void openPath(state.lastOutputPath));

  compileAnotherButton.addEventListener("click", () => {
    const buildPanel = queryRequired<HTMLElement>(root, "[data-panel='build']");
    buildPanel.scrollIntoView({ behavior: "smooth", block: "start" });
    targetHost.querySelector<HTMLElement>("h2")?.focus();
  });

  browseOutputButton.addEventListener("click", () => {
    void (async () => {
      try {
        const picked = await pickOutputFolder();
        if (!picked) {
          return;
        }
        outputInput.value = picked;
        queueSettingsSave();
        await settingsSave;
      } catch (error) {
        const message = errorMessage(error);
        setOutputHint(message, true);
        dock.addActivity("Could not choose output folder", message, "error");
      }
    })();
  });

  clearOutputButton.addEventListener("click", () => {
    outputInput.value = "";
    queueSettingsSave();
  });
  outputInput.addEventListener("change", () => queueSettingsSave());
  plusToggle.addEventListener("change", () => {
    state.includePlus = plusToggle.checked;
    updateSettingsUi();
    queueSettingsSave();
  });
  dataToggle.addEventListener("change", () => {
    state.includeDataRsdk = dataToggle.checked;
    updateSettingsUi();
    queueSettingsSave();
  });
  outputInput.addEventListener("keydown", (event) => {
    if (event.key === "Enter") {
      outputInput.blur();
    }
  });

  root.querySelectorAll<HTMLButtonElement>("[data-nav]").forEach((button) => {
    button.addEventListener("click", () => {
      const panel = queryRequired<HTMLElement>(root, `[data-panel='${button.dataset.nav}']`);
      root.querySelectorAll<HTMLElement>(".nav-item").forEach((item) => {
        item.classList.toggle("is-active", item === button);
      });
      panel.scrollIntoView({ behavior: "smooth", block: "start" });
    });
  });

  renderSource();
  renderTarget();
  renderResult();
  updateSettingsUi();
  updateSummary();

  settingsLoad = loadSettings()
    .then((settings) => {
      const value = settings.outputPath ?? "";
      state.outputPath = value;
      persistedOutput = value;
      outputInput.value = value;
      state.includePlus = Boolean(settings.includePlus);
      persistedIncludePlus = state.includePlus;
      state.includeDataRsdk = Boolean(settings.includeDataRsdk);
      persistedIncludeDataRsdk = state.includeDataRsdk;
      setOutputHint(
        value
          ? `Loaded from local settings · builds create folders in ${value}`
          : "Loaded from local settings · using managed cache outputs",
      );
      updateSettingsUi();
      updateSummary();
    })
    .catch((error: unknown) => {
      const message = errorMessage(error);
      state.includePlus = false;
      state.includeDataRsdk = false;
      plusToggle.checked = false;
      dataToggle.checked = false;
      plusChip.textContent = "Plus disabled (fallback)";
      dataChip.textContent = "Data copy off (fallback)";
      plusHint.textContent = `Settings unavailable: ${message}`;
      dataHint.textContent = `Settings unavailable: ${message}`;
      plusHint.classList.add("is-error");
      dataHint.classList.add("is-error");
      setOutputHint(`Settings unavailable: ${message}`, true);
      dock.addActivity("Settings unavailable", message, "warning");
    });

  void fetchHostPlatform()
    .then((platform) => {
      state.isMacHost = platform === "macos";
      renderTarget();
    })
    .catch(() => {
      // The target selector's user-agent fallback remains available.
    });

  void fetchDockerStatus().then(applyDockerStatus).catch((error: unknown) => {
    applyDockerStatus({
      stage: "failed",
      message: `Docker status unavailable: ${errorMessage(error)}`,
    });
  });

  void listenDockerStatus(applyDockerStatus).catch(() => {
    // The initial fetch above still covers the common startup case.
  });
}
