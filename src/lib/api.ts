import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { DownloadEvent, PlayerEvent, Settings, Snapshot } from './types';

export type PlayerEventListener = (event: PlayerEvent) => void;

let playerEventListeners: PlayerEventListener[] = [];

export function emitPlayerEventForTest(event: PlayerEvent) {
  for (const listener of playerEventListeners) {
    listener(event);
  }
}

export type DownloadEventListener = (event: DownloadEvent | { type: 'refresh' }) => void;

let downloadEventListeners: DownloadEventListener[] = [];

export function emitDownloadEventForTest(event: DownloadEvent | { type: 'refresh' }) {
  for (const listener of downloadEventListeners) {
    listener(event);
  }
}

declare global {
  interface Window {
    __emitDownloadEvent?: (event: DownloadEvent | { type: 'refresh' }) => void;
    __CLOUDSTREAM_TEST__?: (command: string, args: Record<string, unknown>) => Promise<unknown>;
    __emitPlayerEvent?: (event: PlayerEvent) => void;
  }
}

if (typeof window !== 'undefined') {
  window.__emitPlayerEvent = emitPlayerEventForTest;
  window.__emitDownloadEvent = emitDownloadEventForTest;
}

/** Live download queue changes. `refresh` means events were dropped and the list must be reloaded. */
export async function onDownloadEvent(callback: DownloadEventListener): Promise<() => void> {
  downloadEventListeners.push(callback);
  const unlistenLocal = () => {
    downloadEventListeners = downloadEventListeners.filter((l) => l !== callback);
  };
  if (!isTauri()) return unlistenLocal;
  try {
    const unlistenEvent = await listen<DownloadEvent>('download_event', (e) => callback(e.payload));
    const unlistenRefresh = await listen('download_refresh', () => callback({ type: 'refresh' }));
    return () => {
      unlistenLocal();
      unlistenEvent();
      unlistenRefresh();
    };
  } catch {
    return unlistenLocal;
  }
}

export async function onPlayerEvent(callback: PlayerEventListener): Promise<() => void> {
  playerEventListeners.push(callback);
  const unlistenLocal = () => {
    playerEventListeners = playerEventListeners.filter((l) => l !== callback);
  };

  if (!isTauri()) {
    return unlistenLocal;
  }

  try {
    const unlistenTauri = await listen<PlayerEvent>('player_event', (e) => {
      callback(e.payload);
    });
    return () => {
      unlistenLocal();
      unlistenTauri();
    };
  } catch {
    return unlistenLocal;
  }
}

export interface CallOptions {
  signal?: AbortSignal;
  requestId?: string;
}

let requestCounter = 0;

export function nativeAvailable() {
  return isTauri() || Boolean(import.meta.env.DEV && window.__CLOUDSTREAM_TEST__);
}

export async function cancelRequest(requestId: string): Promise<boolean> {
  try {
    if (import.meta.env.DEV && window.__CLOUDSTREAM_TEST__) {
      return (await window.__CLOUDSTREAM_TEST__('cancel_request', { requestId })) as boolean;
    }
    if (!isTauri()) return false;
    return await invoke<boolean>('cancel_request', { requestId });
  } catch {
    return false;
  }
}

export async function call<T>(
  command: string,
  args: Record<string, unknown> = {},
  options?: CallOptions,
): Promise<T> {
  const reqId =
    options?.requestId ??
    (options?.signal ? `req_${Date.now()}_${++requestCounter}` : undefined);
  const payload = reqId ? { ...args, requestId: reqId } : args;

  if (options?.signal?.aborted) {
    if (reqId) void cancelRequest(reqId);
    throw new DOMException('Request aborted', 'AbortError');
  }

  const abortHandler = () => {
    if (reqId) {
      void cancelRequest(reqId);
    }
  };

  if (options?.signal) {
    options.signal.addEventListener('abort', abortHandler, { once: true });
  }

  try {
    if (import.meta.env.DEV && window.__CLOUDSTREAM_TEST__)
      return (await window.__CLOUDSTREAM_TEST__(command, JSON.parse(JSON.stringify(payload)))) as T;
    if (!isTauri())
      throw new Error('Open the desktop app to use this feature. Start it with npm run desktop.');
    return await invoke<T>(command, payload);
  } finally {
    if (options?.signal) {
      options.signal.removeEventListener('abort', abortHandler);
    }
  }
}

