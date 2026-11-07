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

export interface PlaybackPolicy {
  autoNext: boolean;
  startPaused: boolean;
  completionThreshold: number;
  resumeThresholdSeconds: number;
  prefetchNext: boolean;
  failoverAttempts?: number;
  [key: string]: unknown;
}

export interface PlaybackQueue {
  profileId: string;
  media: MediaItem;
  episodes: Episode[];
  currentIndex: number;
  dubStatus?: DubStatus;
  season?: number;
  [key: string]: unknown;
}

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
  resourceHandle?: string | null;
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
  resourceHandle?: string | null;
  [key: string]: unknown;
}

export interface Links {
  streams: Stream[];
  subtitles: Subtitle[];
  [key: string]: unknown;
}

export type ResolvedLinks = Links;

export type WatchStatus =
  | 'watching'
  | 'completed'
  | 'on_hold'
  | 'dropped'
  | 'plan_to_watch'
  | 'none';

export interface LibraryItem {
  id: string;
  item: MediaItem;
  savedAt: number;
  watchStatus?: WatchStatus;
  isFavorite?: boolean;
  isSubscribed?: boolean;
  profileId?: string;
  updatedAt?: number;
  [key: string]: unknown;
}

export interface WatchProgress {
  profileId: string;
  mediaId: string;
  episodeId: string;
  positionSeconds: number;
  durationSeconds?: number;
  updatedAtMs: number;
  isWatched: boolean;
  watchedOverride?: boolean;
  completionReason?: string;
  [key: string]: unknown;
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
  profiles: Profile[];
  activeProfileId?: string;
  extensionStates?: ExtensionState[];
  settings?: Settings;
  providers?: Provider[];
}

export type PosterDensity = 'comfortable' | 'compact';

export type ThemeName = 'dark' | 'black';
export type AccentName = 'indigo' | 'teal' | 'rose' | 'amber' | 'green' | 'violet';

export interface AppearanceSettings {
  posterDensity: PosterDensity;
  searchSuggestions?: boolean;
  theme?: ThemeName;
  accent?: AccentName;
  /** Poster width as a percentage of the density's size (80 to 140). */
  posterScale?: number;
  titlePlacement?: 'below' | 'overlay';
  showBadges?: boolean;
  motion?: 'system' | 'reduced';
  navigation?: 'pointer' | 'spatial';
  /** `system` or a language tag. */
  locale?: string;
  [key: string]: unknown;
}

export interface Profile {
  id: string;
  name: string;
  avatar: string;
  hasPin: boolean;
  createdAt: number;
  updatedAt: number;
}

export interface CreateProfileInput {
  name: string;
  avatar?: string;
  pin?: string;
}

export interface UpdateProfileInput {
  name?: string;
  avatar?: string;
  pin?: string;
  removePin?: boolean;
}

export interface GlobalSettings {
  appearance: AppearanceSettings;
  defaultProfileId?: string;
  autoSelectProfile?: boolean;
  subscriptionNotifications?: boolean;
  playback: PlaybackPolicy;
  [key: string]: unknown;
}

export interface ProfileSettings {
  appearance?: AppearanceSettings;
  selectedProvider?: string;
  playback?: PlaybackPolicy;
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
  sectionIndex?: number;
  listIndex?: number;
  page?: number;
  orientation?: 'horizontal' | 'vertical' | string;
  horizontalImages?: boolean;
  [key: string]: unknown;
}

export interface Home {
  rows: HomeRow[];
  nextSection?: number;
  hasMoreSections?: boolean;
  cached?: boolean;
  [key: string]: unknown;
}

export interface SearchHistoryEntry {
  id: string;
  profileId: string;
  query: string;
  searchedAt: number;
}

export interface SourceSearchState {
  providerId: string;
  providerName: string;
  loading: boolean;
  error: string | null;
  items: MediaItem[];
  page: number;
  hasNext: boolean;
}

export interface SearchFilters {
  source: string;
  type: string;
  language: string;
  quality: string;
  dubStatus: string;
}

export interface PlayerTrack {
  id: number;
  kind: 'audio' | 'sub';
  selected: boolean;
  external: boolean;
  forced: boolean;
  hearingImpaired: boolean;
  language?: string | null;
  title?: string | null;
  codec?: string | null;
}

export interface PlayerState {
  sessionId?: string | null;
  active: boolean;
  playing: boolean;
  paused: boolean;
  position: number;
  duration: number;
  volume: number;
  speed: number;
  itemName?: string | null;
  episodeName?: string | null;
  mediaId?: string | null;
  episodeId?: string | null;
  profileId?: string | null;
  endReason?: string | null;
  hasNext?: boolean;
  hasPrev?: boolean;
  queueIndex?: number | null;
  queueCount?: number | null;
  muted?: boolean;
  fullscreen?: boolean;
  audioDelay?: number;
  subtitleDelay?: number;
  rotation?: number;
  aspect?: 'fit' | 'fill' | 'stretch' | '16:9' | '4:3' | '21:9' | 'custom';
  hwdecCurrent?: string | null;
  stream?: { name: string; qualityLabel: string; key: string } | null;
  tracks?: PlayerTrack[];
}

