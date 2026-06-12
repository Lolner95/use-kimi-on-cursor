interface TauriInternals {
  transformCallback: (callback: (...args: unknown[]) => void, once?: boolean) => number;
}

declare global {
  interface Window {
    __TAURI_INTERNALS__?: TauriInternals;
  }
}

export function isTauriReady(): boolean {
  return typeof window !== "undefined" && !!window.__TAURI_INTERNALS__?.transformCallback;
}

export async function waitForTauri(timeoutMs = 8000): Promise<void> {
  const start = Date.now();
  while (Date.now() - start < timeoutMs) {
    if (isTauriReady()) return;
    await new Promise((resolve) => setTimeout(resolve, 25));
  }
  throw new Error(
    "Tauri runtime is not available. Close any browser tab at localhost:1420 and use the Kimi Cursor Gateway app window from npm run tauri:dev.",
  );
}
