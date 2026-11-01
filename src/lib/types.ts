export interface PluginEntry {
  url: string;
  name: string;
  internalName: string;
  version: number;
  status: number;
  apiVersion: number;
  authors: string[];
  description?: string;
  language?: string;
  jarUrl?: string;
  jarHash?: string;
  fileHash?: string;
  iconUrl?: string;
  tvTypes?: string[];
}
export interface Repository {
  url: string;
  manifest: { name: string; description?: string; manifestVersion: number; pluginLists: string[] };
  plugins: PluginEntry[];
}
export interface InstalledPlugin {
  id: string;
  repositoryUrl: string;
  entry: PluginEntry;
  runtime: 'jvm' | 'android';
  verified: boolean;
  sha256: string;
  installedAt: number;
}
export interface Provider {
  id: string;
  pluginId: string;
  name: string;
  lang: string;
  mainUrl: string;
  hasMainPage: boolean;
  usesWebView: boolean;
  supportedTypes: string[];
}
export type DubStatus = 'none' | 'dubbed' | 'subbed';

export interface MediaRef {
  mediaId: string;
  providerId: string;
  url: string;
  uniqueUrl?: string;
  externalIds?: Record<string, string>;
}

export interface EpisodeRef {
  episodeId: string;
  mediaId: string;
  dubStatus: DubStatus;
  season?: number;
  episode?: number;
  data: string;
  aliases?: string[];
}

export interface WatchProgress {
  profileId: string;
  mediaId: string;
  episodeId: string;
  positionSeconds: number;
  durationSeconds?: number;
  updatedAtMs: number;
  isWatched: boolean;
  completionReason?: string;
}

export interface MediaItem {
  providerId: string;
  name: string;
  url: string;
  posterUrl?: string;
  posterHeaders: Record<string, string>;
  mediaType?: string;
  year?: number;
  uniqueUrl?: string;
  mediaId?: string;
}
export interface Episode {
  name: string;
  data: string;
  season?: number;
  episode?: number;
  posterUrl?: string;
  id?: string;
  dubStatus?: DubStatus;
  index?: number;
}
export interface Details {
  item: MediaItem;
  plot?: string;
  tags: string[];
  episodes: Episode[];
  duration?: number;
}
export interface Stream {
  name: string;
  url: string;
  referer: string;
  quality: number;
  headers: Record<string, string>;
  streamType: string;
}
export interface Subtitle {
  lang: string;
  url: string;
  headers: Record<string, string>;
}
export interface Links {
  streams: Stream[];
  subtitles: Subtitle[];
}
export interface LibraryItem {
  id: string;
  item: MediaItem;
  savedAt: number;
}
export interface WatchEntry {
  id: string;
  item: MediaItem;
  episode: Episode;
  position: number;
  duration: number;
  watchedAt: number;
}
export interface RuntimeStatus {
  jvmAvailable: boolean;
  workerAvailable: boolean;
  mpvAvailable: boolean;
  androidAvailable: boolean;
  notes: string[];
}
export interface Snapshot {
  repositories: Repository[];
  installed: InstalledPlugin[];
  library: LibraryItem[];
  history: WatchEntry[];
  status: RuntimeStatus;
  extensionStates?: { pluginId: string; loaded: boolean; runtime: string; error?: string }[];
  settings?: Settings;
}
export type PosterDensity = 'comfortable' | 'compact';
export interface AppearanceSettings {
  posterDensity: PosterDensity;
  [key: string]: unknown;
}
export interface GlobalSettings {
  appearance: AppearanceSettings;
  [key: string]: unknown;
}
export interface ProfileSettings {
  appearance?: AppearanceSettings;
  [key: string]: unknown;
}
export interface Settings {
  schemaVersion: number;
  revision: number;
  global: GlobalSettings;
  profiles: Record<string, ProfileSettings>;
  [key: string]: unknown;
}
export interface SearchResult {
  items: MediaItem[];
  hasNext: boolean;
}
export interface Home {
  rows: { name: string; items: MediaItem[]; hasNext: boolean }[];
  nextSection?: number;
  hasMoreSections?: boolean;
}