export type PlayerCommand =
  | { type: 'play' }
  | { type: 'pause' }
  | { type: 'toggle_pause' }
  | { type: 'seek_relative'; seconds: number }
  | { type: 'seek_absolute'; seconds: number }
  | { type: 'set_volume'; volume: number }
  | { type: 'set_mute'; muted: boolean }
  | { type: 'set_speed'; speed: number }
  | { type: 'set_fullscreen'; fullscreen: boolean }
  | { type: 'set_aspect'; mode: Exclude<NonNullable<PlayerState['aspect']>, 'custom'> }
  | { type: 'set_audio_delay'; seconds: number }
  | { type: 'set_subtitle_delay'; seconds: number }
  | { type: 'set_audio_track'; id: number | null }
  | { type: 'set_subtitle_track'; id: number | null }
  | { type: 'set_rotation'; degrees: number };

export type PlayerEvent =
  | { type: 'state_changed'; state: PlayerState }
  | { type: 'file_loaded'; sessionId: string }
  | { type: 'end_file'; sessionId: string; reason: string }
  | { type: 'shutdown'; sessionId: string }
  | { type: 'error'; sessionId: string; message: string }
  | { type: 'queue_updated'; queue?: PlaybackQueue | null }
  | { type: 'notice'; sessionId: string; message: string }
  | { type: 'source_changed'; sessionId: string; name: string; quality: number; qualityLabel: string; reason: string; attempt: number };


export type DownloadState =
  | 'queued'
  | 'resolving'
  | 'downloading'
  | 'paused'
  | 'completed'
  | 'failed'
  | 'canceled';

export type DownloadAction = 'pause' | 'resume' | 'cancel' | 'retry';

export interface DownloadSourcePolicy {
  sourceName?: string | null;
  streamName?: string | null;
  quality?: number | null;
}

export interface DownloadJob {
  id: string;
  profileId: string;
  mediaId: string;
  episodeId: string;
  providerId: string;
  title: string;
  episodeName: string;
  season?: number | null;
  episode?: number | null;
  destination: string;
  extension: string;
  policy: DownloadSourcePolicy;
  state: DownloadState;
  bytes: number;
  total?: number | null;
  output?: string | null;
  error?: string | null;
  order: number;
  revision: number;
  createdAtMs: number;
  updatedAtMs: number;
}

export type DownloadEvent =
  | { type: 'job_changed'; job: DownloadJob }
  | { type: 'queue_error'; message: string };

export interface RemovalReport {
  fileDeleted: boolean;
  tempCleaned: boolean;
  notes: string[];
}

export type ConflictPolicy = 'skip_existing' | 'overwrite' | 'keep_newest';

export interface ImportOptions {
  sections?: string[] | null;
  conflictPolicy: ConflictPolicy;
  accountTargets?: Record<string, string>;
}

export interface SectionReport {
  section: string;
  incoming: number;
  new: number;
  replaced: number;
  unchanged: number;
  skipped: number;
}

export interface UnresolvedTitle {
  id: string;
  name: string;
  apiName: string;
  url: string;
  reason: string;
}

export interface ImportReport {
  sections: SectionReport[];
  createdProfiles: { name: string; id: string }[];
  unresolved: UnresolvedTitle[];
  repositoriesToAdd: { url: string; name: string }[];
  notes: string[];
  preImportBackup?: string | null;
}

export interface ImportPreview {
  source: 'desktop' | 'android';
  createdAtMs?: number | null;
  appVersion?: string | null;
  availableSections: string[];
  report: ImportReport;
  token: string;
}

export interface BackupExportResult {
  path: string;
  sections: string[];
  profiles: number;
  library: number;
  history: number;
  progress: number;
}

export type BackupFrequency = 'daily' | 'weekly';

export interface BackupSettings {
  enabled: boolean;
  directory?: string | null;
  frequency: BackupFrequency;
  retention: number;
}

export interface BackupStatus {
  lastAttemptMs?: number | null;
  lastSuccessMs?: number | null;
  lastFile?: string | null;
  lastBytes?: number | null;
  lastError?: string | null;
  nextDueMs?: number | null;
}

export interface BackupRun {
  file: string;
  bytes: number;
  pruned: string[];
}

export type EntryAction =
  | { type: 'add_repository'; input: string }
  | { type: 'search'; query: string }
  | { type: 'open_title'; providerName: string; url: string }
  | { type: 'open_url'; url: string }
  | { type: 'play_url'; url: string; name?: string | null }
  | { type: 'play_file'; path: string }
  | { type: 'resume_latest' }
  | { type: 'open_media'; providerId: string; url: string };

