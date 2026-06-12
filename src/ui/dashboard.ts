import * as api from "../api";
import type {
  DoctorCheck,
  GatewayStatus,
  SettingsView,
  UsageStatsSnapshot,
} from "../types";
import { copyText, el, iconStatus, settingCard, toast } from "./components";
import { renderAnalytics } from "./analytics";

export interface DashboardState {
  settings: SettingsView;
  status: GatewayStatus;
  logs: string[];
  usage: UsageStatsSnapshot | null;
  doctor: DoctorCheck[] | null;
}

// ─── small helpers ────────────────────────────────────────────────────────────
function muted(text: string): string {
  return `<span style="color:#8a8a8a">${text}</span>`;
}

function sectionTitle(title: string): HTMLElement {
  const h = el("h2", "text-sm font-semibold uppercase tracking-widest mb-4");
  h.style.color = "#8a8a8a";
  h.textContent = title;
  return h;
}

function divider(): HTMLElement {
  const d = el("div", "my-6");
  d.style.cssText = "height:1px;background:#ece6e6";
  return d;
}

// ─── Main render ──────────────────────────────────────────────────────────────
export function renderDashboard(
  root: HTMLElement,
  state: DashboardState,
  onRefresh: () => Promise<void>,
): void {
  root.replaceChildren();

  const wrapper = el("div", "min-h-screen");

  // ── Header ──────────────────────────────────────────────────────────────────
  const header = el("header", "sticky top-0 z-20 px-6 py-3.5 flex items-center justify-between");
  header.style.cssText = `
    background: rgba(255,255,255,0.85);
    border-bottom: 1px solid #ece6e6;
    backdrop-filter: blur(20px);
  `;

  // Logo
  const logoGroup = el("div", "flex items-center gap-3");
  const logoIcon = el("div", "w-8 h-8 rounded-lg flex items-center justify-center text-lg font-bold text-white shrink-0");
  logoIcon.style.cssText = "background:linear-gradient(135deg,#df3e2b,#e8b87a);box-shadow:0 2px 12px rgba(223,62,43,0.35)";
  logoIcon.textContent = "K";
  const logoText = el("div", "");
  const logoSub = el("span", "");
  logoSub.style.color = "#8a8a8a";
  logoSub.textContent = "Moonshot → Cursor bridge";
  logoText.append(
    el("div", "text-base font-bold gradient-text", ["Kimi Cursor Gateway"]),
    el("div", "text-xs", [logoSub]),
  );
  logoGroup.append(logoIcon, logoText);
  header.append(logoGroup);

  // Status pill
  const running = state.status.running;
  const pill = el("div", "flex items-center gap-2 px-3.5 py-2 rounded-full text-sm font-medium");
  pill.style.cssText = running
    ? "background:#e8f5e9;border:1px solid #a5d6a7;color:#2e7d32"
    : "background:#fdecea;border:1px solid #f5c6bf;color:#c23520";
  pill.append(iconStatus(running), document.createTextNode(running ? "Running" : "Stopped"));
  header.append(pill);
  wrapper.append(header);

  const main = el("main", "p-5 max-w-5xl mx-auto space-y-5");
  wrapper.append(main);

  // ── Cursor misalignment banner ───────────────────────────────────────────────
  const alignment = state.status.cursorAlignment;
  if (alignment && !alignment.aligned) {
    const banner = el("div", "p-4 rounded-feex-sm flex items-start gap-4");
    banner.style.cssText = "background:#fdecea;border:1px solid #f5c6bf";
    const icon = el("div", "text-xl shrink-0", ["⚠️"]);
    const body = el("div", "flex-1 min-w-0");
    const bannerTitle = el("div", "font-semibold mb-1");
    bannerTitle.style.color = "#c23520";
    bannerTitle.textContent = "Cursor is not configured to use the gateway";
    body.append(bannerTitle);
    const desc = el("div", "text-sm mb-3");
    desc.style.color = "#5a5a5a";
    desc.textContent = alignment.issues.join(" ") || "Click Fix to apply the gateway settings to Cursor.";
    const fixBtn = el("button", "btn-primary text-sm", ["Fix Cursor Settings"]);
    fixBtn.addEventListener("click", async () => {
      try {
        const result = await api.applyCursorSettings();
        toast(result.message, result.alignment.aligned ? "success" : "error");
        await onRefresh();
      } catch (e) { toast(String(e), "error"); }
    });
    body.append(desc, fixBtn);
    banner.append(icon, body);
    main.append(banner);
  } else if (state.status.publicRootUrl) {
  const syncBanner = el("div", "px-4 py-2.5 rounded-feex-sm text-sm flex items-center gap-2");
  syncBanner.style.cssText = "background:#e8f5e9;border:1px solid #c8e6c9;color:#2e7d32";
  const syncCheck = el("span", "", ["✓"]);
  const syncMsg = el("span", "");
  syncMsg.style.color = "#5a5a5a";
  syncMsg.textContent = alignment?.aligned
    ? "Cursor is synced with the gateway. Tunnel URL changes are applied automatically."
    : "Public tunnel active. Cursor settings sync when ready.";
  syncBanner.append(syncCheck, syncMsg);
  main.append(syncBanner);
  }

  // Vision note (subtle)
  const vNote = el("div", "px-4 py-2.5 rounded-feex-sm text-xs flex items-center gap-2");
  vNote.style.cssText = "background:#faf7f7;border:1px solid #ece6e6;color:#8a8a8a";
  const vIcon = el("span", "");
  vIcon.textContent = "ℹ";
  const vText = el("span", "");
  vText.textContent = "Cursor validates image/vision requests against OpenAI directly (known Cursor limitation). Text chat works perfectly.";
  vNote.append(vIcon, vText);
  main.append(vNote);

  // ── Two-column grid: Status | Cursor Settings ────────────────────────────────
  const grid = el("div", "grid grid-cols-1 lg:grid-cols-2 gap-5");

  // — Gateway status card —
  const statusCard = el("div", "glass-card p-5");
  statusCard.append(sectionTitle("Gateway Status"));

  const checks: [string, boolean][] = [
    ["Local server",      state.status.localServer],
    ["Public tunnel",     state.status.tunnel],
    ["Moonshot reachable",state.status.moonshotReachable],
    ["Cursor-ready",      state.status.cursorReady],
  ];
  const checkList = el("div", "space-y-2.5 mb-5");
  checks.forEach(([label, ok]) => {
    const row = el("div", "flex items-center justify-between py-1");
    const lbl = el("span", "text-sm");
    lbl.style.color = "#5a5a5a";
    lbl.textContent = label;
    row.append(lbl, iconStatus(ok, !ok && state.status.running));
    checkList.append(row);
  });
  statusCard.append(checkList);

  const ctrlRow = el("div", "flex flex-wrap gap-2");
  const startBtn  = el("button", "btn-primary text-sm",    ["▶  Start"]);
  const stopBtn   = el("button", "btn-secondary text-sm",  ["⏹  Stop"]);
  const restartBtn= el("button", "btn-secondary text-sm",  ["↺  Restart"]);

  startBtn.addEventListener("click", async () => {
    try { await api.startGateway(); toast("Gateway started", "success"); await onRefresh(); }
    catch (e) { toast(String(e), "error"); }
  });
  stopBtn.addEventListener("click", async () => {
    await api.stopGateway(); toast("Gateway stopped", "info"); await onRefresh();
  });
  restartBtn.addEventListener("click", async () => {
    try { await api.restartGateway(); toast("Gateway restarted", "success"); await onRefresh(); }
    catch (e) { toast(String(e), "error"); }
  });
  ctrlRow.append(startBtn, stopBtn, restartBtn);
  statusCard.append(ctrlRow);
  grid.append(statusCard);

  // — Cursor connection card —
  const cursorCard = el("div", "glass-card p-5");
  cursorCard.append(sectionTitle("Connection Details"));

  const baseUrl = state.status.publicBaseUrl ?? "Start gateway to get URL";
  const settingsBlock = el("div", "space-y-2.5 mb-4");
  settingsBlock.append(
    settingCard("OpenAI API Key", state.status.gatewayKey, () => {
      copyText(state.status.gatewayKey, document.activeElement as HTMLButtonElement);
    }),
    settingCard("Base URL", baseUrl, state.status.publicBaseUrl
      ? () => copyText(state.status.publicBaseUrl!, document.activeElement as HTMLButtonElement)
      : undefined),
    settingCard("Model", state.status.aliasModel, () =>
      copyText(state.status.aliasModel, document.activeElement as HTMLButtonElement)),
  );
  cursorCard.append(settingsBlock);

  // Cursor alignment status row
  if (state.status.cursorAlignment) {
    const s = state.status.cursorAlignment;
    const syncRow = el("div", "px-3 py-2.5 rounded-feex-sm mb-4 text-xs flex items-center gap-3");
    syncRow.style.cssText = s.aligned
      ? "background:#e8f5e9;border:1px solid #c8e6c9"
      : "background:#fdecea;border:1px solid #f5c6bf";
    const badge = el("span", "font-semibold shrink-0");
    badge.style.color = s.aligned ? "#2e7d32" : "#c23520";
    badge.textContent = s.aligned ? "✓ Cursor synced" : "✕ Cursor out of sync";
    const detail = el("span", "");
    detail.style.color = "#8a8a8a";
    detail.textContent = `key: ${s.keyMatches ? "✓" : "✕"}  url: ${s.baseUrlMatches ? "✓" : "✕"}  openai: ${s.useOpenaiKey ? "✓" : "✕"}`;
    syncRow.append(badge, detail);
    cursorCard.append(syncRow);
  }

  const applyBtn = el("button", "btn-primary w-full mb-2 text-sm", ["⚡  Apply to Cursor Automatically"]);
  applyBtn.addEventListener("click", async () => {
    applyBtn.setAttribute("disabled", "true");
    applyBtn.textContent = "Applying…";
    try { const r = await api.applyCursorSettings(); toast(r.message, "success"); }
    catch (e) { toast(String(e), "error"); }
    finally { applyBtn.removeAttribute("disabled"); applyBtn.textContent = "⚡  Apply to Cursor Automatically"; }
  });
  cursorCard.append(applyBtn);

  const copyAllBtn = el("button", "btn-secondary w-full text-sm", ["Copy All Settings"]);
  copyAllBtn.addEventListener("click", async () => {
    await navigator.clipboard.writeText([
      `OpenAI API Key: ${state.status.gatewayKey}`,
      `Base URL: ${state.status.publicBaseUrl ?? ""}`,
      `Model: ${state.status.aliasModel}`,
      `Override OpenAI Base URL: ON`,
    ].join("\n"));
    toast("Copied!", "success");
  });
  cursorCard.append(copyAllBtn);
  grid.append(cursorCard);
  main.append(grid);

  // ── Analytics (full-width) ───────────────────────────────────────────────────
  main.append(renderAnalytics(state.usage, state.status));

  // ── Logs ────────────────────────────────────────────────────────────────────
  const logsCard = el("div", "glass-card p-5");
  logsCard.append(sectionTitle("Activity Logs"));
  const logBox = el("div", "h-44 overflow-y-auto font-mono text-xs rounded-feex-sm p-3 space-y-0.5");
  logBox.style.cssText = "background:#faf7f7;border:1px solid #ece6e6;color:#5a5a5a";
  if (state.logs.length === 0) {
    const empty = el("div", "flex items-center justify-center h-full text-center");
    empty.style.color = "#b3a9a7";
    empty.textContent = "No activity yet — start the gateway to see logs.";
    logBox.append(empty);
  } else {
    state.logs.slice(-80).forEach(line => {
      const ln = el("div", "hover:text-feex-text-dark transition-colors py-px", [line]);
      logBox.append(ln);
    });
    requestAnimationFrame(() => { logBox.scrollTop = logBox.scrollHeight; });
  }
  logsCard.append(logBox);
  const logBtns = el("div", "flex gap-2 mt-3");
  const clrBtn = el("button", "btn-secondary text-xs", ["Clear"]);
  clrBtn.addEventListener("click", async () => { await api.clearLogs(); await onRefresh(); });
  const expBtn = el("button", "btn-secondary text-xs", ["Export Diagnostics"]);
  expBtn.addEventListener("click", async () => {
    try { const p = await api.exportDiagnostics(); toast(`Saved to ${p}`, "success"); }
    catch (e) { toast(String(e), "error"); }
  });
  logBtns.append(clrBtn, expBtn);
  logsCard.append(logBtns);
  main.append(logsCard);

  // ── Controls + Advanced ──────────────────────────────────────────────────────
  const row3 = el("div", "grid grid-cols-1 lg:grid-cols-2 gap-5");

  const controlsCard = el("div", "glass-card p-5");
  controlsCard.append(sectionTitle("Controls"));
  const ctrlGrid = el("div", "flex flex-wrap gap-2");

  const rotateKey = el("button", "btn-secondary text-sm", ["Rotate API Key"]);
  rotateKey.addEventListener("click", async () => {
    const key = await api.rotateGatewayKey();
    toast("Key rotated — update Cursor", "info");
    await onRefresh();
    await navigator.clipboard.writeText(key);
  });

  const openDash = el("button", "btn-secondary text-sm", ["Open Dashboard URL"]);
  openDash.addEventListener("click", async () => {
    const { open } = await import("@tauri-apps/plugin-shell");
    if (state.status.localBaseUrl) await open(state.status.localBaseUrl.replace("/v1", "/dashboard"));
  });

  const doctorBtn = el("button", "btn-secondary text-sm", ["Run Doctor"]);
  doctorBtn.addEventListener("click", async () => {
    const checks = await api.runDoctor();
    state.doctor = checks;
    renderDoctorSection(main, checks);
    toast("Doctor finished", "info");
  });

  ctrlGrid.append(rotateKey, openDash, doctorBtn);
  controlsCard.append(ctrlGrid);

  const autostartLabel = el("label", "flex items-center gap-3 mt-4 cursor-pointer select-none");
  const autostartCheck = el("input", "") as HTMLInputElement;
  autostartCheck.type = "checkbox";
  autostartCheck.checked = state.settings.autostartEnabled;
  autostartCheck.addEventListener("change", async () => {
    await api.setAutostart(autostartCheck.checked);
    toast(autostartCheck.checked ? "Autostart enabled" : "Autostart disabled", "info");
  });
  const autostartText = el("span", "text-sm");
  autostartText.style.color = "#5a5a5a";
  autostartText.textContent = "Start with Windows";
  autostartLabel.append(autostartCheck, autostartText);
  controlsCard.append(autostartLabel);
  row3.append(controlsCard);

  const advancedCard = el("div", "glass-card p-5");
  advancedCard.id = "advanced-settings";
  advancedCard.append(sectionTitle("Advanced Settings"));
  advancedCard.append(renderAdvancedForm(state.settings, onRefresh));
  row3.append(advancedCard);
  main.append(row3);

  if (state.doctor) renderDoctorSection(main, state.doctor);

  root.append(wrapper);
}

