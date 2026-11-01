import { invoke, isTauri } from '@tauri-apps/api/core';
import type { Snapshot } from './types';

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
};