export const defaultSettings: Settings = {
  schemaVersion: 1,
  revision: 1,
  global: {
    appearance: {
      posterDensity: 'comfortable',
      theme: 'dark',
      accent: 'indigo',
      posterScale: 100,
      titlePlacement: 'below',
      showBadges: true,
      motion: 'system',
      navigation: 'pointer',
    },
    playback: {
      autoNext: true,
      startPaused: false,
      completionThreshold: 0.9,
      resumeThresholdSeconds: 5.0,
      prefetchNext: true,
    },
  },
  profiles: {},
};

export const emptySnapshot: Snapshot = {
  repositories: [],
  installed: [],
  library: [],
  history: [],
  profiles: [],
  activeProfileId: 'default',
  status: {
    jvmAvailable: false,
    workerAvailable: false,
    mpvAvailable: false,
    androidAvailable: false,
    notes: [],
  },
  settings: defaultSettings,
};

export function toErrorMessage(error: unknown, fallback = 'An unexpected error occurred'): string {
  if (!error) return fallback;
  if (typeof error === 'string') return error;
  if (error instanceof Error) return error.message;
  if (typeof error === 'object' && 'message' in error && typeof error.message === 'string') {
    return error.message;
  }
  return String(error);
}

export async function getSettings(profileId?: string): Promise<Settings> {
  return call<Settings>('get_settings', profileId ? { profileId } : {});
}

export async function updateSettings(
  patch: Record<string, unknown>,
  scope?: string,
  profileId?: string,
): Promise<Settings> {
  return call<Settings>('update_settings', { patch, scope, profileId });
}

let entryListeners: (() => void)[] = [];

/** Test hook: pretend another launch just delivered a link. */
export function emitEntryPendingForTest() {
  for (const listener of entryListeners) listener();
}

if (typeof window !== 'undefined') {
  (window as unknown as { __emitEntryPending?: () => void }).__emitEntryPending = emitEntryPendingForTest;
}

/** A link or file arrived while the app is running (a second launch, a media widget, a click). */
export async function onEntryPending(callback: () => void): Promise<() => void> {
  entryListeners.push(callback);
  const unlistenLocal = () => {
    entryListeners = entryListeners.filter((l) => l !== callback);
  };
  if (!isTauri()) return unlistenLocal;
  try {
    const unlisten = await listen('entry_pending', () => callback());
    return () => {
      unlistenLocal();
      unlisten();
    };
  } catch {
    return unlistenLocal;
  }
}

let exitListeners: ((info: { playing: boolean; downloads: number }) => void)[] = [];

/** Test hook: pretend the window was asked to close. */
export function emitConfirmExitForTest(info: { playing: boolean; downloads: number }) {
  for (const listener of exitListeners) listener(info);
}

if (typeof window !== 'undefined') {
  (window as unknown as { __emitConfirmExit?: typeof emitConfirmExitForTest }).__emitConfirmExit = emitConfirmExitForTest;
}

/** The window was asked to close while something is running; the interface must ask first. */
export async function onConfirmExit(callback: (info: { playing: boolean; downloads: number }) => void): Promise<() => void> {
  exitListeners.push(callback);
  const unlistenLocal = () => {
    exitListeners = exitListeners.filter((l) => l !== callback);
  };
  if (!isTauri()) return unlistenLocal;
  try {
    const unlisten = await listen<{ playing: boolean; downloads: number }>('confirm_exit', (e) => callback(e.payload));
    return () => {
      unlistenLocal();
      unlisten();
    };
  } catch {
    return unlistenLocal;
  }
}
