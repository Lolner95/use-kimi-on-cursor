import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type {
  AppSettingsUpdate,
  CursorAlignmentStatus,
  DoctorCheck,
  GatewayStatus,
  SettingsView,
  TokenUsageEvent,
  UsageStatsSnapshot,
} from "./types";

export async function getSettings(): Promise<SettingsView> {
  return invoke<SettingsView>("get_settings");
}

export async function saveMoonshotKey(key: string): Promise<void> {
  return invoke("save_moonshot_key", { key });
}

export async function testMoonshotKey(key?: string): Promise<string> {
  return invoke<string>("test_moonshot_key", { key: key ?? null });
}

export async function startGateway(): Promise<GatewayStatus> {
  return invoke<GatewayStatus>("start_gateway");
}

export async function stopGateway(): Promise<GatewayStatus> {
  return invoke<GatewayStatus>("stop_gateway");
}

export async function restartGateway(): Promise<GatewayStatus> {
  return invoke<GatewayStatus>("restart_gateway");
}

export async function getGatewayStatus(): Promise<GatewayStatus> {
  return invoke<GatewayStatus>("get_gateway_status");
}

export async function rotateGatewayKey(): Promise<string> {
  return invoke<string>("rotate_gateway_key");
}

export async function completeWizard(
  enableAutostart = true,
): Promise<void> {
  return invoke("complete_wizard", { enableAutostart });
}

export async function inspectCursorInstall(): Promise<{
  dbPath: string;
  exePath: string | null;
  useOpenaiKeyBefore: boolean | null;
  openaiBaseUrlBefore: string | null;
}> {
  return invoke("inspect_cursor_install");
}

export async function applyCursorSettings(): Promise<{
  applied: boolean;
  dbPath: string;
  exePath: string | null;
  baseUrl: string;
  model: string;
  message: string;
  alignment: CursorAlignmentStatus;
}> {
  return invoke("apply_cursor_settings");
}

export async function getCursorAlignment(): Promise<CursorAlignmentStatus> {
  return invoke<CursorAlignmentStatus>("get_cursor_alignment");
}

export async function getTokenUsage(): Promise<UsageStatsSnapshot> {
  return invoke<UsageStatsSnapshot>("get_token_usage");
}

export async function getTokenUsageForDate(date: string): Promise<TokenUsageEvent[]> {
  return invoke<TokenUsageEvent[]>("get_token_usage_for_date", { date });
}

export async function getAppInfo(): Promise<{
  version: string;
  portableMode: boolean;
  dataDir: string;
  exeDir: string | null;
}> {
  return invoke("get_app_info");
}

export async function getLogs(): Promise<string[]> {
  return invoke<string[]>("get_logs");
}

export async function clearLogs(): Promise<void> {
  return invoke("clear_logs");
}

export async function runDoctor(): Promise<DoctorCheck[]> {
  return invoke<DoctorCheck[]>("run_doctor_checks");
}

export async function setAutostart(enabled: boolean): Promise<boolean> {
  return invoke<boolean>("set_autostart", { enabled });
}

export async function isAutostartEnabled(): Promise<boolean> {
  return invoke<boolean>("is_autostart_enabled");
}

export async function exportDiagnostics(): Promise<string> {
  return invoke<string>("export_diagnostics");
}

export async function updateSettings(
  settings: AppSettingsUpdate & {
    gatewayKey: string;
    autostartEnabled: boolean;
    wizardCompleted: boolean;
  },
): Promise<void> {
  return invoke("update_settings", {
    settings: {
      moonshotKeyEncrypted: null,
      gatewayKey: settings.gatewayKey,
      localPort: settings.localPort,
      realModel: settings.realModel,
      aliasModel: settings.aliasModel,
      forceNonStreaming: settings.forceNonStreaming,
      thinkingDisabled: settings.thinkingDisabled,
      sanitizeTools: settings.sanitizeTools,
      maxTokensDefault: settings.maxTokensDefault,
      injectReasoningPlaceholder: settings.injectReasoningPlaceholder,
      autostartEnabled: settings.autostartEnabled,
      autoStartGateway: settings.autoStartGateway,
      wizardCompleted: settings.wizardCompleted,
      startMinimized: true,
    },
  });
}

export function onTunnelUrlChanged(
  callback: (url: string) => void,
): Promise<() => void> {
  return listen<string>("tunnel-url-changed", (event) => {
    callback(event.payload);
  }).then((unlisten) => unlisten);
}

export function onGatewayStatus(
  callback: (status: GatewayStatus) => void,
): Promise<() => void> {
  return listen<GatewayStatus>("gateway-status", (event) => {
    callback(event.payload);
  }).then((unlisten) => unlisten);
}

export function onNavigate(
  callback: (target: string) => void,
): Promise<() => void> {
  return listen<string>("navigate", (event) => {
    callback(event.payload);
  }).then((unlisten) => unlisten);
}
