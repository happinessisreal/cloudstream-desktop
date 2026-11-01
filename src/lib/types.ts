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
  fileSize?: number;
  jarFileSize?: number;
  iconUrl?: string;
  tvTypes?: string[];
  sourceUrl?: string;
  repositoryUrl?: string;
  [key: string]: unknown;
}

export interface Repository {
  url: string;
  manifest: { name: string; description?: string; iconUrl?: string; manifestVersion: number; pluginLists: string[] };
  plugins: PluginEntry[];
  customName?: string;
  updatedAt?: number;
  [key: string]: unknown;
}

export interface ProviderCloneConfig {
  id: string;
  parentProviderId: string;
  name: string;
  customUrl: string;
  credentialsHandle?: string;
  enabled: boolean;
  [key: string]: unknown;
}

export interface ExtensionUpdate {
  pluginId: string;
  name: string;
  repositoryUrl: string;
  installedVersion: number;
  availableVersion: number;
  changelog?: string;
  hasUpdate: boolean;
  entry: PluginEntry;
}

export interface BulkUpdateResult {
  pluginId: string;
  name: string;
  success: boolean;
  error?: string;
  previousVersion: number;
  updatedVersion?: number;
}

export interface ProviderTestStageResult {
  stage: string;
  status: 'passed' | 'failed' | 'skipped';
  durationMs: number;
  message?: string;
  itemsCount?: number;
}

export interface ProviderTestReport {
  providerId: string;
  providerName: string;
  passed: boolean;
  totalDurationMs: number;
  stages: ProviderTestStageResult[];
  error?: string;
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

export interface ProviderCapabilities {
  hasMainPage: boolean;
  hasQuickSearch: boolean;
  hasDownloadSupport: boolean;
  hasChromecastSupport: boolean;
  usesWebView: boolean;
  instantLinkLoading: boolean;
  sequentialMainPage: boolean;
  sequentialMainPageDelayMs?: number;
  sequentialMainPageScrollDelayMs?: number;
  vpnStatus?: string;
  providerType?: string;
  supportedSyncNames?: string[];
  supportedTypes?: string[];
  [key: string]: unknown;
}

export interface ProviderTimeouts {
  searchTimeoutMs?: number;
  quickSearchTimeoutMs?: number;
  getMainPageTimeoutMs?: number;
  loadTimeoutMs?: number;
  loadLinksTimeoutMs?: number;
  [key: string]: unknown;
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
  disabled?: boolean;
  cloneOf?: string;
  capabilities?: ProviderCapabilities;
  timeouts?: ProviderTimeouts;
  [key: string]: unknown;
}

export type DubStatus = 'none' | 'dubbed' | 'subbed' | 'both';

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
  score?: number;
  rating?: number;
  quality?: string;
  id?: number;
  otherName?: string;
  episodesCount?: number;
  dubStatus?: DubStatus;
  dubEpisodes?: Record<string, number>;
  [key: string]: unknown;
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
  score?: number;
  rating?: number;
  description?: string;
  date?: number;
  runTime?: number;
  [key: string]: unknown;
}

export interface TrailerData {
  extractorUrl: string;
  referer?: string;
  raw?: boolean;
  headers?: Record<string, string>;
  [key: string]: unknown;
}

export interface Actor {
  name: string;
  image?: string;
  [key: string]: unknown;
}

export interface ActorData {
  actor: Actor;
  role?: string;
  roleString?: string;
  voiceActor?: Actor;
  [key: string]: unknown;
}

export interface SeasonData {
  season?: number;
  name?: string;
  episodes: Episode[];
  [key: string]: unknown;
}

export interface Details {
  item: MediaItem;
  plot?: string;
  tags: string[];
  episodes: Episode[];
  duration?: number;
  uniqueUrl?: string;
  backgroundPosterUrl?: string;
  logoUrl?: string;
  contentRating?: string;
  comingSoon?: boolean;
  syncData?: Record<string, string>;
  trailers?: TrailerData[];
  actors?: ActorData[];
  recommendations?: MediaItem[];
  dubGroups?: Record<string, Episode[]>;
  seasons?: SeasonData[];
  [key: string]: unknown;
}

export type MediaDetails = Details;

export interface AudioTrack {
  url: string;
  headers?: Record<string, string>;
  [key: string]: unknown;
}

export interface PlaylistItem {
  url: string;
  durationUs?: number;
  [key: string]: unknown;
}

export interface DrmData {
  uuid?: string;
  keyId?: string;
  kid?: string;
  key?: string;
  k?: string;
  kty?: string;
  licenseUrl?: string;
  keyRequestParameters?: Record<string, string>;
  headers?: Record<string, string>;
  [key: string]: unknown;
}

export interface Stream {
  name: string;
  url: string;
  referer: string;
  quality: number;
  headers: Record<string, string>;
  streamType: string;
  kind?: 'direct' | 'hls' | 'dash' | 'torrent' | 'magnet' | 'playlist' | 'drm' | string;
  source?: string;
  extractorData?: string;
  resourceHandle?: string;
  audioTracks?: AudioTrack[];
  playlist?: PlaylistItem[];
  drm?: DrmData;
  [key: string]: unknown;
}

export interface Subtitle {
  lang: string;
  url: string;
  headers: Record<string, string>;
  langTag?: string;
  [key: string]: unknown;
}

export interface Links {
  streams: Stream[];
  subtitles: Subtitle[];
  [key: string]: unknown;
}

export type ResolvedLinks = Links;

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

export interface ExtensionState {
  pluginId: string;
  loaded: boolean;
  runtime: string;
  format?: 'dex' | 'jar' | string;
  apiVersion?: number;
  pluginType?: 'provider' | 'extractor' | 'importer' | 'empty' | string;
  providers?: string[];
  extractors?: string[];
  requiredUnavailableApis?: string[];
  actionableFailure?: string;
  sha256?: string;
  error?: string;
  [key: string]: unknown;
}

export interface Snapshot {
  repositories: Repository[];
  installed: InstalledPlugin[];
  library: LibraryItem[];
  history: WatchEntry[];
  status: RuntimeStatus;
  extensionStates?: ExtensionState[];
  settings?: Settings;
  providers?: Provider[];
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

export interface HomeRow {
  name: string;
  items: MediaItem[];
  hasNext: boolean;
  rowId?: string;
  orientation?: 'horizontal' | 'vertical' | string;
  horizontalImages?: boolean;
  [key: string]: unknown;
}

export interface Home {
  rows: HomeRow[];
  nextSection?: number;
  hasMoreSections?: boolean;
  [key: string]: unknown;
}