// ─── Advanced form ────────────────────────────────────────────────────────────
function renderAdvancedForm(settings: SettingsView, onRefresh: () => Promise<void>): HTMLElement {
  const form = el("div", "space-y-3.5 text-sm");

  const field = (label: string, input: HTMLElement): void => {
    const lbl = el("label", "block text-xs mb-1.5 font-medium");
    lbl.style.color = "#8a8a8a";
    lbl.textContent = label;
    const wrap = el("div", "");
    wrap.append(lbl, input);
    form.append(wrap);
  };

  const realModel = el("input", "input-field") as HTMLInputElement;
  realModel.value = settings.realModel;
  field("Real model (Moonshot API)", realModel);

  const aliasModel = el("select", "input-field") as HTMLSelectElement;
  [
    ["gpt-5-high-max", "GPT-5 High Max — recommended (256K context)"],
    ["gpt-5.5-high",   "GPT-5.5 High"],
    ["gpt-4-turbo",    "GPT-4 Turbo (legacy)"],
    ["gpt-4o",         "GPT-4o"],
    ["kimi-k2.6",      "kimi-k2.6 (direct)"],
  ].forEach(([v, lbl]) => {
    const opt = el("option", "", [lbl]) as HTMLOptionElement;
    opt.value = v;
    if (v === settings.aliasModel) opt.selected = true;
    aliasModel.append(opt);
  });
  field("Cursor model name (alias)", aliasModel);

  const maxTokens = el("select", "input-field") as HTMLSelectElement;
  ([ [8192,"8K (fast)"], [32768,"32K (default)"], [65536,"64K"], [131072,"128K"], [262144,"256K (max)"] ] as const).forEach(([v, lbl]) => {
    const opt = el("option", "", [lbl]) as HTMLOptionElement;
    opt.value = String(v);
    if (Number(v) === settings.maxTokensDefault) opt.selected = true;
    maxTokens.append(opt);
  });
  field("Max output tokens", maxTokens);

  const port = el("input", "input-field") as HTMLInputElement;
  port.type = "number";
  port.value = String(settings.localPort);
  field("Local port", port);

  const togglesWrap = el("div", "space-y-1.5 pt-1");
  [
    ["thinkingDisabled",       "Disable Kimi reasoning (NOT recommended)", settings.thinkingDisabled],
    ["forceNonStreaming",       "Buffer responses (non-streaming)",          settings.forceNonStreaming],
    ["sanitizeTools",          "Sanitize tool schemas (recommended)",        settings.sanitizeTools],
    ["injectReasoningPlaceholder","Inject reasoning placeholder (required)", settings.injectReasoningPlaceholder],
    ["autoStartGateway",       "Auto-start gateway on launch",               settings.autoStartGateway],
  ].forEach(([key, lbl, checked]) => {
    const label = el("label", "flex items-center gap-2.5 cursor-pointer select-none");
    const cb = el("input", "") as HTMLInputElement;
    cb.type = "checkbox"; cb.checked = checked as boolean; cb.dataset.key = key as string;
    const txt = el("span", "text-xs");
    txt.style.color = "#5a5a5a";
    txt.textContent = lbl as string;
    label.append(cb, txt);
    togglesWrap.append(label);
  });
  form.append(togglesWrap);

  const saveBtn = el("button", "btn-primary w-full mt-3 text-sm", ["Save Settings"]);
  saveBtn.addEventListener("click", async () => {
    const getToggle = (k: string): boolean =>
      (togglesWrap.querySelector(`[data-key="${k}"]`) as HTMLInputElement)?.checked ?? false;
    await api.updateSettings({
      gatewayKey: settings.gatewayKey,
      localPort: Number(port.value) || 4001,
      realModel: realModel.value || "kimi-k2.6",
      aliasModel: aliasModel.value || "gpt-5-high-max",
      maxTokensDefault: Number(maxTokens.value) || 32768,
      forceNonStreaming: getToggle("forceNonStreaming"),
      thinkingDisabled:  getToggle("thinkingDisabled"),
      sanitizeTools:     getToggle("sanitizeTools"),
      injectReasoningPlaceholder: getToggle("injectReasoningPlaceholder"),
      autoStartGateway:  getToggle("autoStartGateway"),
      autostartEnabled:  settings.autostartEnabled,
      wizardCompleted:   settings.wizardCompleted,
    });
    toast("Settings saved", "success");
    await onRefresh();
  });
  form.append(saveBtn);
  return form;
}

