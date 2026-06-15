import { open } from "@tauri-apps/plugin-shell";
import * as api from "../api";
import type { GatewayStatus, SettingsView } from "../types";
import { copyText, el, settingCard, toast } from "./components";

const KIMI_API_URL = "https://platform.moonshot.cn/console/api-keys";

export interface WizardCallbacks {
  onComplete: () => void;
}

const WIZARD_STEP_KEY = "kimi-wizard-step";

function resumeWizardStep(settings: SettingsView, status: GatewayStatus): number {
  const saved = sessionStorage.getItem(WIZARD_STEP_KEY);
  const n = saved ? Number.parseInt(saved, 10) : 0;
  if (n >= 2 && n <= 4) return n;
  if (status.publicBaseUrl && status.running && settings.moonshotKeyMasked) return 4;
  if (settings.moonshotKeyMasked) return 3;
  return 1;
}
const persist = (s: number) => sessionStorage.setItem(WIZARD_STEP_KEY, String(s));
const clearStep = () => sessionStorage.removeItem(WIZARD_STEP_KEY);

export function renderWizard(
  root: HTMLElement,
  settings: SettingsView,
  gatewayStatus: GatewayStatus,
  callbacks: WizardCallbacks,
): void {
  let step = resumeWizardStep(settings, gatewayStatus);
  let moonshotKey = "";
  let status: GatewayStatus | null = gatewayStatus;

  // ── Outer shell - full-screen warm light with ambient glow ────────────────
  const shell = el("div", "min-h-screen flex flex-col items-center justify-center px-4 py-12");
  shell.style.cssText = `
    background: radial-gradient(ellipse 90% 60% at 50% -5%, rgba(248,209,167,0.45) 0%, transparent 55%),
                radial-gradient(ellipse 70% 50% at 80% 90%, rgba(223,62,43,0.06) 0%, transparent 50%),
                #f5f5f5;
  `;
  root.replaceChildren(shell);

  function render(): void {
    shell.replaceChildren();

    // ── Branding ──────────────────────────────────────────────────────────────
    const brand = el("div", "flex flex-col items-center gap-2 mb-8 animate-fade_in");
    const logoIcon = el("div", "w-12 h-12 rounded-xl flex items-center justify-center text-2xl font-bold text-white mb-1");
    logoIcon.style.cssText = "background:linear-gradient(135deg,#df3e2b,#e8b87a);box-shadow:0 4px 24px rgba(223,62,43,0.4)";
    logoIcon.textContent = "K";
    brand.append(logoIcon);
    brand.append(el("div", "text-xl font-bold gradient-text", ["Kimi Cursor Gateway"]));
    const sub = el("div", "text-sm");
    sub.style.color = "#8a8a8a";
    sub.textContent = "Connect Kimi to Cursor in 3 minutes";
    brand.append(sub);
    shell.append(brand);

    // ── Step pills ────────────────────────────────────────────────────────────
    const stepPills = el("div", "flex items-center gap-2 mb-6 animate-fade_in");
    const stepLabels = ["Welcome", "API Key", "Start", "Configure"];
    stepLabels.forEach((label, i) => {
      const num = i + 1;
      const active = num === step;
      const done = num < step;

      const pill = el("div", "flex items-center gap-1.5 text-xs font-medium px-2.5 py-1 rounded-full transition-all");
      if (active) {
        pill.style.cssText = "background:#fdecea;border:1px solid #f5c6bf;color:#c23520";
      } else if (done) {
        pill.style.cssText = "background:#e8f5e9;border:1px solid #c8e6c9;color:#2e7d32";
        pill.textContent = `✓ ${label}`;
        if (i < stepLabels.length - 1) {
          stepPills.append(pill);
          const sep = el("div", "w-4 h-px");
          sep.style.background = "#ddd2d2";
          stepPills.append(sep);
        } else stepPills.append(pill);
        return;
      } else {
        pill.style.cssText = "border:1px solid #ece6e6;color:#b3a9a7;background:#ffffff";
      }
      const numEl = el("span", "w-4 h-4 rounded-full flex items-center justify-center text-xs font-bold shrink-0");
      numEl.style.cssText = active
        ? "background:#df3e2b;color:#fff"
        : "background:#f0eaea;color:#8a8a8a";
      numEl.textContent = String(num);
      pill.append(numEl, document.createTextNode(label));
      stepPills.append(pill);
      if (i < stepLabels.length - 1) {
        const sep = el("div", "w-4 h-px");
        sep.style.background = "#ddd2d2";
        stepPills.append(sep);
      }
    });
    shell.append(stepPills);

    // ── Card ──────────────────────────────────────────────────────────────────
    const card = el("div", "w-full max-w-md animate-slide_up");
    card.style.cssText = "background:#ffffff;border:1px solid #ece6e6;border-radius:20px;padding:32px;box-shadow:0 24px 64px rgba(0,0,0,0.12)";
    shell.append(card);

    if (step === 1) renderWelcome(card);
    else if (step === 2) renderApiKey(card);
    else if (step === 3) renderStart(card);
    else renderCursor(card);
  }

  // ── Step helpers ─────────────────────────────────────────────────────────────
  function heading(text: string): HTMLElement {
    const h = el("h2", "text-xl font-bold mb-1");
    h.style.color = "#5a4a48";
    h.textContent = text;
    return h;
  }
  function subheading(text: string): HTMLElement {
    const p = el("p", "text-sm mb-5 leading-relaxed");
    p.style.color = "#8a8a8a";
    p.textContent = text;
    return p;
  }

  // ── Step 1: Welcome ───────────────────────────────────────────────────────────
  function renderWelcome(body: HTMLElement): void {
    body.append(heading("Welcome"), subheading("A secure bridge so Cursor can use Kimi's AI. No terminals or manual setup needed."));

    const items = [
      ["🔑", "Paste your Kimi API key"],
      ["⚡", "Start the local gateway + tunnel"],
      ["🎯", "Use Kimi in Cursor instantly"],
    ];
    const list = el("div", "space-y-2.5 mb-6");
    items.forEach(([icon, text]) => {
      const row = el("div", "flex items-center gap-3 px-3.5 py-2.5 rounded-feex-sm");
      row.style.cssText = "background:#faf7f7;border:1px solid #ece6e6";
      const ic = el("span", "text-lg shrink-0", [icon]);
      const tx = el("span", "text-sm");
      tx.style.color = "#5a5a5a";
      tx.textContent = text;
      row.append(ic, tx);
      list.append(row);
    });
    body.append(list);

    const btn = el("button", "btn-primary w-full", ["Get Started →"]);
    btn.addEventListener("click", () => { step = 2; persist(step); render(); });
    body.append(btn);
  }

  // ── Step 2: API Key ───────────────────────────────────────────────────────────
  function renderApiKey(body: HTMLElement): void {
    body.append(heading("Moonshot API Key"));

    const desc = el("div", "text-sm mb-4 leading-relaxed");
    desc.style.color = "#8a8a8a";
    const t = document.createTextNode("Get your key from the ");
    const link = el("a", "underline underline-offset-2 cursor-pointer font-medium transition-opacity hover:opacity-70");
    link.style.color = "#df3e2b";
    link.textContent = "Kimi Open Platform →";
    link.addEventListener("click", (e) => { e.preventDefault(); open(KIMI_API_URL).catch(() => window.open(KIMI_API_URL, "_blank")); });
    desc.append(t, link);
    body.append(desc);

    const hint = el("div", "text-xs mb-4 px-3 py-2 rounded-feex-sm");
    hint.style.cssText = "background:#fdf3e9;border:1px solid #f3ddc7;color:#8a7a5a";
    hint.textContent = 'Your key starts with "sk-" - stored encrypted on this device only.';
    body.append(hint);

    const input = el("input", "input-field mb-3") as HTMLInputElement;
    input.type = "password"; input.placeholder = "sk-···"; input.value = moonshotKey;
    body.append(input);

    const result = el("div", "text-sm mb-4 px-3 py-2 rounded-feex-sm hidden");
    body.append(result);

    const row = el("div", "flex gap-2.5");
    const testBtn = el("button", "btn-secondary flex-1 text-sm", ["Test Key"]);
    const nextBtn = el("button", "btn-primary flex-1 text-sm", ["Continue →"]);
    nextBtn.disabled = true;

    if (settings.moonshotKeyMasked) {
      nextBtn.disabled = false;
      result.className = "text-sm mb-4 px-3 py-2 rounded-feex-sm";
      result.style.cssText = "background:#e8f5e9;border:1px solid #c8e6c9;color:#2e7d32";
      result.textContent = "✓ Moonshot API key already saved.";
    }

    testBtn.addEventListener("click", async () => {
      moonshotKey = input.value.trim();
      if (!moonshotKey) { toast("Paste your API key first.", "error"); return; }
      testBtn.textContent = "Testing…";
      testBtn.setAttribute("disabled", "true");
      try {
        const msg = await api.testMoonshotKey(moonshotKey);
        await api.saveMoonshotKey(moonshotKey);
        result.className = "text-sm mb-4 px-3 py-2 rounded-feex-sm";
        result.style.cssText = "background:#e8f5e9;border:1px solid #c8e6c9;color:#2e7d32";
        result.textContent = `✓ ${msg}`;
        nextBtn.disabled = false;
        toast("API key verified!", "success");
      } catch (e) {
        result.className = "text-sm mb-4 px-3 py-2 rounded-feex-sm";
        result.style.cssText = "background:#fdecea;border:1px solid #f5c6bf;color:#c23520";
        result.textContent = `✕ ${String(e)}`;
        toast(String(e), "error");
      }
      testBtn.textContent = "Test Key";
      testBtn.removeAttribute("disabled");
    });

    nextBtn.addEventListener("click", () => { step = 3; persist(step); render(); });
    row.append(testBtn, nextBtn);
    body.append(row);

    const back = el("button", "btn-secondary w-full mt-2.5 text-sm", ["← Back"]);
    back.addEventListener("click", () => { step = 1; persist(step); render(); });
    body.append(back);
  }

  // ── Step 3: Start ──────────────────────────────────────────────────────────────
  function renderStart(body: HTMLElement): void {
    body.append(heading("Start Gateway"), subheading("Launching the local server and creating a secure public tunnel for Cursor."));

    const steps = [
      "Local server starting",
      "Cloudflare tunnel connecting",
      "Kimi API verified",
      "Ready for Cursor",
    ];
    const stepEls: HTMLElement[] = [];
    const stepsWrap = el("div", "space-y-2 mb-5");
    steps.forEach(label => {
      const row = el("div", "flex items-center gap-3 px-3.5 py-2.5 rounded-feex-sm");
      row.style.cssText = "background:#faf7f7;border:1px solid #ece6e6";
      const dot = el("span", "status-dot shrink-0");
      dot.style.cssText = "background:#ddd2d2";
      const lbl = el("span", "text-sm");
      lbl.style.color = "#8a8a8a";
      lbl.textContent = label;
      row.append(dot, lbl);
      stepsWrap.append(row);
      stepEls.push(row);
    });
    body.append(stepsWrap);

    const warn = el("div", "text-xs px-3 py-2.5 rounded-feex-sm mb-4");
    warn.style.cssText = "background:#fdf3e9;border:1px solid #f3ddc7;color:#8a7a5a";
    warn.textContent = "⚡ A public HTTPS tunnel is required because Cursor blocks localhost. The URL may change after restart - the app auto-syncs it.";

    const markStep = (idx: number, state: "active" | "done" | "error"): void => {
      const row = stepEls[idx];
      const dot = row.querySelector(".status-dot") as HTMLElement;
      const lbl = row.querySelector("span:last-child") as HTMLElement;
      if (state === "done") {
        dot.style.cssText = "background:#4caf50;box-shadow:0 0 8px rgba(76,175,80,0.5)";
        lbl.style.color = "#5a4a48";
        row.style.borderColor = "#c8e6c9";
      } else if (state === "active") {
        dot.style.cssText = "background:#df3e2b;box-shadow:0 0 10px rgba(223,62,43,0.5)";
        dot.classList.add("pulsing");
        lbl.style.color = "#c23520";
        row.style.borderColor = "#f5c6bf";
      } else {
        dot.style.cssText = "background:#df3e2b;box-shadow:0 0 8px rgba(223,62,43,0.4)";
        lbl.style.color = "#c23520";
        row.style.borderColor = "#f5c6bf";
      }
    };

    const advance = (): void => { toast("Gateway is ready!", "success"); step = 4; persist(step); render(); };

    if (status?.running && status.publicBaseUrl) {
      stepEls.forEach((_, i) => markStep(i, "done"));
      const btn = el("button", "btn-primary w-full", ["Continue to Cursor Settings →"]);
      btn.addEventListener("click", () => advance());
      body.append(btn, warn);
      return;
    }

    if (status?.running) {
      markStep(0, "done"); markStep(1, "active");
      void (async () => {
        for (let i = 0; i < 60; i++) {
          status = await api.getGatewayStatus();
          if (status.publicBaseUrl) { markStep(1, "done"); markStep(2, "done"); markStep(3, "done"); advance(); return; }
          await new Promise(r => setTimeout(r, 1000));
        }
      })();
    }

    const startBtn = el("button", "btn-primary w-full", ["Start Gateway"]);
    let started = false;

    startBtn.addEventListener("click", async () => {
      if (started) return;
      started = true;
      startBtn.textContent = "Starting…";
      startBtn.setAttribute("disabled", "true");
      try {
        markStep(0, "active");
        await new Promise(r => setTimeout(r, 300));
        status = await api.startGateway();
        markStep(0, "done"); markStep(1, "active");
        for (let i = 0; i < 60; i++) {
          status = await api.getGatewayStatus();
          if (status.publicBaseUrl) break;
          await new Promise(r => setTimeout(r, 1000));
        }
        if (!status?.publicBaseUrl) throw new Error("Tunnel timed out. Check your internet connection.");
        markStep(1, "done"); markStep(2, "active");
        await api.testMoonshotKey();
        markStep(2, "done"); markStep(3, "done");
        advance();
      } catch (e) {
        toast(String(e), "error");
        markStep(0, "error");
        startBtn.textContent = "Retry";
        startBtn.removeAttribute("disabled");
        started = false;
      }
    });

    body.append(startBtn, warn);
  }

  // ── Step 4: Cursor settings ────────────────────────────────────────────────────
  function renderCursor(body: HTMLElement): void {
    body.append(heading("Cursor Settings"), subheading("In Cursor → Settings → Models, enable Override OpenAI Base URL and paste these values:"));

    const key   = status?.gatewayKey ?? settings.gatewayKey;
    const model = status?.aliasModel ?? settings.aliasModel;

    const refresh = async (): Promise<void> => { status = await api.getGatewayStatus(); render(); };

    if (!status?.publicBaseUrl) {
      const wait = el("div", "text-sm mb-4 px-3 py-2.5 rounded-feex-sm flex items-center gap-2");
      wait.style.cssText = "background:#fdf3e9;border:1px solid #f3ddc7;color:#8a7a5a";
      const spinner = el("span", "animate-spin-slow", ["⟳"]);
      wait.append(spinner, document.createTextNode(" Waiting for tunnel URL…"));
      body.append(wait);
      setTimeout(refresh, 2000);
    }

    const settingsBlock = el("div", "space-y-2.5 mb-5");
    settingsBlock.append(
      settingCard("OpenAI API Key", key, () => copyText(key, event?.target as HTMLButtonElement)),
      settingCard("Override OpenAI Base URL", status?.publicBaseUrl ?? "Starting tunnel…",
        status?.publicBaseUrl ? () => copyText(status!.publicBaseUrl!, event?.target as HTMLButtonElement) : undefined),
      settingCard("Model name", model, () => copyText(model, event?.target as HTMLButtonElement)),
    );
    body.append(settingsBlock);

    const note = el("div", "text-xs mb-4 px-3 py-2 rounded-feex-sm");
    note.style.cssText = "background:#faf7f7;border:1px solid #ece6e6;color:#8a8a8a";
    note.textContent = "Remember to turn on Override OpenAI Base URL in Cursor. URL must end with /v1.";
    body.append(note);

    const copyAllBtn = el("button", "btn-secondary w-full mb-3 text-sm", ["Copy All Settings"]);
    copyAllBtn.addEventListener("click", async () => {
      await navigator.clipboard.writeText([
        `OpenAI API Key: ${key}`,
        `Base URL: ${status?.publicBaseUrl ?? ""}`,
        `Model: ${model}`,
        `Override OpenAI Base URL: ON`,
      ].join("\n"));
      toast("All settings copied!", "success");
    });
    body.append(copyAllBtn);

    const autostartLabel = el("label", "flex items-start gap-3 mb-4 p-3.5 rounded-feex-sm cursor-pointer select-none");
    autostartLabel.style.cssText = "background:#faf7f7;border:1px solid #ece6e6";
    const autostartCheck = el("input", "mt-0.5 shrink-0") as HTMLInputElement;
    autostartCheck.type = "checkbox"; autostartCheck.checked = true;
    const autostartText = el("div", "flex flex-col gap-0.5");
    const atLabel = el("div", "text-sm font-medium");
    atLabel.style.color = "#5a4a48";
    atLabel.textContent = "Start with system login";
    const atSub = el("div", "text-xs");
    atSub.style.color = "#8a8a8a";
    atSub.textContent = "Recommended - gateway starts automatically in the tray after login.";
    autostartText.append(atLabel, atSub);
    autostartLabel.append(autostartCheck, autostartText);
    body.append(autostartLabel);

    const applyBtn = el("button", "btn-primary w-full mb-2.5 text-sm", ["⚡ Apply to Cursor Automatically"]);
    applyBtn.addEventListener("click", async () => {
      applyBtn.setAttribute("disabled", "true"); applyBtn.textContent = "Applying…";
      try { const r = await api.applyCursorSettings(); toast(r.message, "success"); }
      catch (e) { toast(String(e), "error"); }
      finally { applyBtn.removeAttribute("disabled"); applyBtn.textContent = "⚡ Apply to Cursor Automatically"; }
    });
    body.append(applyBtn);

    const finish = el("button", "btn-secondary w-full text-sm", ["Finish & Open Dashboard"]);
    finish.addEventListener("click", async () => {
      finish.setAttribute("disabled", "true");
      try {
        try { await api.applyCursorSettings(); } catch { /* ok */ }
        await api.completeWizard(autostartCheck.checked);
        clearStep();
        toast("Setup complete - welcome to the dashboard!", "success");
        callbacks.onComplete();
      } catch (e) { toast(String(e), "error"); finish.removeAttribute("disabled"); }
    });
    body.append(finish);
  }

  render();
}