export type ExternalPlayerKind = 'none' | 'vlc' | 'custom';

export interface ExternalPlayerSettings {
  kind: ExternalPlayerKind;
  program?: string | null;
  args?: string[];
}

export interface DetectedPlayer {
  kind: ExternalPlayerKind;
  name: string;
  program: string;
}

export interface DiagnosticEvent {
  atMs: number;
  category: string;
  message: string;
}

export interface DiagnosticsReport {
  generatedAtMs: number;
  appVersion: string;
  os: string;
  arch: string;
  javaAvailable: boolean;
  workerAvailable: boolean;
  workerJarSha256?: string | null;
  mpvVersion?: string | null;
  playerUnsupported: string[];
  packages: { id: string; name: string; version: number; runtime: string; sha256: string; verified: boolean; repository: string }[];
  providers: number;
  disabledProviders: number;
  events: DiagnosticEvent[];
  timings: { atMs: number; method: string; durationMs: number; ok: boolean; outcome: string }[];
  notes: string[];
}

export interface ReleasePackage {
  kind: string;
  arch: string;
  url: string;
  sha256: string;
  size: number;
}

export type UpdateCheck =
  | { state: 'not_configured'; reason: string }
  | { state: 'up_to_date'; current: string }
  | { state: 'available'; release: { version: string; published: string; notesUrl: string; packages: ReleasePackage[] }; managedByPackageManager: boolean }
  | { state: 'skipped'; version: string }
  | { state: 'no_release'; channel: 'stable' | 'beta' };

export interface MetadataLink {
  mediaId: string;
  service: string;
  externalId: string;
  source: 'provider' | 'user';
  linkedAtMs: number;
  label?: string | null;
  year?: number | null;
}

export interface MetadataCandidate {
  service: string;
  externalId: string;
  title: string;
  year?: number | null;
  format?: string | null;
  posterUrl?: string | null;
  similarity: number;
  warnings: string[];
}

export interface MetadataResult {
  links: MetadataLink[];
  view: {
    provenance: Record<string, string>;
    posterUrl?: string | null;
    backdropUrl?: string | null;
    synopsis?: string | null;
    genres: string[];
    score?: number | null;
    trailerUrl?: string | null;
    cast: { name: string; role?: string | null; imageUrl?: string | null }[];
    recommendations: { title: string; service: string; externalId: string; posterUrl?: string | null }[];
    nextAiringEpisode?: number | null;
    nextAiringAtMs?: number | null;
    fillerEpisodeIds: string[];
    notes: string[];
  };
  errors: string[];
  stale: boolean;
}

export interface MetadataSettings {
  enabled: boolean;
  anilist: boolean;
  kitsu: boolean;
  filler: boolean;
  trailer: boolean;
  cast: boolean;
  recommendations: boolean;
  preferPoster: boolean;
}

// ---- Online subtitle services (T22) ----
export type HearingImpairedChoice = 'include' | 'exclude' | 'only';

export interface SubtitleServiceField {
  name: string;
  label: string;
  secret: boolean;
  required: boolean;
  help: string;
}

export type SubtitleServiceState =
  | { status: 'connected' }
  | { status: 'disconnected' }
  | { status: 'open' }
  | { status: 'keyring_unavailable'; message: string };

export interface SubtitleService {
  id: string;
  name: string;
  description: string;
  site: string;
  fields: SubtitleServiceField[];
  movies: boolean;
  episodes: boolean;
  enabled: boolean;
  state: SubtitleServiceState;
  account?: string | null;
}

export interface SubtitleServicesView {
  enabled: boolean;
  hearingImpaired: HearingImpairedChoice;
  languages: string[];
  keyring: { status: 'ready' } | { status: 'unavailable'; message: string };
  services: SubtitleService[];
}

export interface OnlineSubtitleResult {
  index: number;
  provider: string;
  language: string;
  release: string;
  hearingImpaired: boolean;
  forced: boolean;
  aiTranslated: boolean;
  season?: number | null;
  episode?: number | null;
  pack: boolean;
  downloads?: number | null;
  uploader?: string | null;
  matchedTitle?: string | null;
  matchedYear?: number | null;
}

export interface OnlineSubtitleOutcome {
  provider: string;
  name: string;
  status: string;
  count: number;
  message?: string | null;
  retryAfterSecs?: number | null;
}

export interface OnlineSubtitleSearch {
  searchId: string;
  title: string;
  year?: number | null;
  season?: number | null;
  episode?: number | null;
  languages: string[];
  results: OnlineSubtitleResult[];
  outcomes: OnlineSubtitleOutcome[];
}
