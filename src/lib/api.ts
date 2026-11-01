import { invoke, isTauri } from '@tauri-apps/api/core';
import type { Settings, Snapshot } from './types';

declare global {
  interface Window {
    __CLOUDSTREAM_TEST__?: (command: string, args: Record<string, unknown>) => Promise<unknown>;
  }
}
export function nativeAvailable() {
  return isTauri() || Boolean(import.meta.env.DEV && window.__CLOUDSTREAM_TEST__);
}
export async function call<T>(command: string, args: Record<string, unknown> = {}): Promise<T> {
  if (import.meta.env.DEV && window.__CLOUDSTREAM_TEST__)
    return (await window.__CLOUDSTREAM_TEST__(command, JSON.parse(JSON.stringify(args)))) as T;
  if (!isTauri())
    throw new Error('Open the desktop app to use this feature. Start it with npm run desktop.');
  return invoke<T>(command, args);
}

export const defaultSettings: Settings = {
  schemaVersion: 1,
  revision: 1,
  global: {
    appearance: {
      posterDensity: 'comfortable',
    },
  },
  profiles: {},
};

export const emptySnapshot: Snapshot = {
  repositories: [],
  installed: [],
  library: [],
  history: [],
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