// ─── Doctor section ───────────────────────────────────────────────────────────
function renderDoctorSection(main: HTMLElement, checks: DoctorCheck[]): void {
  document.getElementById("doctor-section")?.remove();
  const section = el("div", "glass-card p-5");
  section.id = "doctor-section";
  section.append(sectionTitle("Doctor Report"));

  const list = el("div", "space-y-2");
  checks.forEach(c => {
    const row = el("div", "flex items-start gap-3 px-3.5 py-3 rounded-feex-sm");
    row.style.cssText = "background:#faf7f7;border:1px solid #ece6e6";
    const dotColor = c.status === "pass" ? "#4caf50" : c.status === "warn" ? "#e8b87a" : "#df3e2b";
    const dot = el("span", "status-dot mt-1 shrink-0");
    dot.style.cssText = `background:${dotColor};box-shadow:0 0 8px ${dotColor}55;flex-shrink:0`;
    const text = el("div", "flex-1 min-w-0");
    const lbl = el("div", "text-sm font-medium mb-0.5");
    lbl.style.color = "#5a4a48";
    lbl.textContent = c.label;
    const detail = el("div", "text-xs");
    detail.style.color = "#8a8a8a";
    detail.textContent = c.detail;
    text.append(lbl, detail);
    row.append(dot, text);
    list.append(row);
  });
  section.append(list);
  main.append(section);
}

// suppress unused helper warning
void muted; void divider;
