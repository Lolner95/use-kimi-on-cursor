export interface GatewayStatus {
  running: boolean;
  localServer: boolean;
  tunnel: boolean;
  moonshotReachable: boolean;
  cursorReady: boolean;
  publicRootUrl: string | null;
  publicBaseUrl: string | null;
  localBaseUrl: string;
  gatewayKey: string;
  aliasModel: string;
  realModel: string;
  lastError: string | null;
  cursorAlignment: CursorAlignmentStatus | null;
}

export interface CursorAlignmentStatus {
  installed: boolean;
  dbPath: string;
  keyMatches: boolean;
  useOpenaiKey: boolean;
  baseUrlMatches: boolean;
  composerModelMatches: boolean;
  aligned: boolean;
  storedKeyPrefix: string | null;
  expectedKeyPrefix: string;
  storedBaseUrl: string | null;
  expectedBaseUrl: string;
  storedComposerModel: string | null;
  expectedModel: string;
  issues: string[];
}

export interface DailyTokenUsage {
  date: string;
  promptTokens: number;
  completionTokens: number;
  totalTokens: number;
  requestCount: number;
}

export interface TokenUsageEvent {
  id: string;
  timestamp: string;
  date: string;
  model: string;
  promptTokens: number;
  completionTokens: number;
  totalTokens: number;
  requestId: string;
  latencyMs: number;
}

export interface UsageStatsSnapshot {
  today: DailyTokenUsage;
  last7Days: DailyTokenUsage[];
  last30Days: DailyTokenUsage[];
  lifetime: DailyTokenUsage;
  recentEvents: TokenUsageEvent[];
}

export interface SettingsView {
  moonshotKeyMasked: string | null;
  gatewayKey: string;
  localPort: number;
  realModel: string;
  aliasModel: string;
  forceNonStreaming: boolean;
  thinkingDisabled: boolean;
  sanitizeTools: boolean;
  maxTokensDefault: number;
  injectReasoningPlaceholder: boolean;
  autostartEnabled: boolean;
  autoStartGateway: boolean;
  wizardCompleted: boolean;
  logsDir: string;
}

export type DoctorStatus = "pass" | "warn" | "fail";

export interface DoctorCheck {
  id: string;
  label: string;
  status: DoctorStatus;
  detail: string;
  repairable: boolean;
}

export interface AppSettingsUpdate {
  localPort: number;
  realModel: string;
  aliasModel: string;
  forceNonStreaming: boolean;
  thinkingDisabled: boolean;
  sanitizeTools: boolean;
  maxTokensDefault: number;
  injectReasoningPlaceholder: boolean;
  autoStartGateway: boolean;
}
