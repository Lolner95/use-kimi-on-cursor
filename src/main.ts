import "./styles.css";
import * as api from "./api";
import { waitForTauri } from "./tauriReady";
import type { DashboardState } from "./ui/dashboard";
import { renderDashboard } from "./ui/dashboard";
import { renderWizard } from "./ui/wizard";
import { el, toast } from "./ui/components";

const appRoot = document.getElementById("app");
if (!appRoot) {
  throw new Error("Missing #app root");
}
const appElement: HTMLElement = appRoot;

const toasts = el("div", "fixed bottom-4 right-4 z-50 space-y-2 max-w-sm");
toasts.id = "toasts";
document.body.append(toasts);

async function loadState(): Promise<DashboardState> {
  const [settings, status, logs, usage] = await Promise.all([
    api.getSettings(),
    api.getGatewayStatus(),
    api.getLogs(),
    api.getTokenUsage().catch(() => null),
  ]);
  return { settings, status, logs, usage, doctor: null };
}

async function refresh(): Promise<void> {
  const state = await loadState();
  const root = appElement;
  if (!state.settings.wizardCompleted) {
    renderWizard(root, state.settings, state.status, {
      onComplete: () => void refresh(),
    });
  } else {
    renderDashboard(root, state, refresh);
  }
}

async function init(): Promise<void> {
  await waitForTauri();

  await api.onTunnelUrlChanged(() => {
    toast("Tunnel URL changed — Cursor settings auto-synced. Restart Cursor if it was open.", "info");
    void refresh();
  });

  await api.onGatewayStatus(() => {
    void loadState().then((state) => {
      // Re-rendering the wizard on every gateway event resets the user to step 1.
      if (state.settings.wizardCompleted) {
        void refresh();
      }
    });
  });

  await api.onNavigate((target) => {
    if (target === "settings") {
      // Ensure the dashboard is shown first, then scroll to the settings section.
      void loadState().then((state) => {
        if (state.settings.wizardCompleted) {
          // If already on dashboard, just scroll.
          const el = document.getElementById("advanced-settings");
          if (el) {
            el.scrollIntoView({ behavior: "smooth", block: "start" });
          } else {
            // Dashboard not rendered yet — refresh first, then scroll.
            void refresh().then(() => {
              document.getElementById("advanced-settings")?.scrollIntoView({ behavior: "smooth", block: "start" });
            });
          }
        }
      });
    }
  });

  setInterval(() => {
    void loadState().then((state) => {
      if (state.settings.wizardCompleted) {
        const badge = document.querySelector("header .status-dot");
        if (badge) {
          badge.className = `status-dot ${state.status.running ? "bg-emerald-400 shadow-[0_0_8px_rgba(52,211,153,0.6)]" : "bg-red-400"}`;
        }
      }
    });
  }, 3000);

  await refresh();
}

init().catch((e) => {
  appElement.replaceChildren(
    el("div", "p-8 text-red-400", [`Failed to start: ${String(e)}`]),
  );
});
