<script lang="ts">
  import { onMount, onDestroy, tick, untrack } from 'svelte';
  import {
    Compass,
    Download as DownloadIcon,
    Library,
    Puzzle,
    Settings,
    Search,
    Plus,
    ArrowRight,
    ArrowUpRight,
    Cloud,
    Play,
    Check,
    X,
    RefreshCw,
    Trash2,
    ChevronLeft,
    ChevronRight,
    Film,
    ShieldCheck,
    HardDrive,
    Terminal,
    CircleHelp,
    Globe,
    MonitorPlay,
    Bookmark,
    LoaderCircle,
    Pause,
    LockKeyhole,
    LockKeyholeOpen,
    SkipBack,
    SkipForward,
    Square,
    Link,
    Layers,
    CheckCircle2,
    AlertCircle,
    ExternalLink,
    Home as HomeIcon,
    ListFilter,
    Grid2X2,
    LayoutGrid,
    Edit2,
    Share2,
    Upload,
    ToggleLeft,
    ToggleRight,
    Copy,
    Activity,
    FlaskConical,
    CheckCheck,
    Clock,
    Heart,
    Bell,
    Circle,
    Shuffle,
    Sparkles,
    Lock,
    Users,
    UserPlus,
    Star,
    Calendar,
    ArrowUpDown,
    Video,
    RotateCcw,
    RotateCw,
  } from 'lucide-svelte';
  import { call, nativeAvailable, emptySnapshot, getSettings, updateSettings, toErrorMessage, onPlayerEvent, onEntryPending, onConfirmExit } from './lib/api';
  import MediaCard from './lib/MediaCard.svelte';
  import MediaRail from './lib/MediaRail.svelte';
  import Spotlight from './lib/Spotlight.svelte';
  import Downloads from './lib/Downloads.svelte';
  import Backup from './lib/Backup.svelte';
  import Metadata from './lib/Metadata.svelte';
  import MetadataSettings from './lib/MetadataSettings.svelte';
  import SubtitleServicesSettings from './lib/SubtitleServicesSettings.svelte';
  import OnlineSubtitles from './lib/OnlineSubtitles.svelte';
  import FirstRun from './lib/FirstRun.svelte';
  import { formatDateTime, i18n, languageName, setLocalePreference, supportedLocales, t, tn } from './lib/i18n.svelte';
  import Incremental from './lib/Incremental.svelte';
  import Diagnostics from './lib/Diagnostics.svelte';
  import EntryPrompt from './lib/EntryPrompt.svelte';
  import ExternalPlayerSettings from './lib/ExternalPlayerSettings.svelte';
  import type {
    Snapshot,
    Provider,
    MediaItem,
    Details,
    Episode,
    Links,
    Stream,
    Home,
    SearchResult,
    PluginEntry,
    InstalledPlugin,
    WatchEntry,
    PosterDensity,
    Settings as AppSettingsType,
    Repository,
    ProviderCloneConfig,
    ExtensionUpdate,
    BulkUpdateResult,
    ProviderTestReport,
    SearchHistoryEntry,
    SourceSearchState,
    SearchFilters,
    WatchStatus,
    WatchProgress,
    PlayerState,
    PlayerCommand,
    PlayerEvent,
    PlaybackPolicy,
    PlaybackQueue,
    Profile,
    CreateProfileInput,
    UpdateProfileInput,
    TrailerData,
    ActorData,
    SeasonData,
  } from './lib/types';
  import type { Settings as AppSettings, AccentName, AppearanceSettings, EntryAction, ExternalPlayerSettings as ExternalPlayerConfig } from './lib/types';

  type Page = 'discover' | 'library' | 'downloads' | 'extensions' | 'settings';
  let page = $state<Page>('discover');
  let searchView = $state(false);
  let contentFilter = $state('All');
  let featuredIndex = $state(0);
  let expandedCategory = $state<number | null>(null);
  let categoryPages = $state<Record<string, number>>({});
  let posterDensity = $state<PosterDensity>(
    (localStorage.getItem('cloudstream.poster-density') as PosterDensity) === 'compact'
      ? 'compact'
      : 'comfortable',
  );
  let snapshot = $state<Snapshot>(structuredClone(emptySnapshot));
  let providers = $state<Provider[]>([]);
  let selectedProvider = $state('');
  let activeProfile = $state('default');
  const currentProfile = $derived(
    snapshot.profiles?.find((p) => p.id === activeProfile) ??
      snapshot.profiles?.[0] ?? {
        id: 'default',
        name: 'Default Profile',
        avatar: 'default',
        hasPin: false,
        createdAt: 0,
        updatedAt: 0,
      },
  );
  let home = $state<Home>({ rows: [] });
  let query = $state('');
  let searchItems = $state<MediaItem[]>([]);
  let searchActive = $state(false);
  let submittedQuery = $state('');
  let searchMode = $state<'all' | 'custom'>('all');
  let selectedSearchProviders = $state<string[]>([]);
  let sourceSearchStates = $state<Record<string, SourceSearchState>>({});
  let searchViewMode = $state<'grouped' | 'merged'>('grouped');
  let searchFilters = $state<SearchFilters>({
    source: 'all',
    type: 'all',
    language: 'all',
    quality: 'all',
    dubStatus: 'all',
  });
  let searchHistory = $state<SearchHistoryEntry[]>([]);
  let searchSuggestions = $state<string[]>([]);
  let showSuggestionsDropdown = $state(false);
  let suggestionAbortController: AbortController | null = null;
  let suggestionDebounceTimer: ReturnType<typeof setTimeout> | null = null;
  let loadingCategories = $state(false);
  let searchPage = $state(1);
  let searchHasNext = $state(false);
  let searchVersion = 0;
  let providerVersion = 0;
  let searchAbortController: AbortController | null = null;
  let resolveAbortController: AbortController | null = null;
  let detailsAbortController: AbortController | null = null;
  let ready = $state(false);
  let busy = $state('');
  let loadingContent = $state(false);
  let error = $state('');
  let notice = $state('');
  let modal = $state<
    | 'repository'
    | 'direct'
    | 'rename'
    | 'localInstall'
    | 'clone'
    | 'test'
    | 'clearLibrary'
    | 'clearHistory'
    | 'profiles'
    | 'profileForm'
    | 'profilePin'
    | 'deleteProfile'
    | 'subtitleArchive'
    | null
  >(null);
  let onlineSubtitlesOpen = $state(false);
  let subtitleArchive = $state<{ id: string; entries: { index: number; name: string; size: number }[] } | null>(null);
  let pendingSwitchProfileId = $state<string | null>(null);
  let pinInput = $state('');
  let pinError = $state('');
  let editingProfile = $state<Profile | null>(null);
  let profileNameInput = $state('');
  let profileAvatarInput = $state('default');
  let profilePinInput = $state('');
  let profileRemovePin = $state(false);
  let deleteTargetProfileId = $state<string | null>(null);
  let repoUrl = $state('');
  let directUrl = $state('');
  let repoFilter = $state('');
  let languageFilter = $state('all');
  let typeFilter = $state<'all' | 'provider' | 'extractor'>('all');
  let statusFilter = $state<'all' | 'installed' | 'updates'>('all');
  let renameRepoUrl = $state('');
  let renameValue = $state('');
  let localPackagePath = $state('');
  let cloneParentProviderId = $state('');
  let cloneName = $state('');
  let cloneUrl = $state('');
  let updates = $state<ExtensionUpdate[]>([]);
  let testReport = $state<ProviderTestReport | null>(null);
  let testingProvider = $state<Provider | null>(null);
  let testAbortController: AbortController | null = null;
  const availableUpdatesCount = $derived(updates.filter((u) => u.hasUpdate).length);
  let extensionTab = $state<'browse' | 'installed'>('browse');
  let details = $state<Details | null>(null);
  const downloadsSupported = $derived(
    !!details?.item &&
      (providers.find((p) => p.id === details!.item.providerId)?.capabilities?.hasDownloadSupport ?? true),
  );
  let selectedEpisode = $state<Episode | null>(null);
  let selectedSeason = $state<number | 'all'>('all');
  let selectedDubGroup = $state<string>('all');
  let episodeRangeIndex = $state(0);
  let episodeSortOrder = $state<'asc' | 'desc'>('asc');
  const DOWNLOAD_TYPES = ['mp4', 'mkv', 'webm', 'mov', 'avi', 'm4v', 'ts', 'hls'] as const;
  let downloadExtension = $state<string>(
    (() => {
      try {
        const saved = localStorage.getItem('cloudstream.download-extension');
        return saved && (DOWNLOAD_TYPES as readonly string[]).includes(saved) ? saved : 'mp4';
      } catch {
        return 'mp4';
      }
    })(),
  );
  let lastFocusedElement: HTMLElement | null = null;
  let lastFocusedMediaUrl: string | null = null;
  let links = $state<Links | null>(null);
  type RankedStream = { index: number; usable: boolean; qualityLabel: string; errored: boolean };
  let rankedStreams = $state<RankedStream[]>([]);
  const visibleStreams = $derived(rankedStreams.filter((entry) => entry.usable && links?.streams[entry.index]));
  let playerState = $state<PlayerState | null>(null);
  let playerLocked = $state(false);
  let lockSession = '';
  let holdSession = $state<string | null>(null);
  let holdStart: Promise<unknown> = Promise.resolve();
  function endSpeedHold() {
    const sessionId = holdSession;
    if (!sessionId) return;
    holdSession = null;
    void holdStart.then(() => call('end_speed_hold', { sessionId })).catch(() => {});
  }
  function beginSpeedHold() {
    const sessionId = playerState?.sessionId;
    if (!sessionId || holdSession || playerLocked) return;
    holdSession = sessionId;
    holdStart = call('begin_speed_hold', { sessionId }).catch((error) => { if (holdSession === sessionId) holdSession = null; showError(error); });
  }
  $effect(() => {
    const session = playerState?.sessionId ?? '';
    if (lockSession !== session) { lockSession = session; playerLocked = false; }
    if (holdSession && holdSession !== session) endSpeedHold();
  });
  type SkipMarker = { kind: string; start: number; end: number; source: string };
  let skipMarkers = $state<SkipMarker[]>([]);
  let skipMarkerSession = $state('');
  let requestedSkipKey = '';
  let skipSaving = $state(false);
  const aniskipEnabled = $derived(Boolean((snapshot.settings?.global as { skip?: { aniskip?: boolean } } | undefined)?.skip?.aniskip));
  $effect(() => {
    const session = playerState?.sessionId;
    const duration = playerState?.duration ?? 0;
    const enabled = aniskipEnabled;
    if (!enabled || !session || !playerState?.active || !playerState.episodeId || duration < 10) {
      requestedSkipKey = '';
      skipMarkers = [];
      skipMarkerSession = '';
      return;
    }
    const key = `${session}:${Math.round(duration)}`;
    if (key === requestedSkipKey) return;
    requestedSkipKey = key;
    skipMarkers = [];
    skipMarkerSession = '';
    void call<SkipMarker[]>('player_skip_markers', { sessionId: session }).then((markers) => {
      if (requestedSkipKey === key && playerState?.sessionId === session && aniskipEnabled) { skipMarkers = markers; skipMarkerSession = session; }
    }).catch(() => { /* Skip service outages never interrupt playback. */ });
  });
  async function setAniskip(enabled: boolean) {
    if (skipSaving) return;
    skipSaving = true;
    try { snapshot.settings = await updateSettings({ skip: { aniskip: enabled } }, 'global'); }
    catch (e) { showError(e); }
    finally { skipSaving = false; }
  }
  async function skipMarker(marker: SkipMarker) {
    const sessionId = playerState?.sessionId;
    if (!sessionId) return;
    try { await call('player_skip_interval', { sessionId, start: marker.start, end: marker.end }); }
    catch (e) { showError(e); }
  }
  function skipLabel(kind: string) {
    return kind === 'recap' ? 'Skip recap' : kind === 'ed' || kind === 'mixed-ed' ? 'Skip ending' : 'Skip opening';
  }
  let playerCapabilities = $state<{ capabilities: { id: string; label: string; supported: boolean; reason?: string }[] } | null>(null);
  const playerPreferences = $derived((snapshot.settings?.global?.player ?? {}) as { decoding?: string; bufferRamMb?: number; bufferSeconds?: number; audioCompressor?: boolean; seekStepSeconds?: number; holdSpeed?: number; subtitlesEnabled?: boolean | null; subtitleLanguage?: string | null; audioLanguage?: string | null; audioEnabled?: boolean | null; subtitleStyle?: Partial<SubtitleStyle>; subtitleTransforms?: Partial<SubtitleTransforms> });
  async function updatePlayerPreferences(patch: Record<string, unknown>) {
    try {
      const updated = await updateSettings({ player: { ...playerPreferences, ...patch } }, 'global');
      snapshot.settings = updated;
      message('Playback preferences updated.');
    } catch (e) { showError(e); }
  }

  type SubtitleTransforms = { removeCaptions: boolean; removeMusic: boolean; removeBloat: boolean; uppercase: boolean };
  const subtitleTransforms = $derived({ removeCaptions: false, removeMusic: false, removeBloat: false, uppercase: false, ...playerPreferences.subtitleTransforms });
  let subtitleTransformsSaving = $state(false);
  async function updateSubtitleTransforms(patch: Partial<SubtitleTransforms>) {
    if (subtitleTransformsSaving) return;
    subtitleTransformsSaving = true;
    try { await updatePlayerPreferences({ subtitleTransforms: { ...subtitleTransforms, ...patch } }); }
    finally { subtitleTransformsSaving = false; }
  }

  type SubtitleStyle = { font: string; fontId: string | null; size: number; color: string; bold: boolean; italic: boolean; outlineColor: string; outlineSize: number; shadowOffset: number; backgroundColor: string; backgroundOpacity: number; backgroundBox: boolean; alignX: 'left' | 'center' | 'right'; alignY: 'top' | 'center' | 'bottom'; verticalMargin: number; overrideAss: boolean; [key: string]: unknown };
  const defaultSubtitleStyle = (): SubtitleStyle => ({ font: 'sans-serif', fontId: null, size: 38, color: '#ffffff', bold: false, italic: false, outlineColor: '#000000', outlineSize: 1.65, shadowOffset: 0, backgroundColor: '#000000', backgroundOpacity: 69, backgroundBox: false, alignX: 'center', alignY: 'bottom', verticalMargin: 34, overrideAss: false });
  const subtitleStyle = $derived({ ...defaultSubtitleStyle(), ...playerPreferences.subtitleStyle });
  let previousSubtitleStyle = $state<SubtitleStyle | null>(null);
  let subtitleStyleSaving = $state(false);
  async function saveSubtitleStyle(style: SubtitleStyle) {
    if (subtitleStyleSaving) return;
    subtitleStyleSaving = true;
    const before = { ...subtitleStyle };
    try {
      const updated = await updateSettings({ player: { ...playerPreferences, subtitleStyle: style } }, 'global');
      snapshot.settings = updated;
      previousSubtitleStyle = before;
      message('Subtitle appearance updated.');
    } catch (e) { showError(e); }
    finally { subtitleStyleSaving = false; }
  }
  function updateSubtitleStyle(patch: Partial<SubtitleStyle>) { void saveSubtitleStyle({ ...subtitleStyle, ...patch }); }
  const selectedSubtitleFontId = $derived(subtitleStyle.fontId);
  let previewFontFamily = $state<string | null>(null);
  let previewFontError = $state('');
  $effect(() => {
    const id = selectedSubtitleFontId;
    let cancelled = false;
    let loaded: FontFace | null = null;
    previewFontFamily = null;
    previewFontError = '';
    if (id) void (async () => {
      try {
        const encoded = await call<string>('subtitle_font_preview', { id });
        const binary = atob(encoded);
        const bytes = new Uint8Array(binary.length);
        for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
        const face = new FontFace(`Cloudstream-${id}`, bytes.buffer);
        await face.load();
        if (cancelled) return;
        document.fonts.add(face);
        loaded = face;
        previewFontFamily = face.family;
      } catch {
        if (!cancelled) previewFontError = 'The custom font preview could not be loaded. Choose the font again if it is missing.';
      }
    })();
    return () => { cancelled = true; if (loaded) document.fonts.delete(loaded); };
  });
  async function chooseSubtitleFont() {
    if (subtitleStyleSaving) return;
    try {
      const font = await call<{ id: string; family: string } | null>('choose_subtitle_font');
      if (font) await saveSubtitleStyle({ ...subtitleStyle, font: font.family, fontId: font.id });
    } catch (e) { showError(e); }
  }

  function subtitlePreviewColor(style: SubtitleStyle) {
    const alpha = Math.round(style.backgroundOpacity * 255 / 100).toString(16).padStart(2, '0');
    return `${style.backgroundColor}${alpha}`;
  }

  type SourceProfile = { id: string; name: string; qualityPriorities: Record<string, number>; sourcePriorities: Record<string, number>; preferredLanguage?: string | null; hideNegativeSources: boolean; hideErrorSources: boolean };
  type SourceConfig = { activeProfileId: string; profiles: SourceProfile[] };
  const defaultSourceProfile = (): SourceProfile => ({ id: 'default', name: 'Default', qualityPriorities: {}, sourcePriorities: {}, hideNegativeSources: false, hideErrorSources: false });
  const sourceConfig = $derived((snapshot.settings?.profiles?.[activeProfile]?.sourcePriority as SourceConfig | undefined) ?? { activeProfileId: 'default', profiles: [defaultSourceProfile()] });
  const sourceProfile = $derived(sourceConfig.profiles.find((profile) => profile.id === sourceConfig.activeProfileId) ?? sourceConfig.profiles[0]);
  let newSourceProfileName = $state('');
  let newSourceName = $state('');
  async function saveSourceConfig(config: SourceConfig) {
    try {
      snapshot.settings = await updateSettings({ sourcePriority: config }, 'profile', activeProfile);
      message('Source preferences updated.');
    } catch (e) { showError(e); }
  }
  function updateSourceProfile(patch: Partial<SourceProfile>) {
    void saveSourceConfig({ ...sourceConfig, profiles: sourceConfig.profiles.map((profile) => profile.id === sourceProfile.id ? { ...profile, ...patch } : profile) });
  }
  function addSourceProfile() {
    const name = newSourceProfileName.trim();
    if (!name) return;
    if (sourceConfig.profiles.length >= 12) { message('You can keep up to 12 source profiles.'); return; }
    const profile = { ...defaultSourceProfile(), id: crypto.randomUUID(), name };
    void saveSourceConfig({ ...sourceConfig, activeProfileId: profile.id, profiles: [...sourceConfig.profiles, profile] });
    newSourceProfileName = '';
  }
  function removeSourceProfile() {
    if (sourceConfig.profiles.length <= 1) return;
    const profiles = sourceConfig.profiles.filter((profile) => profile.id !== sourceProfile.id);
    void saveSourceConfig({ ...sourceConfig, profiles, activeProfileId: profiles[0].id });
  }
  function addSourcePriority() {
    const name = newSourceName.trim();
    if (!name) return;
    updateSourceProfile({ sourcePriorities: { ...sourceProfile.sourcePriorities, [name]: 1 } });
    newSourceName = '';
  }
  function removeSourcePriority(name: string) {
    const priorities = { ...sourceProfile.sourcePriorities };
    delete priorities[name];
    // Settings patches merge objects, so explicitly clear deleted keys.
    updateSourceProfile({ sourcePriorities: priorities });
  }

  let toastTimer: ReturnType<typeof setTimeout>;
  let dialogElement: HTMLDialogElement;
  let searchInput: HTMLInputElement;

  const activeProvider = $derived(providers.find((p) => p.id === selectedProvider));
  function matchesFilter(item: MediaItem) {
    if (contentFilter === 'All') return true;
    if (contentFilter === 'Movies') return ['Movie', 'AnimeMovie'].includes(item.mediaType ?? '');
    if (contentFilter === 'TV series')
      return ['TvSeries', 'AsianDrama', 'Cartoon'].includes(item.mediaType ?? '');
    return ['Anime', 'AnimeMovie', 'OVA'].includes(item.mediaType ?? '');
  }
  const featuredItems = $derived(home.rows.flatMap((row) => row.items).filter(matchesFilter));
  const featuredItem = $derived(featuredItems[featuredIndex % Math.max(1, featuredItems.length)]);
  const installedCount = $derived(snapshot.installed.length);
  const jvmCount = $derived(snapshot.installed.filter((p) => p.runtime === 'jvm').length);
  const failedExtensions = $derived(snapshot.extensionStates?.filter((s) => !s.loaded) ?? []);

  const availableLanguages = $derived([
    'all',
    ...Array.from(
      new Set(
        snapshot.repositories
          .flatMap((r) => r.plugins)
          .map((p) => (p.language ? p.language.toLowerCase() : 'multi')),
      ),
    ).sort(),
  ]);

  const entries = $derived(
    snapshot.repositories
      .flatMap((repo) => repo.plugins.map((entry) => ({ repo, entry })))
      .filter(({ repo, entry }) => {
        const installed = snapshot.installed.find(
          (p) => p.repositoryUrl === repo.url && p.entry.internalName === entry.internalName,
        );

        if (repoFilter.trim()) {
          const q = repoFilter.trim().toLowerCase();
          const authorStr = (entry.authors ?? []).join(' ').toLowerCase();
          const match = `${entry.name} ${entry.language ?? ''} ${entry.description ?? ''} ${authorStr}`
            .toLowerCase()
            .includes(q);
          if (!match) return false;
        }

        if (languageFilter === 'mine') {
          const lang = (entry.language ?? 'multi').toLowerCase();
          // Language-neutral extensions (multi) are always shown.
          if (lang !== 'multi' && !sourceLanguagePrefs.some((l) => lang === l.toLowerCase() || lang.startsWith(`${l.toLowerCase()}-`))) return false;
        } else if (languageFilter !== 'all') {
          const lang = (entry.language ?? 'multi').toLowerCase();
          if (lang !== languageFilter.toLowerCase()) return false;
        }

        if (typeFilter === 'provider') {
          if (entry.tvTypes && entry.tvTypes.length === 0) return false;
        } else if (typeFilter === 'extractor') {
          if (entry.tvTypes && entry.tvTypes.length > 0) return false;
        }

        if (statusFilter === 'installed') {
          if (!installed) return false;
        } else if (statusFilter === 'updates') {
          if (!installed || installed.entry.version >= entry.version) return false;
        }

        return true;
      }),
  );
  type LibraryTab =
    | 'all'
    | 'watching'
    | 'plan_to_watch'
    | 'completed'
    | 'on_hold'
    | 'dropped'
    | 'favorites'
    | 'subscribed';

  let libraryTab = $state<LibraryTab>(
    (typeof localStorage !== 'undefined' &&
      (localStorage.getItem('cloudstream:library:tab') as LibraryTab)) ||
      'all',
  );
  let libraryQuery = $state('');
  let librarySort = $state<string>(
    (typeof localStorage !== 'undefined' &&
      localStorage.getItem('cloudstream:library:sort')) ||
      'updated',
  );
  let episodeProgresses = $state<WatchProgress[]>([]);

  function onLibraryTabChange(tab: LibraryTab) {
    libraryTab = tab;
    try {
      localStorage.setItem('cloudstream:library:tab', tab);
    } catch {}
  }

  function onLibrarySortChange(sort: string) {
    librarySort = sort;
    try {
      localStorage.setItem('cloudstream:library:sort', sort);
    } catch {}
  }

  const detailLibraryItem = $derived(
    details
      ? snapshot.library.find(
          (saved) =>
            (saved.item.mediaId && details?.item.mediaId && saved.item.mediaId === details.item.mediaId) ||
            (saved.item.providerId === details?.item.providerId && saved.item.url === details?.item.url),
        ) ?? null
      : null,
  );
  const detailSaved = $derived(Boolean(detailLibraryItem));
  const detailStatus = $derived<WatchStatus>(detailLibraryItem?.watchStatus ?? 'none');
  const detailFavorite = $derived<boolean>(Boolean(detailLibraryItem?.isFavorite));
  const detailSubscribed = $derived<boolean>(Boolean(detailLibraryItem?.isSubscribed));

  const libraryTabFiltered = $derived.by(() => {
    return snapshot.library.filter((saved) => {
      if (libraryTab === 'all') return true;
      if (libraryTab === 'favorites') return Boolean(saved.isFavorite);
      if (libraryTab === 'subscribed') return Boolean(saved.isSubscribed);
      return saved.watchStatus === libraryTab;
    });
  });

  const librarySearchFiltered = $derived.by(() => {
    const q = libraryQuery.trim().toLowerCase();
    if (!q) return libraryTabFiltered;
    return libraryTabFiltered.filter((saved) =>
      (saved.item.name ?? '').toLowerCase().includes(q),
    );
  });

  const hasScoredItems = $derived.by(() => {
    return snapshot.library.some((saved) => (saved.item.score ?? 0) > 0);
  });

  const sortedLibrary = $derived.by(() => {
    const list = [...librarySearchFiltered];
    if (librarySort === 'updated') {
      list.sort((a, b) => (b.updatedAt ?? b.savedAt) - (a.updatedAt ?? a.savedAt));
    } else if (librarySort === 'alpha_asc') {
      list.sort((a, b) => (a.item.name ?? '').localeCompare(b.item.name ?? ''));
    } else if (librarySort === 'alpha_desc') {
      list.sort((a, b) => (b.item.name ?? '').localeCompare(a.item.name ?? ''));
    } else if (librarySort === 'release_date') {
      list.sort((a, b) => (b.item.year ?? 0) - (a.item.year ?? 0));
    } else if (librarySort === 'score') {
      list.sort((a, b) => (b.item.score ?? 0) - (a.item.score ?? 0));
    }
    return list;
  });

  const homePlanToWatchItems = $derived.by(() => {
    return snapshot.library
      .filter((s) => s.watchStatus === 'plan_to_watch')
      .map((s) => s.item);
  });

  const homeFavoriteItems = $derived.by(() => {
    return snapshot.library
      .filter((s) => s.isFavorite)
      .map((s) => s.item);
  });

  let spotlightDetails = $state<Details | null>(null);
  let spotlightAbortController: AbortController | null = null;

  $effect(() => {
    const item = featuredItem;
    spotlightAbortController?.abort();
    spotlightDetails = null;
    if (item && item.providerId && item.url) {
      const controller = new AbortController();
      spotlightAbortController = controller;
      void (async () => {
        try {
          const d = await call<Details>(
            'load_details',
            { providerId: item.providerId, url: item.url, profileId: activeProfile },
            { signal: controller.signal },
          );
          if (!controller.signal.aborted) {
            spotlightDetails = d;
          }
        } catch {
          // ignore background detail resolution failure
        }
      })();
    }
  });

  function pickRandomSource() {
    const candidates = providers.filter((p) => !p.disabled && p.hasMainPage);
    if (!candidates.length) return;
    const chosen = candidates[Math.floor(Math.random() * candidates.length)];
    void chooseProvider(chosen.id);
  }

  function pickRandomTitle() {
    const candidates = home.rows.flatMap((r) => r.items);
    if (candidates.length > 0) {
      const item = candidates[Math.floor(Math.random() * candidates.length)];
      void openItem(item);
    } else if (snapshot.library.length > 0) {
      const item = snapshot.library[Math.floor(Math.random() * snapshot.library.length)].item;
      void openItem(item);
    }
  }

  // ---- Metadata (T24) ----
  let metadataFillerIds = $state<string[]>([]);
  let metadataArt = $state<{ posterUrl?: string; backdropUrl?: string }>({});
  const metadataEnabled = $derived(Boolean((snapshot.settings?.global as { metadata?: { enabled?: boolean } } | undefined)?.metadata?.enabled));
  function findSourceFor(title: string) {
    query = title;
    openSearch();
    void runSearch();
  }

  // ---- Language and first-run setup (T31) ----
  let setupOpen = $state(false);
  const setupState = $derived((snapshot.settings?.global as { setup?: { completed?: boolean } } | undefined)?.setup);
  const sourceLanguagePrefs = $derived(((snapshot.settings?.global as { providerLanguages?: string[] } | undefined)?.providerLanguages) ?? []);
  $effect(() => {
    setLocalePreference(appearance.locale ?? 'system');
  });
  // Show setup once, on a profile that is unlocked. Installs that already have data are marked as
  // set up without asking, so an upgrade does not greet people who are long past the beginning.
  $effect(() => {
    if (!subscriptionProfileReady || !setupState || setupState.completed !== false) return;
    const hasData = snapshot.repositories.length > 0 || snapshot.installed.length > 0 || snapshot.library.length > 0 || snapshot.history.length > 0;
    untrack(() => {
      if (hasData) {
        void updateSettings({ setup: { completed: true, skipped: true, completedAtMs: Date.now(), version: 1 } }, 'global')
          .then((updated) => (snapshot.settings = updated))
          .catch(() => {});
      } else {
        setupOpen = true;
      }
    });
  });
  // The extension catalog starts on "my languages" once the person has chosen some.
  let languagePrefApplied = false;
  $effect(() => {
    if (!languagePrefApplied && sourceLanguagePrefs.length > 0) {
      languagePrefApplied = true;
      languageFilter = 'mine';
    }
  });
  function finishSetup(updated: AppSettings) {
    snapshot.settings = updated;
    setupOpen = false;
  }
  async function changeInterfaceLanguage(value: string) {
    await setAppearance({ locale: value });
  }
  async function changeSourceLanguages(codes: string[]) {
    try {
      snapshot.settings = await updateSettings({ providerLanguages: codes }, 'global');
    } catch (e) {
      showError(e);
    }
  }
  const SOURCE_LANGUAGE_CHOICES = ['en', 'es', 'fr', 'de', 'it', 'pt', 'ru', 'ar', 'hi', 'ja', 'ko', 'zh', 'tr', 'nl', 'pl'];

  // ---- Settings search, section jump, links, shortcuts and exit confirmation (T30) ----
  let settingsQuery = $state('');
  let settingsPanels = $state<HTMLElement | null>(null);
  let settingsSections = $state<{ id: string; title: string }[]>([]);
  let settingsMatches = $state(0);
  let settingsObserver: MutationObserver | undefined;
  let legalDoc = $state<{ name: string; text: string } | null>(null);
  let projectLinks = $state<{ id: string; url: string }[]>([]);
  let shortcutsOpen = $state(false);
  let shortcutsDialog = $state<HTMLDialogElement | null>(null);
  let exitPrompt = $state<{ playing: boolean; downloads: number } | null>(null);
  let exitDialog = $state<HTMLDialogElement | null>(null);

  const LINK_LABELS: Record<string, string> = {
    project: 'This project',
    issues: 'Report a problem',
    upstream: 'Cloudstream (Android)',
    docs: 'Documentation',
    community: 'Community chat',
    'extension-policy': 'Extension policy',
  };

  function slug(text: string) {
    return text.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '');
  }

  /** Show only the panels that mention every word typed; keep a jump list of the visible ones. */
  function applySettingsFilter() {
    if (!settingsPanels) return;
    const words = settingsQuery.toLowerCase().split(/\s+/).filter(Boolean);
    const panels = [...settingsPanels.children].filter((el): el is HTMLElement => el instanceof HTMLElement && el.classList.contains('settings-panel'));
    let matches = 0;
    const sections: { id: string; title: string }[] = [];
    for (const panel of panels) {
      const heading = panel.querySelector('h2');
      const title = heading?.textContent?.trim() || 'Settings';
      if (!panel.id) panel.id = `settings-${slug(title)}`;
      if (heading && !heading.hasAttribute('tabindex')) heading.setAttribute('tabindex', '-1');
      const haystack = `${title} ${panel.dataset.keywords ?? ''} ${panel.textContent ?? ''}`.toLowerCase();
      const visible = words.every((w) => haystack.includes(w));
      panel.hidden = !visible;
      if (visible) {
        matches += 1;
        sections.push({ id: panel.id, title });
      }
    }
    settingsMatches = matches;
    settingsSections = sections;
  }

  function jumpToSetting(id: string) {
    const panel = document.getElementById(id);
    panel?.scrollIntoView({ block: 'start' });
    panel?.querySelector<HTMLElement>('h2')?.focus();
  }

  $effect(() => {
    settingsQuery;
    if (page === 'settings' && settingsPanels) void tick().then(applySettingsFilter);
  });
  $effect(() => {
    settingsObserver?.disconnect();
    if (page !== 'settings' || !settingsPanels) return;
    // Panels from other components fill in after load; keep the filter and jump list current.
    settingsObserver = new MutationObserver(() => applySettingsFilter());
    settingsObserver.observe(settingsPanels, { childList: true });
    applySettingsFilter();
    return () => settingsObserver?.disconnect();
  });

  async function loadAboutLinks() {
    if (projectLinks.length) return;
    try {
      projectLinks = await call<{ id: string; url: string }[]>('list_links');
    } catch {
      projectLinks = [];
    }
  }
  $effect(() => {
    if (page === 'settings') void untrack(() => loadAboutLinks());
  });
  async function openProjectLink(id: string) {
    try {
      await call('open_link', { id });
    } catch (e) {
      showError(e);
    }
  }
  async function showLegal(name: 'license' | 'notices' | 'third-party', label: string) {
    try {
      legalDoc = { name: label, text: await call<string>('get_legal_text', { name }) };
    } catch (e) {
      showError(e);
    }
  }

  /** Move focus to the nearest control in an arrow-key direction (remote-style navigation). */
  function spatialMove(direction: 'left' | 'right' | 'up' | 'down'): boolean {
    const current = document.activeElement as HTMLElement | null;
    const candidates = [...document.querySelectorAll<HTMLElement>('main a[href], main button:not(:disabled), aside button:not(:disabled), aside a[href], main [role="tab"], main select, main input:not([type="hidden"])')].filter(
      (el) => el.offsetParent !== null && !el.closest('[hidden]') && !el.closest('dialog:not([open])'),
    );
    const origin = current && candidates.includes(current) ? current.getBoundingClientRect() : null;
    const cx = origin ? origin.left + origin.width / 2 : 0;
    const cy = origin ? origin.top + origin.height / 2 : 0;
    let best: HTMLElement | null = null;
    let bestScore = Infinity;
    for (const el of candidates) {
      if (el === current) continue;
      const r = el.getBoundingClientRect();
      const dx = r.left + r.width / 2 - cx;
      const dy = r.top + r.height / 2 - cy;
      const along = direction === 'right' ? dx : direction === 'left' ? -dx : direction === 'down' ? dy : -dy;
      const across = direction === 'left' || direction === 'right' ? Math.abs(dy) : Math.abs(dx);
      if (origin && along <= 1) continue;
      const score = along + across * 2;
      if (score < bestScore) {
        bestScore = score;
        best = el;
      }
    }
    if (!best) return false;
    best.focus();
    best.scrollIntoView({ block: 'nearest', inline: 'nearest' });
    return true;
  }

  async function quitNow() {
    try {
      await call('quit_app');
    } catch (e) {
      showError(e);
    }
  }

  // ---- Links and files from outside the app (T27) ----
  let entryConfirm = $state<EntryAction | null>(null);
  let entryWaiting: EntryAction[] = [];
  let entryBusy = $state(false);
  let externalPlayer = $state<ExternalPlayerConfig>({ kind: 'none' });

  function providerForShared(name: string, url: string): Provider | undefined {
    const wanted = name.trim().toLowerCase();
    const byName = providers.filter((p) => !p.disabled && p.name.trim().toLowerCase() === wanted);
    if (byName.length <= 1) return byName[0];
    const host = (text: string) => {
      try {
        return new URL(text).hostname.replace(/^www\./, '').toLowerCase();
      } catch {
        return '';
      }
    };
    const matching = byName.filter((p) => host(p.mainUrl) === host(url));
    return matching.length === 1 ? matching[0] : undefined;
  }

  async function openSharedTitle(provider: Provider, url: string) {
    await openItem({ providerId: provider.id, name: url, url } as MediaItem);
  }

  /** Navigation actions run at once; anything that adds, plays or resumes waits for a click. */
  async function routeEntry(action: EntryAction) {
    if (action.type === 'search') {
      query = action.query;
      openSearch();
      await runSearch();
    } else if (action.type === 'open_title') {
      const provider = providerForShared(action.providerName, action.url);
      if (!provider) {
        message(`No installed source is called “${action.providerName}”. Install it, then open the link again.`);
        return;
      }
      await openSharedTitle(provider, action.url);
    } else if (action.type === 'open_media') {
      const provider = providers.find((p) => p.id === action.providerId && !p.disabled);
      if (!provider) {
        message('That title’s source is not available right now. Enable or install it, then open the title from your library.');
        return;
      }
      await openSharedTitle(provider, action.url);
    } else if (action.type === 'open_url') {
      const provider = providers.find((p) => !p.disabled && p.mainUrl && action.url.startsWith(p.mainUrl));
      if (provider) await openSharedTitle(provider, action.url);
      else entryWaiting.push(action);
    } else {
      entryWaiting.push(action);
    }
  }

  function showNextEntryPrompt() {
    if (!entryConfirm && entryWaiting.length) entryConfirm = entryWaiting.shift() ?? null;
  }

  async function drainEntries() {
    // Held back while a profile PIN or the profile chooser is on screen.
    if (!subscriptionProfileReady || modal) return;
    try {
      const batch = await call<{ actions: EntryAction[]; errors: string[] }>('take_entry_actions');
      if (batch.errors.length) message(batch.errors.join(' · '));
      for (const action of batch.actions) await routeEntry(action);
      showNextEntryPrompt();
    } catch {
      // Without the native app there is nothing to receive.
    }
  }

  async function confirmEntry() {
    const action = entryConfirm;
    if (!action) return;
    entryBusy = true;
    try {
      if (action.type === 'add_repository') {
        await call('add_repository', { url: action.input });
        message('Repository added.');
        await refresh();
        navigate('extensions');
      } else if (action.type === 'play_url' || action.type === 'open_url') {
        const name = action.type === 'play_url' && action.name ? action.name : 'Direct stream';
        await call('play', {
          stream: { name, url: action.url, referer: '', quality: 0, headers: {}, streamType: 'VIDEO' },
          subtitles: [],
          item: null,
          episode: null,
          resume: 0,
          profileId: activeProfile,
        });
      } else if (action.type === 'play_file') {
        await call('play_local_file', { path: action.path, profileId: activeProfile });
      } else if (action.type === 'resume_latest' && snapshot.history[0]) {
        await resume(snapshot.history[0]);
      }
    } catch (e) {
      showError(e);
    } finally {
      entryBusy = false;
      entryConfirm = null;
      showNextEntryPrompt();
    }
  }

  function cancelEntry() {
    entryConfirm = null;
    showNextEntryPrompt();
  }

  async function openInExternalPlayer(stream: Stream) {
    if (!details?.item || !selectedEpisode) return;
    try {
      const result = await call<{ program: string; warnings: string[] }>('open_in_external_player', {
        stream,
        title: `${details.item.name} — ${selectedEpisode.name}`,
        start: pendingResumePosition || 0,
        subtitles: links?.subtitles ?? [],
      });
      message(result.warnings.length ? `Opened. ${result.warnings.join(' ')}` : 'Opened in your external player.');
    } catch (e) {
      showError(e);
    }
  }

  async function downloadEpisode(episode: Episode, policy: Record<string, unknown> = {}) {
    if (!details?.item) return;
    try {
      localStorage.setItem('cloudstream.download-extension', downloadExtension);
    } catch {
      // The choice simply is not remembered.
    }
    await task('download', async () => {
      const job = await call<{ title: string } | null>('enqueue_download', {
        profileId: activeProfile,
        item: details!.item,
        episode,
        extension: downloadExtension,
        policy,
      });
      // A null reply means the folder chooser was closed without choosing a folder.
      if (job) message(`Added “${episode.name}” to Downloads.`);
    });
  }
  async function downloadEpisodes(list: Episode[]) {
    if (!details?.item || list.length === 0) return;
    try {
      localStorage.setItem('cloudstream.download-extension', downloadExtension);
    } catch {
      // The choice simply is not remembered.
    }
    await task('download', async () => {
      const result = await call<{ added: number; alreadyQueued: number; failed: string[] } | null>(
        'enqueue_downloads',
        {
          profileId: activeProfile,
          item: details!.item,
          episodes: list,
          extension: downloadExtension,
          policy: {},
        },
      );
      if (!result) return;
      const parts = [`Added ${result.added} ${result.added === 1 ? 'episode' : 'episodes'} to Downloads.`];
      if (result.alreadyQueued) parts.push(`${result.alreadyQueued} already in the queue.`);
      if (result.failed.length) parts.push(`${result.failed.length} could not be added.`);
      message(parts.join(' '));
    });
  }
  function message(text: string) {
    notice = text;
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => (notice = ''), 5000);
  }
  function showError(e: unknown) {
    error = toErrorMessage(e);
  }
  async function setPosterDensity(density: PosterDensity) {
    posterDensity = density;
    localStorage.setItem('cloudstream.poster-density', density);
    try {
      await updateSettings({ appearance: { posterDensity: density } });
    } catch (e) {
      showError(e);
    }
  }
  async function refresh(targetProfileId?: string) {
    if (!nativeAvailable()) {
      ready = true;
      return;
    }
    const pid = targetProfileId ?? activeProfile;
    snapshot = await call<Snapshot>('snapshot', { profileId: pid });
    if (snapshot.activeProfileId && !targetProfileId) {
      activeProfile = snapshot.activeProfileId;
    }
    const legacyDensity = localStorage.getItem('cloudstream.poster-density');
    if (legacyDensity === 'compact' || legacyDensity === 'comfortable') {
      posterDensity = legacyDensity;
      if (snapshot.settings?.global?.appearance?.posterDensity !== legacyDensity) {
        updateSettings({ appearance: { posterDensity: legacyDensity } }).catch(() => {});
      }
    } else if (snapshot.settings?.global?.appearance?.posterDensity) {
      posterDensity = snapshot.settings.global.appearance.posterDensity;
    }
    providers = await call<Provider[]>('list_providers');
    const savedProv =
      snapshot.settings?.profiles?.[activeProfile]?.selectedProvider ||
      (() => {
        try {
          return localStorage.getItem('cs_selected_provider_' + activeProfile);
        } catch {
          return null;
        }
      })() ||
      '';
    if (savedProv && providers.some((p) => p.id === savedProv)) {
      selectedProvider = savedProv;
    } else if (!selectedProvider && providers.length > 0) {
      selectedProvider = providers[0]?.id ?? '';
    } else if (selectedProvider && !providers.some((p) => p.id === selectedProvider)) {
      selectedProvider = providers[0]?.id ?? '';
    }
    await loadSearchHistory();
    ready = true;
  }

  function openProfileManager() {
    error = '';
    modal = 'profiles';
  }

  function startCreateProfile() {
    editingProfile = null;
    profileNameInput = '';
    profileAvatarInput = 'popcorn';
    profilePinInput = '';
    profileRemovePin = false;
    error = '';
    modal = 'profileForm';
  }

  function startEditProfile(p: Profile) {
    editingProfile = p;
    profileNameInput = p.name;
    profileAvatarInput = p.avatar;
    profilePinInput = '';
    profileRemovePin = false;
    error = '';
    modal = 'profileForm';
  }

  async function saveProfileForm() {
    if (!profileNameInput.trim()) {
      error = 'Profile name cannot be empty';
      return;
    }
    try {
      if (editingProfile) {
        await call<Profile>('update_profile', {
          id: editingProfile.id,
          input: {
            name: profileNameInput.trim(),
            avatar: profileAvatarInput,
            pin: profilePinInput.trim() || undefined,
            removePin: profileRemovePin || undefined,
          },
        });
        message('Profile updated');
      } else {
        await call<Profile>('create_profile', {
          input: {
            name: profileNameInput.trim(),
            avatar: profileAvatarInput,
            pin: profilePinInput.trim() || undefined,
          },
        });
        message('Profile created');
      }
      await refresh();
      modal = 'profiles';
    } catch (e) {
      showError(e);
    }
  }

  async function requestSwitchProfile(targetId: string) {
    const target = snapshot.profiles.find((p) => p.id === targetId);
    if (!target) return;
    if (target.id === activeProfile && modal !== 'profiles') {
      modal = null;
      return;
    }
    if (target.hasPin) {
      pendingSwitchProfileId = target.id;
      pinInput = '';
      pinError = '';
      modal = 'profilePin';
    } else {
      await performSwitch(target.id, null);
    }
  }

  async function submitProfilePin() {
    if (!pendingSwitchProfileId) return;
    await performSwitch(pendingSwitchProfileId, pinInput.trim());
  }

  async function performSwitch(targetId: string, pin: string | null) {
    try {
      await call('switch_profile', { targetProfileId: targetId, pin: pin || undefined });
      subscriptionProfileReady = false;
      activeProfile = targetId;
      pendingSwitchProfileId = null;
      pinInput = '';
      pinError = '';
      modal = null;
      await refresh(targetId);
      subscriptionProfileReady = true;
      if (selectedProvider) {
        await chooseProvider(selectedProvider, true);
      }
      message(`Switched profile to ${snapshot.profiles.find((p) => p.id === targetId)?.name ?? targetId}`);
    } catch (e) {
      pinError = toErrorMessage(e);
      showError(e);
    }
  }

  function startDeleteProfile(id: string) {
    if (snapshot.profiles.length <= 1) {
      showError('Cannot delete the only remaining profile');
      return;
    }
    deleteTargetProfileId = id;
    modal = 'deleteProfile';
  }

  async function executeDeleteProfile() {
    if (!deleteTargetProfileId) return;
    const targetId = deleteTargetProfileId;
    try {
      await call('delete_profile', { id: targetId });
      deleteTargetProfileId = null;
      if (activeProfile === targetId) {
        subscriptionProfileReady = false;
        activeProfile = 'default';
      }
      message('Profile deleted');
      await refresh();
      modal = 'profiles';
    } catch (e) {
      showError(e);
    }
  }

  async function updateAutoSelectProfile(enabled: boolean) {
    try {
      await updateSettings({ autoSelectProfile: enabled }, 'global');
      await refresh();
      message(`Auto-select profile ${enabled ? 'enabled' : 'disabled'}`);
    } catch (e) {
      showError(e);
    }
  }

  async function updateDefaultProfile(profileId: string) {
    try {
      await updateSettings({ defaultProfileId: profileId }, 'global');
      await refresh();
      message('Default profile updated');
    } catch (e) {
      showError(e);
    }
  }
  async function task(label: string, operation: () => Promise<void>) {
    if (busy) return;
    busy = label;
    error = '';
    try {
      await operation();
    } catch (e) {
      showError(e);
    } finally {
      busy = '';
    }
  }
  async function chooseProvider(id: string, forceRefresh = false) {
    selectedProvider = id;
    try {
      localStorage.setItem('cs_selected_provider_' + activeProfile, id);
      call('update_settings', {
        patch: { selectedProvider: id },
        profileId: activeProfile,
      }).catch(() => {});
    } catch {
      // ignore
    }
    searchActive = false;
    query = '';
    featuredIndex = 0;
    contentFilter = 'All';
    expandedCategory = null;
    categoryPages = {};
    ++searchVersion;
    const version = ++providerVersion;
    loadingContent = false;
    if (!id || !providers.find((p) => p.id === id)?.hasMainPage) {
      home = { rows: [] };
      return;
    }
    if (!home.rows.length || forceRefresh) {
      loadingContent = true;
    }
    try {
      const result = await call<Home>('provider_home', {
        providerId: id,
        page: 1,
        forceRefresh,
      });
      if (version === providerVersion) home = result;
    } catch (e) {
      if (version === providerVersion) showError(e);
    } finally {
      if (version === providerVersion) loadingContent = false;
    }
  }
  async function moreCategories() {
    if (loadingCategories || !home.hasMoreSections) return;
    const providerId = selectedProvider;
    loadingCategories = true;
    try {
      const next = await call<Home>('provider_home', {
        providerId,
        page: 1,
        section: home.nextSection ?? 1,
      });
      if (selectedProvider === providerId && !searchActive)
        home = { ...next, rows: [...home.rows, ...next.rows] };
    } catch (e) {
      showError(e);
    } finally {
      loadingCategories = false;
    }
  }
  function navigate(next: Page) {
    searchAbortController?.abort();
    detailsAbortController?.abort();
    resolveAbortController?.abort();
    spotlightAbortController?.abort();
    page = next;
    searchView = false;
    details = null;
    links = null;
    error = '';
    if (next === 'discover' && selectedProvider && home.rows.length === 0)
      void chooseProvider(selectedProvider);
  }
  function openSearch() {
    searchAbortController?.abort();
    detailsAbortController?.abort();
    resolveAbortController?.abort();
    spotlightAbortController?.abort();
    page = 'discover';
    details = null;
    searchView = true;
    error = '';
    void tick().then(() => searchInput?.focus());
  }
  async function saveItem(item: MediaItem) {
    await task('bookmark', async () => {
      const saved = snapshot.library.find(
        (s) => s.item.providerId === item.providerId && s.item.url === item.url,
      );
      if (saved) await call('remove_bookmark', { id: saved.id });
      else await call('bookmark', { item });
      await refresh();
      message(saved ? 'Removed from your library.' : 'Saved to your library.');
    });
  }
  async function moreTitles(index: number) {
    await task('category', async () => {
      const current = home.rows[index];
      if (!current) return;
      const rowKey = current.rowId || String(index);
      const nextPage = (categoryPages[rowKey] ?? current.page ?? 1) + 1;
      const section = current.sectionIndex ?? index;
      const result = await call<Home>('provider_home', {
        providerId: selectedProvider,
        page: nextPage,
        section,
      });
      categoryPages[rowKey] = nextPage;
      const next =
        result.rows.find((r) => r.rowId === current.rowId) ??
        result.rows.find((r) => r.name === current.name) ??
        result.rows[0];
      const existing = new Set(current.items.map((item) => item.url));
      const added = next?.items.filter((item) => !existing.has(item.url)) ?? [];
      home.rows[index] = {
        ...current,
        items: [...current.items, ...added],
        hasNext: added.length > 0 && Boolean(next?.hasNext),
        page: nextPage,
      };
    });
  }
  async function addSample() {
    await task('sample', async () => {
      await call('install_sample');
      await refresh();
      page = 'discover';
      await chooseProvider(selectedProvider);
      message('Blender Open Movies is ready to browse.');
    });
  }
  async function addTorrentio() {
    await task('torrentio', async () => {
      await call('install_torrentio');
      await refresh();
      page = 'discover';
      await chooseProvider('bundled:Torrentio');
      message('Torrentio is installed and ready.');
    });
  }
  async function addRepository(event: SubmitEvent) {
    event.preventDefault();
    await task('repository', async () => {
      await call('add_repository', { url: repoUrl });
      await refresh();
      modal = null;
      page = 'extensions';
      extensionTab = 'browse';
      repoUrl = '';
      message('Repository added. Choose an extension to install.');
    });
  }
  async function install(repositoryUrl: string, entry: PluginEntry) {
    await task(entry.internalName, async () => {
      const result = await call<InstalledPlugin>('install_extension', {
        repositoryUrl,
        internalName: entry.internalName,
      });
      await refresh();
      const state = snapshot.extensionStates?.find((s) => s.pluginId === result.id);
      message(
        state?.loaded || result.runtime === 'jvm'
          ? `${entry.name} is ready.`
          : `${entry.name} downloaded, but cannot run on this desktop yet. See Installed for details.`,
      );
    });
  }
  async function removePlugin(plugin: InstalledPlugin) {
    await task(plugin.id, async () => {
      await call('remove_extension', { id: plugin.id });
      await refresh();
      home = { rows: [] };
      message(`${plugin.entry.name} removed.`);
    });
  }
  async function copyShareLink(repo: Repository) {
    const link = `https://cs.repo/?${repo.url}`;
    try {
      if (navigator?.clipboard?.writeText) {
        await navigator.clipboard.writeText(link);
      }
    } catch {
      // Ignored if browser permissions restrict clipboard
    }
    message('Repository share link copied.');
  }
  function openRename(repo: Repository) {
    renameRepoUrl = repo.url;
    renameValue = repo.customName ?? repo.manifest.name;
    modal = 'rename';
  }
  async function submitRename(event?: SubmitEvent) {
    event?.preventDefault();
    if (!renameValue.trim()) return;
    await task('rename', async () => {
      await call('rename_repository', { url: renameRepoUrl, name: renameValue.trim() });
      await refresh();
      modal = null;
      message('Repository renamed.');
    });
  }
  async function submitLocalInstall(event?: SubmitEvent) {
    event?.preventDefault();
    if (!localPackagePath.trim()) return;
    await task('localInstall', async () => {
      await call('install_local_extension', { path: localPackagePath.trim() });
      await refresh();
      modal = null;
      localPackagePath = '';
      message('Local extension installed.');
    });
  }
  async function toggleProvider(provider: Provider) {
    await task(`provider-${provider.id}`, async () => {
      await call('set_provider_enabled', {
        providerId: provider.id,
        enabled: Boolean(provider.disabled),
      });
      await refresh();
      message(`Provider ${provider.disabled ? 'enabled' : 'disabled'}.`);
    });
  }
  function openClone(provider: Provider) {
    cloneParentProviderId = provider.id;
    cloneName = `${provider.name} (Clone)`;
    cloneUrl = provider.mainUrl;
    modal = 'clone';
  }
  async function submitClone(event?: SubmitEvent) {
    event?.preventDefault();
    if (!cloneName.trim() || !cloneUrl.trim()) return;
    await task('clone', async () => {
      const slug = cloneName.trim().toLowerCase().replace(/[^a-z0-9]+/g, '-');
      const cloneConfig: ProviderCloneConfig = {
        id: `clone::${cloneParentProviderId}::${slug}`,
        parentProviderId: cloneParentProviderId,
        name: cloneName.trim(),
        customUrl: cloneUrl.trim(),
        enabled: true,
      };
      await call('configure_provider_clone', { clone: cloneConfig });
      await refresh();
      modal = null;
      message('Provider clone configured.');
    });
  }
  async function checkForUpdates() {
    await task('checkUpdates', async () => {
      updates = await call<ExtensionUpdate[]>('check_for_updates');
      const count = updates.filter((u) => u.hasUpdate).length;
      if (count > 0) {
        message(`${count} extension update(s) available.`);
      } else {
        message('All extensions are up to date.');
      }
    });
  }
  async function updateAll() {
    await task('updateAll', async () => {
      const results = await call<BulkUpdateResult[]>('update_all_extensions');
      await refresh();
      updates = await call<ExtensionUpdate[]>('check_for_updates');
      const successful = results.filter((r) => r.success).length;
      const failed = results.filter((r) => !r.success);
      if (failed.length > 0) {
        message(
          `Updated ${successful} extension(s). ${failed.length} failed: ${failed.map((f) => f.name).join(', ')}`,
        );
      } else {
        message(`Successfully updated ${successful} extension(s).`);
      }
    });
  }
  async function runProviderTest(provider: Provider) {
    testingProvider = provider;
    testReport = null;
    modal = 'test';
    testAbortController?.abort();
    testAbortController = new AbortController();
    const currentSignal = testAbortController.signal;

    await task('testing', async () => {
      try {
        testReport = await call<ProviderTestReport>(
          'test_provider',
          { providerId: provider.id },
          { signal: currentSignal },
        );
      } catch (e) {
        if (!currentSignal.aborted) {
          error = toErrorMessage(e);
        }
      }
    });
  }
  function cancelProviderTest() {
    testAbortController?.abort();
    testAbortController = null;
    modal = null;
    message('Provider test cancelled.');
  }
  async function loadSearchHistory() {
    if (!nativeAvailable()) return;
    try {
      searchHistory = await call<SearchHistoryEntry[]>('get_search_history', {
        profileId: activeProfile,
      });
    } catch {
      searchHistory = [];
    }
  }

  async function addSearchHistory(q: string) {
    if (!nativeAvailable() || !q.trim()) return;
    try {
      const entry = await call<SearchHistoryEntry>('add_search_history', {
        profileId: activeProfile,
        query: q.trim(),
      });
      searchHistory = [
        entry,
        ...searchHistory.filter((e) => e.query.toLowerCase() !== q.trim().toLowerCase()),
      ].slice(0, 50);
    } catch {
      // ignore
    }
  }

  async function deleteSearchHistory(q: string) {
    if (!nativeAvailable()) return;
    try {
      await call('delete_search_history_entry', {
        profileId: activeProfile,
        query: q,
      });
      searchHistory = searchHistory.filter((e) => e.query !== q);
    } catch (e) {
      showError(e);
    }
  }

  async function clearSearchHistory() {
    if (!nativeAvailable()) return;
    try {
      await call('clear_search_history', { profileId: activeProfile });
      searchHistory = [];
    } catch (e) {
      showError(e);
    }
  }

  function onSearchInput(val: string) {
    query = val;
    if (suggestionDebounceTimer) {
      clearTimeout(suggestionDebounceTimer);
      suggestionDebounceTimer = null;
    }
    suggestionAbortController?.abort();

    if (page === 'extensions') {
      repoFilter = query;
      return;
    }
    const trimmed = query.trim();
    if (!trimmed) {
      showSuggestionsDropdown = searchHistory.length > 0;
      searchSuggestions = [];
      return;
    }
    const suggestionsEnabled = snapshot.settings?.global?.appearance?.searchSuggestions ?? true;
    if (!suggestionsEnabled) {
      showSuggestionsDropdown = false;
      searchSuggestions = [];
      return;
    }
    if (trimmed.length < 2) {
      searchSuggestions = [];
      showSuggestionsDropdown = false;
      return;
    }
    suggestionDebounceTimer = setTimeout(async () => {
      suggestionAbortController = new AbortController();
      const signal = suggestionAbortController.signal;
      try {
        const provId = selectedProvider || undefined;
        const res = await call<string[]>(
          'search_suggestions',
          { profileId: activeProfile, query: trimmed, providerId: provId },
          { signal },
        );
        if (!signal.aborted) {
          searchSuggestions = res;
          showSuggestionsDropdown = res.length > 0;
        }
      } catch {
        // cancelled or error
      }
    }, 250);
  }

  const appearance = $derived(snapshot.settings?.global?.appearance ?? ({ posterDensity: 'comfortable' } as AppearanceSettings));
  const ACCENTS: { id: AccentName; label: string; swatch: string }[] = [
    { id: 'indigo', label: 'Indigo', swatch: '#3d50fa' },
    { id: 'teal', label: 'Teal', swatch: '#0f766e' },
    { id: 'rose', label: 'Rose', swatch: '#be123c' },
    { id: 'amber', label: 'Amber', swatch: '#b45309' },
    { id: 'green', label: 'Green', swatch: '#15803d' },
    { id: 'violet', label: 'Violet', swatch: '#7c3aed' },
  ];
  // Saved appearance choices are applied to the whole window through root attributes.
  $effect(() => {
    const root = document.documentElement;
    root.dataset.theme = appearance.theme ?? 'dark';
    root.dataset.accent = appearance.accent ?? 'indigo';
    root.dataset.motion = appearance.motion ?? 'system';
    root.dataset.nav = appearance.navigation ?? 'pointer';
    root.style.setProperty('--poster-scale', String((appearance.posterScale ?? 100) / 100));
  });
  async function setAppearance(patch: Partial<AppearanceSettings>) {
    try {
      const updated = await updateSettings({ appearance: patch }, 'global');
      snapshot.settings = updated;
    } catch (e) {
      showError(e);
    }
  }
  async function updateSearchSuggestions(enabled: boolean) {
    if (suggestionDebounceTimer) {
      clearTimeout(suggestionDebounceTimer);
      suggestionDebounceTimer = null;
    }
    suggestionAbortController?.abort();
    if (snapshot.settings?.global?.appearance) {
      snapshot.settings.global.appearance.searchSuggestions = enabled;
    }
    if (!enabled) {
      showSuggestionsDropdown = false;
      searchSuggestions = [];
    }
    try {
      const updated = await updateSettings({ appearance: { searchSuggestions: enabled } });
      if (snapshot.settings) {
        snapshot.settings = updated;
      }
    } catch (e) {
      showError(e);
    }
  }

  const playbackPolicy = $derived(
    snapshot.settings?.global?.playback ?? {
      autoNext: true,
      startPaused: false,
      completionThreshold: 0.9,
      resumeThresholdSeconds: 5.0,
      prefetchNext: true,
    },
  );

  async function updatePlaybackPolicy(patch: Partial<PlaybackPolicy>) {
    try {
      const current = snapshot.settings?.global?.playback ?? {
        autoNext: true,
        startPaused: false,
        completionThreshold: 0.9,
        resumeThresholdSeconds: 5.0,
        prefetchNext: true,
      };
      const merged = { ...current, ...patch };
      const updated = await updateSettings({ playback: merged }, 'global');
      if (snapshot.settings) {
        snapshot.settings = updated;
      }
      message('Playback preferences updated.');
    } catch (e) {
      showError(e);
    }
  }

  function syncSearchItems() {
    const combined: MediaItem[] = [];
    let hasAnyNext = false;
    for (const state of Object.values(sourceSearchStates)) {
      combined.push(...state.items);
      if (state.hasNext) hasAnyNext = true;
    }
    searchItems = combined;
    searchHasNext = hasAnyNext;
  }

  function filterMediaItem(item: MediaItem): boolean {
    if (searchFilters.source !== 'all' && item.providerId !== searchFilters.source) {
      return false;
    }
    if (searchFilters.type !== 'all') {
      const itemType = (item.mediaType || '').toLowerCase();
      if (itemType !== searchFilters.type.toLowerCase()) return false;
    }
    if (searchFilters.quality !== 'all') {
      const itemQuality = (item.quality || '').toLowerCase();
      if (itemQuality !== searchFilters.quality.toLowerCase()) return false;
    }
    if (searchFilters.dubStatus !== 'all') {
      const itemDub = (item.dubStatus || '').toLowerCase();
      if (itemDub !== searchFilters.dubStatus.toLowerCase()) return false;
    }
    return true;
  }

  const filteredSearchItems = $derived(searchItems.filter(filterMediaItem));

  async function runSearch(event?: SubmitEvent, targetProviderId?: string, nextPage = 1) {
    event?.preventDefault();
    spotlightAbortController?.abort();
    if (page === 'extensions') {
      repoFilter = query.trim();
      extensionTab = 'browse';
      return;
    }
    showSuggestionsDropdown = false;
    const q = query.trim();
    if (!q) {
      searchActive = false;
      return;
    }

    const enabledProviders = providers.filter((p) => !p.disabled);
    let targets: Provider[] = [];

    if (targetProviderId) {
      const found = providers.find((p) => p.id === targetProviderId);
      if (found) targets = [found];
    } else if (searchMode === 'custom' && selectedSearchProviders.length > 0) {
      targets = enabledProviders.filter((p) => selectedSearchProviders.includes(p.id));
    } else if (searchMode === 'all') {
      targets = enabledProviders;
    } else if (selectedProvider) {
      const found = providers.find((p) => p.id === selectedProvider);
      if (found) targets = [found];
    }

    if (targets.length === 0 && selectedProvider) {
      const found = providers.find((p) => p.id === selectedProvider);
      if (found) targets = [found];
    }

    if (targets.length === 0) {
      searchActive = false;
      error = 'No active media sources available to search. Open Extensions to connect or enable sources.';
      return;
    }

    void addSearchHistory(q);

    searchAbortController?.abort();
    searchAbortController = new AbortController();
    const currentSignal = searchAbortController.signal;

    const version = ++searchVersion;
    ++providerVersion;
    submittedQuery = q;
    searchActive = true;
    loadingContent = true;
    error = '';
    page = 'discover';
    searchView = true;
    details = null;

    if (nextPage === 1) {
      const states: Record<string, SourceSearchState> = {};
      for (const p of targets) {
        states[p.id] = {
          providerId: p.id,
          providerName: p.name,
          loading: true,
          error: null,
          items: [],
          page: 1,
          hasNext: false,
        };
      }
      sourceSearchStates = states;
    }

    const promises = targets.map(async (p) => {
      try {
        const result = await call<SearchResult>(
          'search',
          { providerId: p.id, query: q, page: nextPage },
          { signal: currentSignal, requestId: `search-${version}-${p.id}` },
        );
        if (version === searchVersion && !currentSignal.aborted) {
          const prevItems = nextPage === 1 ? [] : (sourceSearchStates[p.id]?.items ?? []);
          sourceSearchStates[p.id] = {
            providerId: p.id,
            providerName: p.name,
            loading: false,
            error: null,
            items: [...prevItems, ...result.items],
            page: nextPage,
            hasNext: result.hasNext,
          };
          syncSearchItems();
        }
      } catch (e) {
        if (currentSignal.aborted) return;
        if (version === searchVersion) {
          sourceSearchStates[p.id] = {
            providerId: p.id,
            providerName: p.name,
            loading: false,
            error: toErrorMessage(e),
            items: sourceSearchStates[p.id]?.items ?? [],
            page: sourceSearchStates[p.id]?.page ?? 1,
            hasNext: sourceSearchStates[p.id]?.hasNext ?? false,
          };
        }
      }
    });

    await Promise.allSettled(promises);
    if (version === searchVersion) {
      loadingContent = false;
      syncSearchItems();
    }
  }

  async function loadMoreSource(providerId: string) {
    const state = sourceSearchStates[providerId];
    if (!state || state.loading || !state.hasNext) return;

    state.loading = true;
    const nextPage = state.page + 1;
    try {
      const result = await call<SearchResult>(
        'search',
        {
          providerId,
          query: submittedQuery,
          page: nextPage,
        },
        {
          signal: searchAbortController?.signal,
          requestId: `more-${searchVersion}-${providerId}-${nextPage}`,
        },
      );
      sourceSearchStates[providerId] = {
        ...state,
        loading: false,
        items: [...state.items, ...result.items],
        page: nextPage,
        hasNext: result.hasNext,
      };
      syncSearchItems();
    } catch (e) {
      sourceSearchStates[providerId] = {
        ...state,
        loading: false,
        error: toErrorMessage(e),
      };
    }
  }

  async function loadMoreAll() {
    const sourcesWithMore = Object.values(sourceSearchStates).filter(
      (s) => s.hasNext && !s.loading,
    );
    await Promise.allSettled(sourcesWithMore.map((s) => loadMoreSource(s.providerId)));
  }

  async function retrySource(providerId: string) {
    const state = sourceSearchStates[providerId];
    if (!state) return;
    state.loading = true;
    state.error = null;
    try {
      const result = await call<SearchResult>(
        'search',
        {
          providerId,
          query: submittedQuery,
          page: state.page,
        },
        {
          signal: searchAbortController?.signal,
          requestId: `retry-${searchVersion}-${providerId}-${state.page}`,
        },
      );
      sourceSearchStates[providerId] = {
        ...state,
        loading: false,
        items: result.items,
        hasNext: result.hasNext,
      };
      syncSearchItems();
    } catch (e) {
      sourceSearchStates[providerId] = {
        ...state,
        loading: false,
        error: toErrorMessage(e),
      };
    }
  }

  function toggleSearchProvider(providerId: string) {
    if (selectedSearchProviders.includes(providerId)) {
      selectedSearchProviders = selectedSearchProviders.filter((id) => id !== providerId);
    } else {
      selectedSearchProviders = [...selectedSearchProviders, providerId];
    }
  }
  let pendingResumePosition = $state<number>(0);

  const availableSeasons = $derived.by(() => {
    if (!details) return [];
    const seasons = new Set<number>();
    if (details.seasons) {
      for (const s of details.seasons) {
        if (s.season !== undefined) seasons.add(s.season);
      }
    }
    for (const ep of details.episodes) {
      if (ep.season !== undefined) seasons.add(ep.season);
    }
    return Array.from(seasons).sort((a, b) => a - b);
  });

  const availableDubGroups = $derived.by(() => {
    if (!details) return [];
    const groups = new Set<string>();
    if (details.dubGroups) {
      for (const key of Object.keys(details.dubGroups)) {
        groups.add(key);
      }
    }
    for (const ep of details.episodes) {
      if (ep.dubStatus && ep.dubStatus !== 'none') {
        groups.add(ep.dubStatus === 'subbed' ? 'Sub' : ep.dubStatus === 'dubbed' ? 'Dub' : 'Both');
      }
    }
    return Array.from(groups);
  });

  const filteredEpisodes = $derived.by(() => {
    if (!details) return [];
    let list = details.episodes;
    if (selectedDubGroup !== 'all') {
      if (details.dubGroups && details.dubGroups[selectedDubGroup]) {
        list = details.dubGroups[selectedDubGroup];
      } else {
        list = list.filter((ep) => {
          if (selectedDubGroup === 'Sub') return ep.dubStatus === 'subbed' || ep.dubStatus === 'both';
          if (selectedDubGroup === 'Dub') return ep.dubStatus === 'dubbed' || ep.dubStatus === 'both';
          return true;
        });
      }
    }
    if (selectedSeason !== 'all') {
      list = list.filter((ep) => (ep.season ?? 1) === selectedSeason);
    }
    const cloned = [...list];
    if (episodeSortOrder === 'desc') {
      cloned.reverse();
    }
    return cloned;
  });

  const EPISODES_PER_CHUNK = 50;
  const episodeRangeCount = $derived(Math.ceil(filteredEpisodes.length / EPISODES_PER_CHUNK));
  const pagedEpisodes = $derived.by(() => {
    if (filteredEpisodes.length <= EPISODES_PER_CHUNK) return filteredEpisodes;
    const start = episodeRangeIndex * EPISODES_PER_CHUNK;
    return filteredEpisodes.slice(start, start + EPISODES_PER_CHUNK);
  });

  const latestWatchProgress = $derived.by(() => {
    if (!episodeProgresses.length) return null;
    const withProgress = episodeProgresses.filter((p) => p.positionSeconds > 5);
    if (!withProgress.length) return null;
    return withProgress.sort((a, b) => b.updatedAtMs - a.updatedAtMs)[0];
  });

  const resumeEpisode = $derived.by(() => {
    if (!latestWatchProgress || !details) return null;
    return (
      details.episodes.find((ep, i) => {
        const id = ep.id ?? `${details!.item.mediaId}::s${ep.season ?? 1}e${ep.episode ?? i + 1}`;
        return id === latestWatchProgress.episodeId || ep.data === latestWatchProgress.episodeId;
      }) ?? details.episodes[0]
    );
  });

  function sanitizePlot(text?: string | null): string {
    if (!text) return 'This source did not provide a description.';
    return text.replace(/<[^>]*>?/gm, '').trim();
  }

  function formatRuntime(seconds?: number): string {
    if (!seconds || seconds <= 0) return '';
    const mins = Math.round(seconds / 60);
    if (mins >= 60) {
      const h = Math.floor(mins / 60);
      const m = mins % 60;
      return m > 0 ? `${h}h ${m}m` : `${h}h`;
    }
    return `${mins}m`;
  }

  function formatScore(score?: number, rating?: number): string {
    const val = score ?? rating;
    if (val === undefined || val === null || val <= 0) return '';
    return (val > 10 ? (val / 10).toFixed(1) : val.toFixed(1)) + '/10';
  }

  function closeDetails() {
    details = null;
    links = null;
    selectedEpisode = null;
    selectedSeason = 'all';
    selectedDubGroup = 'all';
    episodeRangeIndex = 0;
    episodeSortOrder = 'asc';
    void tick().then(() => {
      if (lastFocusedElement && document.contains(lastFocusedElement)) {
        lastFocusedElement.focus();
      } else if (lastFocusedMediaUrl) {
        const el = document.querySelector<HTMLElement>(`[data-media-url="${CSS.escape(lastFocusedMediaUrl)}"]`);
        el?.focus();
      }
      lastFocusedElement = null;
      lastFocusedMediaUrl = null;
    });
  }

  function shareMedia() {
    if (!details) return;
    const itemUrl = details.item.url;
    const shareText = `${details.item.name} - ${itemUrl}`;
    if (typeof navigator !== 'undefined' && navigator.clipboard) {
      navigator.clipboard.writeText(shareText).then(() => {
        message('Title and source link copied to clipboard.');
      }).catch(() => {
        message(`Source: ${itemUrl}`);
      });
    } else {
      message(`Source: ${itemUrl}`);
    }
  }

  async function playTrailer(trailer: TrailerData) {
    if (!trailer.extractorUrl) {
      message('Trailer URL not available.');
      return;
    }
    await task('play', async () => {
      try {
        const stream: Stream = {
          name: `${details?.item.name ?? 'Media'} - Trailer`,
          url: trailer.extractorUrl,
          referer: trailer.referer ?? '',
          quality: 0,
          headers: trailer.headers ?? {},
          streamType: 'VIDEO',
        };
        const initial = await call<PlayerState | string>('play', {
          stream,
          subtitles: [],
          item: details?.item ?? null,
          episode: null,
          resume: 0,
          profileId: activeProfile,
        });
        if (initial && typeof initial === 'object' && 'active' in initial && initial.active) {
          playerState = initial;
        } else if (!playerState) {
          playerState = {
            sessionId: typeof initial === 'string' ? initial : undefined,
            active: true,
            playing: true,
            paused: false,
            position: 0,
            duration: 0,
            volume: 100,
            speed: 1,
            itemName: `${details?.item.name ?? 'Media'} - Trailer`,
            episodeName: null,
            mediaId: details?.item.url ?? null,
            episodeId: null,
            profileId: activeProfile,
          };
        }
        message('Trailer playing in mpv.');
      } catch (e) {
        message('Could not play trailer: ' + toErrorMessage(e));
      }
    });
  }

  async function openItem(item: MediaItem) {
    if (typeof document !== 'undefined' && document.activeElement instanceof HTMLElement) {
      lastFocusedElement = document.activeElement;
    }
    lastFocusedMediaUrl = item.url ?? null;
    detailsAbortController?.abort();
    detailsAbortController = new AbortController();
    const currentSignal = detailsAbortController.signal;

    selectedSeason = 'all';
    selectedDubGroup = 'all';
    episodeRangeIndex = 0;
    episodeSortOrder = 'asc';

    await task('details', async () => {
      details = null;
      links = null;
      selectedEpisode = null;
      episodeProgresses = [];
      details = await call<Details>(
        'load_details',
        { providerId: item.providerId, url: item.url, profileId: activeProfile },
        { signal: currentSignal },
      );
      if (details?.item) {
        await loadEpisodeProgresses();
      }
    });
  }
  async function resolve(episode: Episode, resume = 0) {
    if (!details) return;
    pendingResumePosition = resume;
    const item = details.item;
    resolveAbortController?.abort();
    resolveAbortController = new AbortController();
    const currentSignal = resolveAbortController.signal;

    await task('resolve', async () => {
      selectedEpisode = episode;
      links = null;
      rankedStreams = [];
      const resolved = await call<Links>(
        'resolve_links',
        {
          providerId: item.providerId,
          data: episode.data,
        },
        { signal: currentSignal },
      );
      const matchingSession = playerState?.active && episode.id && playerState.episodeId === episode.id
        ? playerState.sessionId : undefined;
      const ranked = await call<RankedStream[]>('rank_streams', { streams: resolved.streams, profileId: activeProfile, sessionId: matchingSession });
      if (currentSignal.aborted) return;
      links = resolved;
      rankedStreams = ranked;
      if (!resolved.streams.length) message('This extension returned no playable streams.');
      else if (!ranked.some((entry) => entry.usable)) message('No streams match your source preferences.');
    });
  }
  async function playStream(stream: Stream, resume = pendingResumePosition) {
    // Older player states may lack stable IDs; only carry the position when both title and
    // episode match, rather than leaking the previous episode's position into a new one.
    const sameLegacyEpisode = !playerState?.episodeId && playerState?.active &&
      !!selectedEpisode?.name && playerState.episodeName === selectedEpisode.name &&
      !!details?.item.name && playerState.itemName === details.item.name;
    const resumePosition = sameLegacyEpisode ? (playerState?.position ?? resume) : resume;
    await task('play', async () => {
      if (playerState?.active && selectedEpisode?.id && playerState.episodeId === selectedEpisode.id) {
        await call('switch_stream', { stream, subtitles: links?.subtitles ?? [] });
        return;
      }
      let queue: PlaybackQueue | null = null;
      if (details?.item && filteredEpisodes.length > 0 && selectedEpisode) {
        const curIdx = filteredEpisodes.findIndex(
          (e) => (e.id && e.id === selectedEpisode?.id) || (e.data === selectedEpisode?.data) || (e.name === selectedEpisode?.name)
        );
        queue = {
          profileId: activeProfile,
          media: details.item,
          episodes: filteredEpisodes,
          currentIndex: curIdx >= 0 ? curIdx : 0,
          dubStatus: selectedDubGroup !== 'all' ? (selectedDubGroup.toLowerCase() as any) : undefined,
          season: selectedSeason !== 'all' ? selectedSeason : undefined,
        };
      }
      const initial = await call<PlayerState | string>('play', {
        stream,
        mirrors: links?.streams ?? [],
        subtitles: links?.subtitles ?? [],
        item: details?.item ?? null,
        episode: selectedEpisode,
        resume: resumePosition,
        profileId: activeProfile,
        queue,
      });
      if (initial && typeof initial === 'object' && 'active' in initial && initial.active) {
        playerState = initial;
      } else if (!playerState) {
        playerState = {
          sessionId: typeof initial === 'string' ? initial : undefined,
          active: true,
          playing: true,
          paused: false,
          position: resumePosition,
          duration: 0,
          volume: 100,
          speed: 1,
          itemName: details?.item.name ?? 'Direct stream',
          episodeName: selectedEpisode?.name ?? null,
          mediaId: details?.item.url ?? null,
          episodeId: selectedEpisode?.name ?? null,
          profileId: activeProfile,
          hasNext: queue ? queue.currentIndex + 1 < queue.episodes.length : false,
          hasPrev: queue ? queue.currentIndex > 0 : false,
          queueIndex: queue ? queue.currentIndex : undefined,
          queueCount: queue ? queue.episodes.length : undefined,
        };
      }
      modal = null;
      message('Playback opened in mpv.');
      await refresh();
    });
  }
  async function directPlay(event: SubmitEvent) {
    event.preventDefault();
    const stream = {
      name: 'Direct stream',
      url: directUrl,
      referer: '',
      quality: 0,
      headers: {},
      streamType: 'VIDEO',
    };
    await task('play', async () => {
      const initial = await call<PlayerState | string>('play', {
        stream,
        subtitles: [],
        item: null,
        episode: null,
        resume: 0,
        profileId: activeProfile,
      });
      if (initial && typeof initial === 'object' && 'active' in initial && initial.active) {
        playerState = initial;
      } else if (!playerState) {
        playerState = {
          sessionId: typeof initial === 'string' ? initial : undefined,
          active: true,
          playing: true,
          paused: false,
          position: 0,
          duration: 0,
          volume: 100,
          speed: 1,
          itemName: 'Direct stream',
          profileId: activeProfile,
        };
      }
      modal = null;
      message('Playback opened in mpv.');
    });
  }
  async function loadEpisodeProgresses() {
    if (!details?.item) {
      episodeProgresses = [];
      return;
    }
    try {
      const mediaId = details.item.mediaId ?? `${details.item.providerId}::${details.item.url}`;
      episodeProgresses = await call<WatchProgress[]>('get_media_watch_progress', {
        mediaId,
      });
    } catch {
      episodeProgresses = [];
    }
  }

  function isEpisodeWatched(ep: Episode, idx: number): boolean {
    if (!episodeProgresses.length) return false;
    return episodeProgresses.some((p) => {
      if (ep.id && p.episodeId === ep.id) return p.isWatched;
      if (ep.id && p.episodeId.endsWith(`::${ep.id}`)) return p.isWatched;
      if (ep.data && p.episodeId.includes(ep.data) && ep.data.length > 2) return p.isWatched;
      if (p.episodeId.includes(`s${ep.season ?? 1}e${ep.episode ?? idx + 1}`)) return p.isWatched;
      return false;
    });
  }

  async function toggleEpisodeWatched(ep: Episode, idx: number) {
    if (!details) return;
    const mediaId = details.item.mediaId ?? `${details.item.providerId}::${details.item.url}`;
    const targetId = ep.id ?? `${mediaId}::s${ep.season ?? 1}e${ep.episode ?? idx + 1}`;
    const currentlyWatched = isEpisodeWatched(ep, idx);
    await call('mark_episode_watched', {
      mediaId,
      episodeId: targetId,
      isWatched: !currentlyWatched,
    });
    await loadEpisodeProgresses();
    await refresh();
    message(!currentlyWatched ? 'Marked as watched.' : 'Marked as unwatched.');
  }

  async function markUpToEpisode(ep: Episode, idx: number) {
    if (!details) return;
    const mediaId = details.item.mediaId ?? `${details.item.providerId}::${details.item.url}`;
    const targetId = ep.id ?? `${mediaId}::s${ep.season ?? 1}e${ep.episode ?? idx + 1}`;
    await call('mark_episodes_up_to', {
      mediaId,
      targetEpisodeId: targetId,
      episodes: details.episodes,
    });
    await loadEpisodeProgresses();
    await refresh();
    message('Marked watched up to this episode.');
  }

  async function updateWatchStatus(status: WatchStatus) {
    if (!details) return;
    await task('bookmark', async () => {
      await call('set_library_status', {
        item: details!.item,
        status,
      });
      await refresh();
      message(`Watch status updated to ${status.replace(/_/g, ' ')}.`);
    });
  }

  async function toggleFavorite() {
    if (!details) return;
    await task('bookmark', async () => {
      const nextFav = !detailFavorite;
      await call('set_library_favorite', {
        item: details!.item,
        isFavorite: nextFav,
      });
      await refresh();
      message(nextFav ? 'Added to favorites.' : 'Removed from favorites.');
    });
  }

  async function toggleSubscribed() {
    if (!details) return;
    await task('bookmark', async () => {
      const nextSub = !detailSubscribed;
      await call('set_library_subscribed', {
        item: details!.item,
        isSubscribed: nextSub,
        profileId: activeProfile,
      });
      await refresh();
      message(nextSub ? 'Added to subscriptions.' : 'Unsubscribed from updates.');
      if (nextSub) void checkSubscriptionsAutomatically(activeProfile);
    });
  }

  type SubscriptionCheckReport = { checked: number; skipped: number; failed: number; updates: { mediaId: string; title: string; episodeCount: number; generation: string; episodes: Record<string, string[]> }[] };
  let subscriptionReport = $state<SubscriptionCheckReport | null>(null);
  let subscriptionProfileReady = $state(false);
  let subscriptionAlertsSaving = $state(false);
  let subscriptionAlertError = $state('');
  async function setSubscriptionAlerts(enabled: boolean) {
    if (subscriptionAlertsSaving) return;
    subscriptionAlertsSaving = true;
    try {
      snapshot.settings = await updateSettings({ subscriptionNotifications: enabled }, 'global');
      subscriptionAlertError = '';
    } catch (e) { showError(e); }
    finally { subscriptionAlertsSaving = false; }
  }
  async function deliverSubscriptionAlerts(profileId: string) {
    if (!snapshot.settings?.global?.subscriptionNotifications || !subscriptionProfileReady || activeProfile !== profileId) return;
    const result = await call<{ delivered: number; failed: number }>('deliver_subscription_alerts', { profileId });
    if (activeProfile === profileId) subscriptionAlertError = result.failed ? 'Desktop notifications are unavailable. Alerts will be retried while the app is open.' : '';
  }
  async function checkSubscriptionsAutomatically(profileId: string) {
    if (!subscriptionProfileReady || activeProfile !== profileId) return;
    try {
      await call<SubscriptionCheckReport>('check_subscriptions', { profileId, force: false });
      if (subscriptionProfileReady && activeProfile === profileId) await deliverSubscriptionAlerts(profileId);
    } catch {
      if (activeProfile === profileId && snapshot.settings?.global?.subscriptionNotifications) {
        subscriptionAlertError = 'The subscription check or desktop notification failed. It will be retried while the app is open.';
      }
    }
  }
  $effect(() => {
    if (!subscriptionProfileReady) return;
    const profileId = activeProfile;
    const start = window.setTimeout(() => { void checkSubscriptionsAutomatically(profileId); }, 2000);
    const interval = window.setInterval(() => { void checkSubscriptionsAutomatically(profileId); }, 15 * 60 * 1000);
    return () => { window.clearTimeout(start); window.clearInterval(interval); };
  });
  $effect(() => { activeProfile; subscriptionReport = null; });
  async function checkSubscriptions() {
    const profileId = activeProfile;
    await task('subscriptions', async () => {
      const result = await call<SubscriptionCheckReport>('check_subscriptions', { profileId, force: true });
      if (activeProfile !== profileId) return;
      subscriptionReport = result;
      try { await deliverSubscriptionAlerts(profileId); }
      catch { subscriptionAlertError = 'Desktop notifications are unavailable. Alerts will be retried while the app is open.'; }
      message(`Checked ${result.checked} subscription${result.checked === 1 ? '' : 's'}.`);
    });
  }

  async function acknowledgeSubscriptionUpdates() {
    if (!subscriptionReport?.updates.length) return;
    const profileId = activeProfile;
    const updates = subscriptionReport.updates;
    await task('subscriptions', async () => {
      await call('acknowledge_subscription_updates', { profileId, updates });
      if (activeProfile === profileId && subscriptionReport) subscriptionReport = { ...subscriptionReport, updates: [] };
      message('Subscription updates marked seen.');
    });
  }

  async function removeLibraryItem(id: string) {
    await task('bookmark', async () => {
      await call('remove_library_item', { id });
      await refresh();
      message('Removed from library.');
    });
  }

  async function confirmClearLibrary(scope: 'all' | 'tab') {
    modal = null;
    await task('bookmark', async () => {
      if (scope === 'all') {
        await call('clear_library', { status: null });
        message('Library cleared.');
      } else {
        const status =
          ['watching', 'plan_to_watch', 'completed', 'on_hold', 'dropped'].includes(libraryTab)
            ? libraryTab
            : null;
        await call('clear_library', { status });
        message(`Cleared ${libraryTab.replace(/_/g, ' ')} list.`);
      }
      await refresh();
    });
  }

  async function removeHistoryItem(id: string) {
    await task('history', async () => {
      await call('remove_history_item', { id });
      await refresh();
      message('Removed from watch history.');
    });
  }

  async function confirmClearHistory() {
    modal = null;
    await task('history', async () => {
      await call('clear_history');
      await refresh();
      message('Watch history cleared.');
    });
  }

  async function toggleBookmark() {
    if (!details) return;
    const item = details.item;
    await task('bookmark', async () => {
      const existing = detailLibraryItem;
      if (existing) {
        await call('remove_library_item', { id: existing.id });
        message('Removed from your library.');
      } else {
        await call('bookmark', { item });
        message('Saved to your library.');
      }
      await refresh();
    });
  }
  async function resume(watch: WatchEntry) {
    await task('resume', async () => {
      const loaded = await call<Details>('load_details', {
        providerId: watch.item.providerId,
        url: watch.item.url,
        profileId: activeProfile,
      });
      const episode =
        loaded.episodes.find((ep) => ep.data === watch.episode.data) ??
        loaded.episodes.find(
          (ep) => ep.name === watch.episode.name && ep.season === watch.episode.season,
        ) ??
        watch.episode;
      const resolved = await call<Links>('resolve_links', {
        providerId: watch.item.providerId,
        data: episode.data,
      });
      if (!resolved.streams.length)
        throw new Error('No playable stream was returned. Open the title and try another source.');
      let queue: PlaybackQueue | null = null;
      if (loaded.item && loaded.episodes.length > 0) {
        const curIdx = loaded.episodes.findIndex(
          (ep) =>
            (ep.id && ep.id === episode.id) ||
            ep.data === episode.data ||
            (ep.name === episode.name && ep.season === episode.season),
        );
        queue = {
          profileId: activeProfile,
          media: loaded.item,
          episodes: loaded.episodes,
          currentIndex: curIdx >= 0 ? curIdx : 0,
        };
      }
      const ranked = await call<RankedStream[]>('rank_streams', { streams: resolved.streams, profileId: activeProfile });
      const preferred = ranked.find((entry) => entry.usable);
      if (!preferred || !resolved.streams[preferred.index]) throw new Error('No streams match your source preferences.');
      const initial = await call<PlayerState | string>('play', {
        stream: resolved.streams[preferred.index],
        mirrors: resolved.streams,
        subtitles: resolved.subtitles,
        item: loaded.item,
        episode,
        resume: watch.position,
        profileId: activeProfile,
        queue,
      });
      if (initial && typeof initial === 'object' && 'active' in initial && initial.active) {
        playerState = initial;
      } else if (!playerState) {
        playerState = {
          sessionId: typeof initial === 'string' ? initial : undefined,
          active: true,
          playing: true,
          paused: false,
          position: watch.position,
          duration: watch.duration,
          volume: 100,
          speed: 1,
          itemName: loaded.item.name,
          episodeName: episode.name,
          mediaId: loaded.item.url,
          episodeId: episode.name,
          profileId: activeProfile,
          hasNext: queue ? queue.currentIndex + 1 < queue.episodes.length : false,
          hasPrev: queue ? queue.currentIndex > 0 : false,
          queueIndex: queue ? queue.currentIndex : undefined,
          queueCount: queue ? queue.episodes.length : undefined,
        };
      }
      details = loaded;
      if (details?.item) {
        await loadEpisodeProgresses();
      }
      selectedEpisode = episode;
      links = resolved;
      rankedStreams = ranked;
      await refresh();
    });
  }
  async function playerAction(action: string) {
    try {
      await call('player_action', { action });
      if (action === 'stop') {
        playerState = null;
        await refresh();
      } else if (action === 'pause' || action === 'play') {
        if (playerState) {
          const p = action === 'pause';
          playerState = { ...playerState, paused: p, playing: !p };
        }
      }
    } catch (e) {
      showError(e);
    }
  }
  function isSubtitleArchive(subtitle: { url: string; [key: string]: unknown }): boolean {
    const format = subtitle.format ?? subtitle.mimeType ?? subtitle.mime_type;
    if (typeof format === 'string' && ['zip', 'application/zip'].includes(format.toLowerCase())) return true;
    try { return new URL(subtitle.url).pathname.toLowerCase().endsWith('.zip'); } catch { return false; }
  }
  const providerSubtitleArchives = $derived.by(() => {
    if (!links || !playerState?.active || !selectedEpisode || !details) return [];
    const matching = selectedEpisode.id ? selectedEpisode.id === playerState.episodeId
      : !!selectedEpisode.name && selectedEpisode.name === playerState.episodeName && details.item.name === playerState.itemName;
    return matching ? links.subtitles.map((subtitle, index) => ({ subtitle, index })).filter(({ subtitle }) => isSubtitleArchive(subtitle)) : [];
  });
  async function chooseProviderArchive(index: number) {
    const sessionId = playerState?.sessionId;
    if (!sessionId || busy) return;
    busy = 'subtitleArchiveFetch';
    try {
      const result = await call<{ status: 'archive'; id: string; entries: { index: number; name: string; size: number }[] }>('prepare_provider_subtitle_archive', { sessionId, index });
      subtitleArchive = result;
      modal = 'subtitleArchive';
    } catch (e) { showError(e); }
    finally { busy = ''; }
  }
  async function chooseLocalSubtitle() {
    const sessionId = playerState?.sessionId;
    if (!sessionId) return;
    try {
      const result = await call<{ status: 'added' } | { status: 'archive'; id: string; entries: { index: number; name: string; size: number }[] } | null>('choose_local_subtitle', { sessionId });
      if (result?.status === 'added') message('Subtitle added.');
      else if (result?.status === 'archive') { subtitleArchive = result; modal = 'subtitleArchive'; }
    } catch (e) { showError(e); }
  }
  function cancelSubtitleArchive() {
    const archive = subtitleArchive;
    subtitleArchive = null;
    if (archive) void call('cancel_archived_subtitle', { id: archive.id }).catch(() => {});
  }
  async function selectArchivedSubtitle(index: number) {
    const archive = subtitleArchive;
    if (!archive || busy) return;
    busy = 'subtitleArchive';
    try {
      await call('select_archived_subtitle', { id: archive.id, index });
      message('Subtitle added.');
    } catch (e) { showError(e); }
    finally { subtitleArchive = null; modal = null; busy = ''; }
  }
  async function sendPlayerCommand(command: PlayerCommand) {
    try {
      playerState = await call<PlayerState>('player_command', { command });
    } catch (e) {
      showError(e);
    }
  }
  function formatTime(seconds: number) {
    const minutes = Math.floor(seconds / 60);
    return `${minutes}:${String(Math.floor(seconds % 60)).padStart(2, '0')}`;
  }
  /** Show a dialog as a modal as soon as it exists (no timing dependence on bindings). */
  function openModal(node: HTMLDialogElement) {
    if (!node.open) node.showModal();
    return {};
  }
  function typingTarget(target: EventTarget | null) {
    return target instanceof HTMLElement && !!target.closest('input, textarea, select, [contenteditable="true"]');
  }
  function shortcut(event: KeyboardEvent) {
    if (playerLocked && event.key === 'Escape' && !modal) {
      event.preventDefault();
      playerLocked = false;
      return;
    }
    if (event.defaultPrevented || event.ctrlKey || event.metaKey || event.altKey) {
      // Ctrl+K below still needs to run.
      if (!((event.ctrlKey || event.metaKey) && event.key === 'k')) return;
    }
    if (event.key === '?' && !typingTarget(event.target) && !modal) {
      event.preventDefault();
      shortcutsOpen = true;
      return;
    }
    if (!typingTarget(event.target) && !modal && !shortcutsOpen && !exitPrompt) {
      const onControl = event.target instanceof HTMLElement && !!event.target.closest('button, a[href], [role="slider"], [role="tab"], summary, dialog');
      if (appearance.navigation === 'spatial' && ['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown'].includes(event.key)) {
        if (spatialMove(event.key.replace('Arrow', '').toLowerCase() as 'left' | 'right' | 'up' | 'down')) event.preventDefault();
        return;
      }
      // Player keys work when nothing else is focused (so Space still presses a focused button).
      if (playerState?.active && !onControl && !playerLocked) {
        if (event.key.toLowerCase() === 'h') {
          event.preventDefault();
          if (!event.repeat) beginSpeedHold();
          return;
        }
        const seekStep = playerPreferences.seekStepSeconds ?? 10;
        const keyed: Record<string, PlayerCommand> = {
          ' ': { type: 'toggle_pause' },
          k: { type: 'toggle_pause' },
          j: { type: 'seek_relative', seconds: -seekStep },
          l: { type: 'seek_relative', seconds: seekStep },
          m: { type: 'set_mute', muted: !playerState.muted },
          f: { type: 'set_fullscreen', fullscreen: !playerState.fullscreen },
          '<': { type: 'set_speed', speed: Math.max(0.25, Math.round((playerState.speed - 0.25) * 100) / 100) },
          '>': { type: 'set_speed', speed: Math.min(4, Math.round((playerState.speed + 0.25) * 100) / 100) },
        };
        const command = keyed[event.key];
        if (command) {
          event.preventDefault();
          void sendPlayerCommand(command);
          return;
        }
      }
    }
    if (
      event.key === '/' &&
      !(
        event.target instanceof HTMLElement &&
        event.target.closest('input, textarea, select, [contenteditable="true"]')
      )
    ) {
      event.preventDefault();
      openSearch();
    }
    if (event.key === 'Escape') {
      if (showSuggestionsDropdown) {
        showSuggestionsDropdown = false;
        return;
      }
      if (details && !modal) {
        event.preventDefault();
        closeDetails();
        return;
      }
    }
    if ((event.ctrlKey || event.metaKey) && event.key === 'k') {
      event.preventDefault();
      if (!selectedProvider) page = 'extensions';
      void tick().then(() => searchInput?.focus());
    }
  }
  async function refreshActiveMirrorRanking(sessionId: string) {
    const currentLinks = links;
    const currentEpisode = selectedEpisode;
    if (!currentLinks || !currentEpisode?.id || currentEpisode.id !== playerState?.episodeId) return;
    try {
      const ranked = await call<RankedStream[]>('rank_streams', { streams: currentLinks.streams, profileId: activeProfile, sessionId });
      if (links === currentLinks && selectedEpisode === currentEpisode) rankedStreams = ranked;
    } catch (e) { showError(e); }
  }
  let unlistenPlayer: (() => void) | undefined;
  let unlistenEntry: (() => void) | undefined;
  let unlistenExit: (() => void) | undefined;
  // Links delivered before the interface was ready (or while a PIN prompt was up) are shown now.
  $effect(() => {
    if (subscriptionProfileReady && !modal) void untrack(() => drainEntries());
  });
  onMount(() => {
    void onEntryPending(() => void drainEntries()).then((unlisten) => {
      unlistenEntry = unlisten;
    });
    void onConfirmExit((info) => (exitPrompt = info)).then((unlisten) => {
      unlistenExit = unlisten;
    });
    void getSettings()
      .then((settings) => {
        const configured = (settings.global as { externalPlayer?: ExternalPlayerConfig }).externalPlayer;
        if (configured) externalPlayer = configured;
      })
      .catch(() => {});
    void call<typeof playerCapabilities>('player_capabilities').then((caps) => { playerCapabilities = caps; }).catch(() => {});
    void call<PlayerState>('get_player_state').then((st) => {
      if (st?.active) playerState = st;
    }).catch(() => {});
    void onPlayerEvent((event) => {
      if (event.type === 'state_changed') {
        playerState = event.state.active ? event.state : null;
      } else if (event.type === 'shutdown') {
        playerState = null;
        void refresh();
      } else if (event.type === 'end_file') {
        if (event.reason !== 'eof' || !playerState?.hasNext) {
          playerState = null;
        }
        void refresh();
      } else if (event.type === 'notice') {
        message(event.message);
      } else if (event.type === 'source_changed') {
        message(`Playing ${event.name} (${event.qualityLabel}).`);
        void refreshActiveMirrorRanking(event.sessionId);
      } else if (event.type === 'error') {
        message(event.message);
        playerState = null;
      }
    }).then((unlisten) => {
      unlistenPlayer = unlisten;
    });

    void refresh()
      .then(() => {
        const glob = snapshot.settings?.global;
        if (glob?.autoSelectProfile === false && (snapshot.profiles?.length ?? 0) > 1) {
          modal = 'profiles';
        } else if (currentProfile?.hasPin) {
          pendingSwitchProfileId = currentProfile.id;
          modal = 'profilePin';
        } else {
          subscriptionProfileReady = true;
        }
        if (selectedProvider) return chooseProvider(selectedProvider);
      })
      .catch((e) => {
        ready = true;
        showError(e);
      });
    return () => {
      clearTimeout(toastTimer);
      unlistenPlayer?.();
      unlistenEntry?.();
      unlistenExit?.();
    };
  });
  onDestroy(() => {
    searchAbortController?.abort();
    detailsAbortController?.abort();
    resolveAbortController?.abort();
  });
  $effect(() => {
    if (modal && dialogElement && !dialogElement.open) {
      dialogElement.showModal();
      queueMicrotask(() => dialogElement.querySelector('input')?.focus());
    }
    if (!modal && dialogElement?.open) dialogElement.close();
  });
</script>

<svelte:window
  onkeydown={shortcut}
  onkeyup={(event) => { if (event.key.toLowerCase() === 'h') endSpeedHold(); }}
  onblur={endSpeedHold}
  onpointerdown={(e) => {
    const target = e.target as HTMLElement | null;
    if (showSuggestionsDropdown && !target?.closest('.search-bar-container')) {
      showSuggestionsDropdown = false;
    }
  }}
/>

<a class="skip-link" href="#main-content">Skip to content</a>
<div
  class="app-shell"
  class:compact={posterDensity === 'compact'}
  class:hide-badges={appearance.showBadges === false}
  class:title-overlay={appearance.titlePlacement === 'overlay'}
>
  <aside class="sidebar">
    <a
      href="#home"
      class="brand"
      onclick={(event) => {
        event.preventDefault();
        navigate('discover');
        searchActive = false;
      }}
      aria-label="CloudStream DESKTOP home"
      ><img src="/cloudstream-mark.svg" alt="" />
      <div>CloudStream <span>DESKTOP</span></div></a
    >
    <span class="nav-label">BROWSE</span>
    <nav aria-label={t('nav.mainLabel')}>
      <button
        class:active={page === 'discover' && !searchView}
        aria-current={page === 'discover' && !searchView ? 'page' : undefined}
        onclick={() => {
          navigate('discover');
          searchActive = false;
        }}><HomeIcon size={21} /><span>{t('nav.home')}</span></button
      >
      <button
        class:active={page === 'discover' && searchView}
        aria-label={i18n.locale === 'en' ? t('nav.searchPage') : t('nav.search')}
        aria-current={page === 'discover' && searchView ? 'page' : undefined}
        onclick={openSearch}><Search size={21} /><span>{t('nav.search')}</span><kbd>/</kbd></button
      >
      <button
        class:active={page === 'library'}
        aria-current={page === 'library' ? 'page' : undefined}
        onclick={() => navigate('library')}
        ><Library size={21} /><span>{t('nav.library')}</span>{#if snapshot.library.length}<span
            class="nav-count">{snapshot.library.length}</span
          >{/if}</button
      >
      <button
        class:active={page === 'downloads'}
        aria-current={page === 'downloads' ? 'page' : undefined}
        onclick={() => navigate('downloads')}><DownloadIcon size={21} /><span>{t('nav.downloads')}</span></button
      >
      <button
        class:active={page === 'extensions'}
        aria-current={page === 'extensions' ? 'page' : undefined}
        onclick={() => navigate('extensions')}><Puzzle size={21} /><span>{t('nav.extensions')}</span></button
      >
      <button
        class="mobile-settings"
        class:active={page === 'settings'}
        onclick={() => navigate('settings')}><Settings size={21} /><span>{t('nav.settings')}</span></button
      >
    </nav>
    <div class="sidebar-sources">
      <div class="sidebar-section-heading">
        <span class="nav-label">SOURCES</span><button
          class="icon-button"
          aria-label="Add a repository"
          onclick={() => {
            navigate('extensions');
            modal = 'repository';
          }}><Plus size={17} /></button
        >
      </div>
      {#each providers as source}<button
          class="sidebar-provider"
          class:selected={selectedProvider === source.id}
          onclick={() => {
            navigate('discover');
            searchActive = false;
            void chooseProvider(source.id);
          }}
          ><span class="provider-avatar">{source.lang.toUpperCase().slice(0, 2)}</span><span
            >{source.name}</span
          ><span class="connection-dot"></span></button
        >{/each}
      {#if !providers.length}<p class="sidebar-empty">
          Connect your first source<br />to start exploring.
        </p>{/if}
      <button
        class="add-source"
        onclick={() => {
          navigate('extensions');
          modal = 'repository';
        }}><Plus size={16} />Add a repository</button
      >
    </div>
    <div class="sidebar-bottom">
      <button class:active={page === 'settings'} onclick={() => navigate('settings')}
        ><Settings size={20} /><span>{t('nav.settings')}</span></button
      >
      <button
        type="button"
        class="local-profile profile-action-btn"
        data-testid="profile-switcher-btn"
        onclick={openProfileManager}
        title="Manage profiles"
      >
        <span class="profile avatar-badge">
          {#if currentProfile?.avatar === 'popcorn'}🍿
          {:else if currentProfile?.avatar === 'film'}🎬
          {:else if currentProfile?.avatar === 'sparkles'}✨
          {:else if currentProfile?.avatar === 'heart'}❤️
          {:else if currentProfile?.avatar === 'lock'}🔒
          {:else}{currentProfile?.name ? currentProfile.name.charAt(0).toUpperCase() : 'P'}
          {/if}
        </span>
        <div class="profile-info-text">
          <div class="profile-name-row">
            <strong>{currentProfile?.name ?? 'Default Profile'}</strong>
            {#if currentProfile?.hasPin}
              <span class="pin-indicator" title="PIN protected">🔒</span>
            {/if}
          </div>
          <span>Switch profile</span>
        </div>
      </button>
      <div class="build-label">CloudStream for Linux <span>0.1.0</span></div>
    </div>
  </aside>
  <div class="main-shell">
    <header class="topbar">
      <div class="app-context">
        {details
          ? 'Details'
          : page === 'discover'
            ? searchView
              ? t('nav.search')
              : t('nav.home')
            : page === 'library'
              ? i18n.locale === 'en' ? 'Library' : t('nav.library')
              : page === 'downloads'
                ? t('nav.downloads')
                : page === 'extensions'
                  ? t('nav.extensions')
                  : t('nav.settings')}
      </div>
      <div class="search-bar-container">
        <form class="search-bar" onsubmit={runSearch}>
          <Search size={19} /><input
            bind:this={searchInput}
            value={query}
            aria-label={page === 'extensions' ? 'Search extensions' : 'Search your selected source'}
            placeholder={page === 'extensions'
              ? 'Search extensions…'
              : activeProvider
                ? 'Search movies, series, anime…'
                : 'Connect a source to search'}
            disabled={!selectedProvider && page !== 'extensions'}
            onfocus={() => {
              if (page !== 'extensions') {
                if (!query.trim() && searchHistory.length > 0) {
                  showSuggestionsDropdown = true;
                } else if (query.trim().length >= 2 && searchSuggestions.length > 0) {
                  showSuggestionsDropdown = true;
                }
              }
            }}
            oninput={(e) => onSearchInput(e.currentTarget.value)}
          /><kbd>Ctrl K</kbd><button
            class="search-submit"
            type="submit"
            aria-label="Search"
            disabled={(!selectedProvider && page !== 'extensions') || !query.trim()}
            ><ArrowRight size={18} /></button
          >
        </form>
        {#if showSuggestionsDropdown && page !== 'extensions'}
          <div class="search-suggestions-dropdown" role="listbox">
            {#if !query.trim() && searchHistory.length > 0}
              <div class="suggestions-header">
                <span>Recent searches</span>
                <button type="button" class="text-button" onclick={clearSearchHistory}>Clear all</button>
              </div>
              {#each searchHistory as entry}
                <div class="suggestion-item">
                  <button
                    type="button"
                    class="suggestion-text"
                    onclick={() => {
                      query = entry.query;
                      void runSearch();
                    }}
                  >
                    <Clock size={15} />
                    <span>{entry.query}</span>
                  </button>
                  <button
                    type="button"
                    class="icon-button mini"
                    aria-label="Delete history entry"
                    onclick={() => deleteSearchHistory(entry.query)}
                  >
                    <X size={13} />
                  </button>
                </div>
              {/each}
            {:else if searchSuggestions.length > 0}
              <div class="suggestions-header">
                <span>Suggestions</span>
              </div>
              {#each searchSuggestions as sug}
                <button
                  type="button"
                  class="suggestion-item suggestion-button"
                  onclick={() => {
                    query = sug;
                    void runSearch();
                  }}
                >
                  <Search size={15} />
                  <span>{sug}</span>
                </button>
              {/each}
            {/if}
          </div>
        {/if}
      </div>
      {#if providers.length}<div class="topbar-source">
          <span class="provider-avatar"><Globe size={16} /></span>
          <div>
            <label for="source-select">YOUR SOURCE</label><select
              id="source-select"
              aria-label="Your source"
              value={selectedProvider}
              onchange={(event) => chooseProvider(event.currentTarget.value)}
              ><option value="">No source selected</option>
              {#each providers as source}<option value={source.id}>{source.name}</option
                >{/each}</select
            >
          </div>
        </div>{/if}
      <button
        class="icon-button"
        aria-label="Random source"
        title="Pick random source"
        disabled={!providers.length}
        onclick={pickRandomSource}><Shuffle size={18} /></button
      >
      <button
        class="icon-button"
        aria-label="Random title"
        title="Pick random title"
        disabled={!home.rows.some((r) => r.items.length) && !snapshot.library.length}
        onclick={pickRandomTitle}><Sparkles size={18} /></button
      >
      <button
        class="icon-button"
        aria-label="Refresh source"
        title="Refresh source"
        disabled={!selectedProvider || loadingContent}
        onclick={() => chooseProvider(selectedProvider, true)}><RefreshCw size={19} /></button
      >
      <button
        class="icon-button"
        title="Open a stream URL"
        aria-label="Open a stream URL"
        onclick={() => (modal = 'direct')}><Link size={19} /></button
      >
    </header>
    {#if !nativeAvailable()}<div class="preview-banner">
        <MonitorPlay size={16} /><span
          >Browser preview · Open the desktop app to connect sources and play media.</span
        ><code>npm run desktop</code>
      </div>{/if}
    <main id="main-content">
      {#if ready && !providers.length && installedCount && page === 'discover' && !details}
        <div class="source-unavailable" role="status">
          <CircleHelp size={20} />
          <div>
            <strong>No media sources are active</strong>
            <p>
              {failedExtensions.length
                ? 'Your installed extensions could not start. Review their compatibility details.'
                : 'Installed packages have not registered a searchable media source.'}
            </p>
          </div>
          <button
            class="button secondary"
            onclick={() => {
              navigate('extensions');
              extensionTab = 'installed';
            }}>Review extensions <ArrowRight size={15} /></button
          >
        </div>
      {/if}
      {#if error}<div class="error-banner" role="alert">
          <AlertCircle size={19} /><span>{error}</span><button
            aria-label="Dismiss error"
            onclick={() => (error = '')}><X size={18} /></button
          >
        </div>{/if}

      {#if details}
        {#if details.backgroundPosterUrl || (details as any).backgroundPoster || metadataArt.backdropUrl}
          <div class="details-backdrop-hero">
            <img src={details.backgroundPosterUrl ?? (details as any).backgroundPoster ?? metadataArt.backdropUrl} alt="" referrerpolicy="no-referrer" />
          </div>
        {/if}
        <button
          class="back-button"
          onclick={closeDetails}
        ><ChevronLeft size={17} />Back to {page === 'library' ? 'library' : 'browse'}</button>
        <section class="details-layout">
          <div class="detail-poster">
            <Film size={70} strokeWidth={0.8} /><span>FROM YOUR SOURCES</span>
            {#if metadataArt.posterUrl ?? details.item.posterUrl}
              <img
                alt={details.item.name}
                src={metadataArt.posterUrl ?? details.item.posterUrl}
                referrerpolicy="no-referrer"
              />
            {/if}
          </div>
          <div class="detail-copy">
            {#if details.logoUrl || (details as any).logo}
              <img class="detail-logo" src={details.logoUrl ?? (details as any).logo} alt={details.item.name} referrerpolicy="no-referrer" />
            {/if}
            <span class="eyebrow"
              >{details.item.mediaType?.replace('TvSeries', 'TV SERIES') ?? 'VIDEO'}{details.item.year || (details as any).year
                ? ` · ${details.item.year ?? (details as any).year}`
                : ''}</span
            >
            <h1>{details.item.name}</h1>

            <div class="detail-meta-pills">
              {#if details.item.score || details.item.rating || (details as any).score || (details as any).rating}
                <span class="detail-meta-pill detail-rating-score">
                  <Star size={13} fill="currentColor" />{formatScore(details.item.score ?? (details as any).score, details.item.rating ?? (details as any).rating)}
                </span>
              {/if}
              {#if details.item.year || (details as any).year}
                <span class="detail-meta-pill">
                  {details.item.year ?? (details as any).year}
                </span>
              {/if}
              {#if details.duration}
                <span class="detail-meta-pill">
                  <Clock size={13} />{formatRuntime(details.duration)}
                </span>
              {/if}
              {#if details.contentRating}
                <span class="detail-meta-pill content-rating-badge">{details.contentRating}</span>
              {/if}
              {#if (details as any).nextAiring || (details as any).status}
                <span class="detail-meta-pill">
                  {(details as any).nextAiring ?? (details as any).status}
                </span>
              {/if}
              {#if details.comingSoon}
                <span class="detail-meta-pill" style="color: #60a5fa;"><Calendar size={13} />Coming Soon</span>
              {/if}
              <span class="detail-meta-pill">{details.item.providerId}</span>
            </div>

            <p class="detail-plot">
              {sanitizePlot(details.plot)}
            </p>
            <div class="tags">
              {#each details.tags as tag}<span>{tag}</span>{/each}
            </div>
            <div class="button-row">
              {#if latestWatchProgress && resumeEpisode}
                <button
                  class="button primary"
                  disabled={!!busy}
                  onclick={() => resolve(resumeEpisode, latestWatchProgress!.positionSeconds)}
                >
                  <Play size={18} fill="currentColor" />Resume {resumeEpisode.name || 'Episode'} ({Math.floor(latestWatchProgress.positionSeconds / 60)}m)
                </button>
              {/if}
              <button
                class="button {latestWatchProgress ? 'secondary' : 'primary'}"
                disabled={!!busy || !details.episodes.length}
                onclick={() => resolve(details!.episodes[0])}
              >
                {#if busy === 'resolve'}<LoaderCircle class="spin" size={18} />{:else}<Play
                    size={18}
                    fill="currentColor"
                  />{/if}{latestWatchProgress ? 'Play from start' : 'Find a stream'}
              </button>
              <button class="button secondary" disabled={!!busy} onclick={toggleBookmark}>
                {#if detailSaved}<Check size={18} />{:else}<Bookmark size={18} />{/if}{detailSaved
                  ? 'Saved to library'
                  : 'Save to library'}
              </button>
              {#if details.trailers?.length}
                <button class="button secondary" disabled={!!busy} onclick={() => playTrailer(details!.trailers![0])}>
                  <Video size={17} />Watch trailer
                </button>
              {/if}
              <button class="icon-button" title="Share title link" onclick={shareMedia}>
                <Share2 size={17} />
              </button>
            </div>
            <div class="detail-library-controls">
              <div class="status-select-wrap">
                <select
                  class="status-select"
                  value={detailStatus}
                  onchange={(e) => updateWatchStatus(e.currentTarget.value as WatchStatus)}
                  disabled={!!busy}
                >
                  <option value="none">No status</option>
                  <option value="watching">Watching</option>
                  <option value="plan_to_watch">Plan to watch</option>
                  <option value="completed">Completed</option>
                  <option value="on_hold">On hold</option>
                  <option value="dropped">Dropped</option>
                </select>
              </div>
              <button
                class="button secondary icon-toggle-btn"
                class:active={detailFavorite}
                disabled={!!busy}
                onclick={toggleFavorite}
                title={detailFavorite ? 'Remove from favorites' : 'Add to favorites'}
              ><Heart size={16} fill={detailFavorite ? 'currentColor' : 'none'} />{detailFavorite ? 'Favorited' : 'Favorite'}</button>
              <button
                class="button secondary icon-toggle-btn"
                class:active={detailSubscribed}
                disabled={!!busy}
                onclick={toggleSubscribed}
                title={detailSubscribed ? 'Unsubscribe' : 'Subscribe'}
              ><Bell size={16} fill={detailSubscribed ? 'currentColor' : 'none'} />{detailSubscribed ? 'Subscribed' : 'Subscribe'}</button>
            </div>
          </div>
        </section>

        {#if details.actors?.length}
          <section class="section cast-section">
            <div class="section-title">
              <h2>Cast & Characters</h2>
              <span>{details.actors.length} actors</span>
            </div>
            <div class="cast-list">
              {#each details.actors as actorData}
                {@const act = actorData.actor ?? (actorData as any)}
                <div class="cast-card">
                  {#if act.image}
                    <img class="cast-avatar" src={act.image} alt={act.name} referrerpolicy="no-referrer" />
                  {:else}
                    <div class="cast-avatar fallback-avatar"><Users size={22} /></div>
                  {/if}
                  <span class="cast-name" title={act.name}>{act.name ?? 'Unknown'}</span>
                  {#if actorData.role || actorData.roleString}
                    <span class="cast-role" title={actorData.role || actorData.roleString}>{actorData.role || actorData.roleString}</span>
                  {/if}
                </div>
              {/each}
            </div>
          </section>
        {/if}

        {#if links}
          <section class="section streams-section">
            <div class="section-title">
              <h2>Choose a stream</h2>
              <div style="display: flex; align-items: center; gap: 10px;">
                <span>{visibleStreams.length} available</span>
                <button
                  type="button"
                  class="button secondary small"
                  disabled={!!busy || !selectedEpisode}
                  onclick={() => selectedEpisode && resolve(selectedEpisode)}
                >
                  <RefreshCw size={13} />Reload streams
                </button>
              </div>
            </div>
            {#each visibleStreams as rankedStream}
              {@const stream = links.streams[rankedStream.index]}
              <div class="stream-item">
                <button
                  class="stream-row"
                  disabled={!!busy}
                  onclick={() => playStream(stream)}
                >
                  <span class="source-icon"><Play size={18} /></span>
                  <div>
                    <strong>{stream.name}</strong>
                    <span>{rankedStream.qualityLabel ?? (stream.quality === 400 || stream.quality < 0 ? 'Unknown' : stream.quality === 0 ? 'Auto' : `${stream.quality}p`)} · {stream.streamType}</span>
                  </div>
                  <span class="stream-play">Play in mpv <ArrowUpRight size={16} /></span>
                </button>
                {#if externalPlayer.kind !== 'none' && !stream.resourceHandle}
                  <button
                    class="icon-button stream-external"
                    title="Open in your external player"
                    aria-label={`Open ${stream.name} in the external player`}
                    disabled={!!busy}
                    onclick={() => openInExternalPlayer(stream)}
                  ><ArrowUpRight size={18} /></button>
                {/if}
                {#if downloadsSupported && selectedEpisode}
                  <button
                    class="icon-button stream-download"
                    title="Download from this source"
                    aria-label={`Download ${selectedEpisode.name} from ${stream.name}`}
                    disabled={!!busy}
                    onclick={() => downloadEpisode(selectedEpisode!, {
                      sourceName: (stream.source?.trim() || stream.name),
                      streamName: stream.name,
                      quality: stream.quality,
                    })}
                  ><DownloadIcon size={18} /></button>
                {/if}
              </div>
            {/each}
            {#if !visibleStreams.length}
              <p class="muted">
                No streams available. Try another episode or adjust your source preferences.
              </p>
            {/if}
          </section>
        {/if}

        <Metadata {details} enabled={metadataEnabled} onfiller={(ids) => (metadataFillerIds = ids)} onart={(art) => (metadataArt = art)} onfindsource={findSourceFor} />

        <section class="section">
          <div class="section-title">
            <h2>{details.episodes.length > 1 ? 'Episodes' : 'Watch'}</h2>
            <span>{filteredEpisodes.length} {filteredEpisodes.length === 1 ? 'item' : 'items'}</span>
          </div>

          {#if availableSeasons.length > 1 || availableDubGroups.length > 1 || details.episodes.length > 1}
            <div class="episode-controls-bar">
              <div class="episode-controls-left">
                {#if availableSeasons.length > 1}
                  <select
                    class="filter-select"
                    aria-label="Filter by season"
                    value={selectedSeason}
                    onchange={(e) => {
                      selectedSeason = e.currentTarget.value === 'all' ? 'all' : Number(e.currentTarget.value);
                      episodeRangeIndex = 0;
                    }}
                  >
                    <option value="all">All Seasons</option>
                    {#each availableSeasons as s}
                      <option value={s}>Season {s}</option>
                    {/each}
                  </select>
                {/if}

                {#if availableDubGroups.length > 1}
                  <select
                    class="filter-select"
                    aria-label="Filter by audio dub"
                    value={selectedDubGroup}
                    onchange={(e) => {
                      selectedDubGroup = e.currentTarget.value;
                      episodeRangeIndex = 0;
                    }}
                  >
                    <option value="all">All Audio</option>
                    {#each availableDubGroups as dg}
                      <option value={dg}>{dg}</option>
                    {/each}
                  </select>
                {/if}

                {#if episodeRangeCount > 1}
                  <div class="episode-range-chips" role="radiogroup" aria-label="Episode ranges">
                    {#each Array.from({ length: episodeRangeCount }) as _, rIdx}
                      <button
                        type="button"
                        class="range-chip"
                        class:active={episodeRangeIndex === rIdx}
                        onclick={() => (episodeRangeIndex = rIdx)}
                      >
                        {rIdx * EPISODES_PER_CHUNK + 1}–{Math.min((rIdx + 1) * EPISODES_PER_CHUNK, filteredEpisodes.length)}
                      </button>
                    {/each}
                  </div>
                {/if}
              </div>

              <div class="episode-controls-right">
                {#if downloadsSupported && details.episodes.length > 1}
                  <button
                    type="button"
                    class="button secondary small"
                    disabled={!!busy || pagedEpisodes.length === 0}
                    onclick={() => downloadEpisodes(pagedEpisodes)}
                  >
                    <DownloadIcon size={13} />Download {pagedEpisodes.length === filteredEpisodes.length ? 'all' : 'these'} {pagedEpisodes.length}
                  </button>
                  {#if filteredEpisodes.length > pagedEpisodes.length}
                    <button
                      type="button"
                      class="button secondary small"
                      disabled={!!busy}
                      onclick={() => downloadEpisodes(filteredEpisodes)}
                    >
                      <DownloadIcon size={13} />Download all {filteredEpisodes.length}
                    </button>
                  {/if}
                {/if}
                {#if downloadsSupported}
                  <label class="download-type">
                    <span>Save as</span>
                    <select
                      class="filter-select"
                      aria-label="Download file type"
                      bind:value={downloadExtension}
                    >
                      {#each DOWNLOAD_TYPES as type}<option value={type}>{type === 'hls' ? 'HLS folder' : type}</option>{/each}
                    </select>
                  </label>
                {/if}
                <button
                  type="button"
                  class="button secondary small"
                  onclick={() => (episodeSortOrder = episodeSortOrder === 'asc' ? 'desc' : 'asc')}
                  title="Toggle episode sort order"
                >
                  <ArrowUpDown size={13} />
                  {episodeSortOrder === 'asc' ? 'Oldest first' : 'Newest first'}
                </button>
              </div>
            </div>
          {/if}

          {#each pagedEpisodes as episode, i}
            {@const globalIdx = episodeRangeIndex * EPISODES_PER_CHUNK + i}
            {@const watched = isEpisodeWatched(episode, globalIdx)}
            <div class="episode-item-wrap" class:watched={watched}>
              <button
                class="episode-row"
                disabled={!!busy}
                onclick={() => resolve(episode)}
              >
                <span class="episode-number">{String(episode.episode ?? globalIdx + 1).padStart(2, '0')}</span>
                <div style="display: flex; flex-direction: column; min-width: 0; flex: 1;">
                  <div style="display: flex; align-items: center; gap: 8px;">
                    <span style="font-weight: 500;">{episode.name}</span>
                    {#if (episode as any).isFiller}
                      <span class="filler-badge">FILLER</span>
                    {:else if episode.id && metadataFillerIds.includes(episode.id)}
                      <span class="filler-badge" title="Marked as filler by a metadata service">FILLER</span>
                    {/if}
                  </div>
                  {#if episode.description}
                    <p class="episode-synopsis">{sanitizePlot(episode.description)}</p>
                  {/if}
                  <div class="episode-meta-tags">
                    {#if episode.season}<small>S{episode.season}</small>{/if}
                    {#if episode.episode}<small>E{episode.episode}</small>{/if}
                    {#if episode.runTime}<small>{formatRuntime(episode.runTime * 60)}</small>{/if}
                    {#if episode.score || episode.rating}<small>⭐ {formatScore(episode.score, episode.rating)}</small>{/if}
                    {#if episode.dubStatus && episode.dubStatus !== 'none'}
                      <small style="text-transform: capitalize;">{episode.dubStatus}</small>
                    {/if}
                  </div>
                </div>
                <Play size={17} />
              </button>
              <div class="episode-actions">
                {#if downloadsSupported}
                  <button
                    class="episode-download-btn"
                    title="Download this episode"
                    aria-label={`Download ${episode.name}`}
                    disabled={!!busy}
                    onclick={() => downloadEpisode(episode)}
                  >
                    <DownloadIcon size={16} />
                  </button>
                {/if}
                <button
                  class="watched-toggle-btn"
                  class:is-watched={watched}
                  title={watched ? 'Mark unwatched' : 'Mark watched'}
                  disabled={!!busy}
                  onclick={() => toggleEpisodeWatched(episode, globalIdx)}
                >
                  {#if watched}<CheckCheck size={16} />{:else}<Circle size={16} />{/if}
                </button>
                {#if details.episodes.length > 1}
                  <button
                    class="mark-up-to-btn"
                    title="Mark watched up to here"
                    disabled={!!busy}
                    onclick={() => markUpToEpisode(episode, globalIdx)}
                  >
                    <CheckCheck size={13} />Up to here
                  </button>
                {/if}
              </div>
            </div>
          {/each}
        </section>

        {#if details.recommendations?.length}
          <MediaRail
            title="You might also like"
            items={details.recommendations}
            onopen={openItem}
            onexpand={() => {}}
          />
        {/if}
      {:else if page === 'discover'}
        <div class="page-heading home-heading">
          <div>
            <h1>
              {searchView
                ? 'Search'
                : expandedCategory !== null
                  ? home.rows[expandedCategory]?.name
                  : 'Home'}
            </h1>
            <p>
              {searchView
                ? 'Find something worth watching.'
                : activeProvider
                  ? `Explore ${activeProvider.name}`
                  : 'Your media. Your sources. One place.'}
            </p>
          </div>
          <div class="heading-actions">
            <button class="button secondary" onclick={() => (modal = 'direct')}
              ><Link size={17} />Open stream</button
            >
          </div>
        </div>
        {#if !providers.length}
          <section class="welcome-hero">
            <img src="/cloudstream-mark.svg" alt="" /><span class="eyebrow"
              >WELCOME TO CLOUDSTREAM</span
            >
            <h2>Your media. Your way.</h2>
            <p>
              Add your favorite Cloudstream sources and explore them on a bigger screen. Your
              library and watch progress stay with you, on this device.
            </p>
            <div class="button-row">
              <button
                class="button primary"
                onclick={() => {
                  navigate('extensions');
                  modal = 'repository';
                }}><Plus size={19} />Add a repository</button
              ><button class="button secondary" disabled={!!busy} onclick={addSample}
                >{#if busy === 'sample'}<LoaderCircle class="spin" size={18} />{:else}<Play
                    size={18}
                  />{/if}Try an open movie</button
              ><button class="button secondary" disabled={!!busy} onclick={addTorrentio}
                >{#if busy === 'torrentio'}<LoaderCircle class="spin" size={18} />{:else}<Sparkles
                    size={18}
                  />{/if}Install Torrentio</button
              >
            </div>
            <span class="welcome-footnote"
              ><ShieldCheck size={16} />No account needed. No tracking.</span
            >
          </section>
        {:else}
          {#if !searchView}<div
              class="content-filters"
              role="group"
              aria-label="Filter titles by type"
            >
              {#each ['All', 'Movies', 'TV series', 'Anime'] as filter}<button
                  class:chosen={contentFilter === filter}
                  aria-pressed={contentFilter === filter}
                  onclick={() => {
                    contentFilter = filter;
                    featuredIndex = 0;
                  }}>{filter}</button
                >{/each}<span class="source-caption"
                ><span class="connection-dot"></span>{activeProvider?.lang.toUpperCase()} source</span
              >
            </div>{/if}
          {#if loadingContent}<div class="loading-state" role="status">
              <div class="loading-posters" aria-hidden="true">
                <span></span><span></span><span></span><span></span><span></span>
              </div>
              <div><LoaderCircle class="spin" size={21} />Loading {activeProvider?.name}…</div>
            </div>
          {:else if searchView}
            {#if searchActive}
              <section class="section search-results">
                <div class="section-title">
                  <h2>Results for “{submittedQuery}”</h2>
                  <div style="display: flex; align-items: center; gap: 12px;">
                    <span>{filteredSearchItems.length} {filteredSearchItems.length === 1 ? 'title' : 'titles'}</span>
                    <div class="segmented-control" role="group" aria-label="Search view mode">
                      <button
                        type="button"
                        class="segmented-item"
                        class:active={searchViewMode === 'grouped'}
                        aria-pressed={searchViewMode === 'grouped'}
                        onclick={() => (searchViewMode = 'grouped')}>Grouped</button
                      >
                      <button
                        type="button"
                        class="segmented-item"
                        class:active={searchViewMode === 'merged'}
                        aria-pressed={searchViewMode === 'merged'}
                        onclick={() => (searchViewMode = 'merged')}>Merged</button
                      >
                    </div>
                  </div>
                </div>

                <!-- Source selector and client-side filters -->
                <div class="search-view-controls">
                  {#if providers.length > 1}
                    <div class="search-source-selector">
                      <span class="nav-label" style="margin-right: 4px;">SOURCES:</span>
                      <button
                        type="button"
                        class="segmented-item"
                        class:active={searchMode === 'all'}
                        aria-pressed={searchMode === 'all'}
                        onclick={() => {
                          searchMode = 'all';
                          selectedSearchProviders = [];
                          void runSearch();
                        }}
                      >
                        All sources ({providers.filter((p) => !p.disabled).length})
                      </button>
                      {#each providers.filter((p) => !p.disabled) as prov}
                        <button
                          type="button"
                          class="segmented-item"
                          class:active={searchMode === 'custom' && selectedSearchProviders.includes(prov.id)}
                          aria-pressed={searchMode === 'custom' && selectedSearchProviders.includes(prov.id)}
                          onclick={() => {
                            searchMode = 'custom';
                            toggleSearchProvider(prov.id);
                            void runSearch();
                          }}
                        >
                          {prov.name}
                        </button>
                      {/each}
                    </div>
                  {/if}

                  <!-- Structured filters for loaded items -->
                  <div class="search-filters-bar">
                    <div class="search-filter-group">
                      <label for="filter-type">Type:</label>
                      <select id="filter-type" bind:value={searchFilters.type}>
                        <option value="all">All types</option>
                        <option value="Movie">Movies</option>
                        <option value="TvSeries">TV Series</option>
                        <option value="Anime">Anime</option>
                      </select>
                    </div>
                    <div class="search-filter-group">
                      <label for="filter-quality">Quality:</label>
                      <select id="filter-quality" bind:value={searchFilters.quality}>
                        <option value="all">All qualities</option>
                        <option value="HD">HD</option>
                        <option value="4K">4K</option>
                        <option value="SD">SD</option>
                      </select>
                    </div>
                    <div class="search-filter-group">
                      <label for="filter-dub">Audio/Sub:</label>
                      <select id="filter-dub" bind:value={searchFilters.dubStatus}>
                        <option value="all">All audio</option>
                        <option value="dubbed">Dubbed</option>
                        <option value="subbed">Subbed</option>
                      </select>
                    </div>
                  </div>
                </div>

                <!-- Source scoped errors if any source failed -->
                {#each Object.values(sourceSearchStates) as state}
                  {#if state.error}
                    <div class="error-banner" style="margin-bottom: 12px; display: flex; justify-content: space-between; align-items: center;">
                      <span><strong>{state.providerName}:</strong> {state.error}</span>
                      <button type="button" class="button secondary" style="padding: 4px 10px; font-size: 12px;" onclick={() => retrySource(state.providerId)}>
                        Retry
                      </button>
                    </div>
                  {/if}
                {/each}

                {#if searchViewMode === 'grouped'}
                  <!-- Grouped by Source View -->
                  {#each Object.entries(sourceSearchStates) as [provId, state]}
                    {@const sourceItems = state.items.filter(filterMediaItem)}
                    <div class="source-group-section" data-source-id={provId}>
                      <div class="source-group-header">
                        <div class="source-group-title">
                          <Globe size={16} />
                          <span>{state.providerName}</span>
                          <span class="tag-pill">{sourceItems.length} {sourceItems.length === 1 ? 'item' : 'items'}</span>
                          {#if state.loading}
                            <LoaderCircle class="spin" size={16} />
                          {/if}
                        </div>
                        {#if state.hasNext}
                          <button
                            type="button"
                            class="button secondary"
                            style="padding: 4px 10px; font-size: 12px;"
                            disabled={state.loading}
                            onclick={() => loadMoreSource(provId)}
                          >
                            Load more from {state.providerName}
                          </button>
                        {/if}
                      </div>

                      {#if sourceItems.length > 0}
                        <div class="media-grid">
                          <Incremental items={sourceItems}>
                            {#snippet children(item, i)}<MediaCard {item} index={i} onclick={() => openItem(item)} />{/snippet}
                          </Incremental>
                        </div>
                      {:else if !state.loading && !state.error}
                        <p style="color: var(--subtle); font-size: 13px; margin: 8px 0 16px;">No titles found from this source.</p>
                      {/if}
                    </div>
                  {/each}
                {:else}
                  <!-- Merged Grid View -->
                  <div class="media-grid">
                    <Incremental items={filteredSearchItems}>
                      {#snippet children(item, i)}<MediaCard {item} index={i} onclick={() => openItem(item)} />{/snippet}
                    </Incremental>
                  </div>
                {/if}

                {#if searchHasNext}
                  <button
                    class="button secondary load-more"
                    onclick={loadMoreAll}
                    disabled={loadingContent}
                  >
                    Load more results <ChevronRight size={17} />
                  </button>
                {/if}

                {#if !searchItems.length && !loadingContent && Object.values(sourceSearchStates).every((s) => !s.loading)}
                  <div class="empty-state">
                    <Search size={42} strokeWidth={1.3} />
                    <h3>No matches this time</h3>
                    <p>Try another title or choose a different source above.</p>
                  </div>
                {/if}
              </section>
            {:else}
              <div class="empty-state search-empty">
                <div class="empty-search-icon"><Search size={42} strokeWidth={1.3} /></div>
                <h2>What would you like to watch?</h2>
                <p>
                  {activeProvider
                    ? `Search ${activeProvider.name} for a movie, series or anime.`
                    : 'Search movies, series or anime across your connected sources.'}
                </p>
                <span class="shortcut-hint"><kbd>Ctrl</kbd> + <kbd>K</kbd> to focus search</span>
              </div>
            {/if}
          {:else if expandedCategory !== null}
            <button class="back-button" onclick={() => (expandedCategory = null)}
              ><ChevronLeft size={18} />Back to home</button
            >
            <div class="media-grid">
              {#each home.rows[expandedCategory]?.items.filter(matchesFilter) ?? [] as item, i}<MediaCard
                  {item}
                  index={i}
                  onclick={() => openItem(item)}
                />{/each}
            </div>
            {#if home.rows[expandedCategory]?.hasNext}<button
                class="button secondary load-more"
                disabled={!!busy}
                onclick={() => moreTitles(expandedCategory!)}
                >{#if busy === 'category'}<LoaderCircle class="spin" size={17} />{/if}Load more
                titles</button
              >{/if}
          {:else}
            {#if featuredItem}<Spotlight
                item={featuredItem}
                source={activeProvider?.name ?? ''}
                category={home.rows[0]?.name ?? 'From your source'}
                index={featuredIndex % featuredItems.length}
                count={featuredItems.length}
                saved={snapshot.library.some(
                  (saved) =>
                    saved.item.url === featuredItem.url &&
                    saved.item.providerId === featuredItem.providerId,
                )}
                busy={!!busy}
                plot={spotlightDetails?.plot}
                tags={spotlightDetails?.tags}
                score={spotlightDetails?.item?.rating ?? featuredItem.rating}
                backdropUrl={spotlightDetails?.backgroundPosterUrl}
                ondetails={() => openItem(featuredItem)}
                onsave={() => saveItem(featuredItem)}
                onprevious={() =>
                  (featuredIndex =
                    (featuredIndex - 1 + featuredItems.length) % featuredItems.length)}
                onnext={() => (featuredIndex = (featuredIndex + 1) % featuredItems.length)}
              />{/if}
            <div id="home-catalog">
              {#if snapshot.history.length > 0}
                <section class="catalog-section" aria-label="Continue watching">
                  <div class="section-title">
                    <h2>Continue watching</h2>
                    <div class="rail-actions">
                      <button class="button secondary small" onclick={() => (modal = 'clearHistory')}>Clear all</button>
                    </div>
                  </div>
                  <div class="media-rail horizontal">
                    {#each snapshot.history as watch}
                      <div class="history-card-wrapper" style="flex: 0 0 280px; width: 280px;">
                        <button class="history-card" onclick={() => resume(watch)}>
                          {#if watch.item.posterUrl}
                            <div class="history-thumb">
                              <img src={watch.item.posterUrl} alt="" loading="lazy" />
                              <span class="history-play-overlay"><Play fill="currentColor" size={24} /></span>
                            </div>
                          {:else}
                            <div class="history-thumb history-placeholder">
                              <span class="history-play"><Play fill="currentColor" size={20} /></span>
                            </div>
                          {/if}
                          <div class="history-info">
                            <strong>{watch.item.name}</strong>
                            <span>{formatTime(watch.position)}{watch.duration ? ` / ${formatTime(watch.duration)}` : ''}</span>
                            <div class="progress-track">
                              <i style={`width:${watch.duration ? Math.min(100, (watch.position / watch.duration) * 100) : 0}%`}></i>
                            </div>
                          </div>
                        </button>
                        <button class="history-remove-btn" title="Remove" aria-label={`Remove ${watch.item.name} from history`} onclick={() => removeHistoryItem(watch.id)}>
                          <X size={14} />
                        </button>
                      </div>
                    {/each}
                  </div>
                </section>
              {/if}

              {#if homePlanToWatchItems.length > 0}
                <MediaRail
                  title="Plan to watch"
                  items={homePlanToWatchItems}
                  onopen={openItem}
                  onexpand={() => { page = 'library'; libraryTab = 'plan_to_watch'; }}
                />
              {/if}

              {#if homeFavoriteItems.length > 0}
                <MediaRail
                  title="Favorites"
                  items={homeFavoriteItems}
                  onopen={openItem}
                  onexpand={() => { page = 'library'; libraryTab = 'favorites'; }}
                />
              {/if}

              {#each home.rows as row, i}{#if row.items.some(matchesFilter)}<MediaRail
                    title={row.name}
                    items={row.items.filter(matchesFilter)}
                    orientation={row.orientation === 'horizontal' ? 'horizontal' : 'vertical'}
                    onopen={openItem}
                    onexpand={() => (expandedCategory = i)}
                  />{/if}{/each}
            </div>
            {#if !featuredItems.length && !snapshot.history.length && !homePlanToWatchItems.length && !homeFavoriteItems.length}<div class="empty-state">
                <Film size={42} strokeWidth={1.3} />
                {#if !selectedProvider}
                  <h3>No source selected</h3>
                  <p>Choose a connected source or pick one randomly to start browsing.</p>
                  <div class="button-row">
                    <button class="button primary" onclick={pickRandomSource}><Shuffle size={16} />Random source</button>
                    <button class="button secondary" onclick={() => navigate('extensions')}><Plus size={16} />Browse sources</button>
                  </div>
                {:else}
                  <h3>
                    {contentFilter === 'All'
                      ? 'Nothing to show yet'
                      : `No ${contentFilter.toLowerCase()} in this selection`}
                  </h3>
                  <p>
                    {contentFilter === 'All'
                      ? 'Try searching this source or loading another category.'
                      : 'Choose another filter or load more categories from this source.'}
                  </p>
                  {#if contentFilter !== 'All'}<button
                      class="button secondary"
                      onclick={() => (contentFilter = 'All')}>Show all titles</button
                    >{/if}
                {/if}
              </div>{/if}
            {#if home.hasMoreSections}<button
                class="button secondary load-more"
                disabled={loadingCategories}
                onclick={moreCategories}
                >{#if loadingCategories}<LoaderCircle class="spin" size={17} />{:else}<Plus
                    size={17}
                  />{/if}More categories</button
              >{/if}
          {/if}
        {/if}
      {:else if page === 'downloads'}
        <Downloads profileId={activeProfile} />
      {:else if page === 'library'}
        <div class="page-heading">
          <div>
            <span class="eyebrow">YOUR COLLECTION</span>
            <h1>{t('nav.library')}</h1>
            <p>Saved titles and everything you're watching.</p>
          </div>
          <span class="subtle-count">{tn('library.items', snapshot.library.length)}</span>
        </div>
        {#if snapshot.history.length}<section class="section">
            <div class="section-title">
              <h2>Continue watching</h2>
              <div style="display:flex;gap:8px;align-items:center;">
                <span>Saved on this device</span>
                <button class="button secondary small" onclick={() => (modal = 'clearHistory')}>Clear all</button>
              </div>
            </div>
            <div class="history-grid">
              {#each snapshot.history as watch}<div class="history-card-wrapper">
                  <button
                    class="history-card"
                    onclick={() => resume(watch)}
                    ><span class="history-play"><Play fill="currentColor" size={20} /></span>
                    <div>
                      <strong>{watch.item.name}</strong><span
                        >{formatTime(watch.position)}{watch.duration
                          ? ` / ${formatTime(watch.duration)}`
                          : ''}</span
                      >
                      <div class="progress-track">
                        <i
                          style={`width:${watch.duration ? Math.min(100, (watch.position / watch.duration) * 100) : 0}%`}
                        ></i>
                      </div>
                    </div></button
                  >
                  <button class="history-remove-btn" title="Remove" onclick={() => removeHistoryItem(watch.id)}><X size={14} /></button>
                </div>{/each}
            </div>
          </section>{/if}
        <section class="section">
          <div class="section-title">
            <h2>Saved for later</h2>
            <Bookmark size={18} />
          </div>
          <nav class="library-tabs-bar">
            {#each [['all','All'],['watching','Watching'],['plan_to_watch','Plan to watch'],['completed','Completed'],['on_hold','On hold'],['dropped','Dropped'],['favorites','Favorites'],['subscribed','Subscribed']] as [key, label]}
              <button class="library-tab-button" class:active={libraryTab === key} onclick={() => onLibraryTabChange(key as LibraryTab)}>{label}</button>
            {/each}
          </nav>
          {#if libraryTab === 'subscribed'}
            <p>Subscriptions are checked while the app is open, about every six hours. The first check establishes a baseline. Checks do not run while the app is closed.</p>
            <label><input type="checkbox" aria-label="New episode desktop notifications" checked={Boolean(snapshot.settings?.global?.subscriptionNotifications)} disabled={subscriptionAlertsSaving} onchange={(e) => setSubscriptionAlerts(e.currentTarget.checked)} /> Desktop notifications for new episodes</label>
            {#if subscriptionAlertError}<p role="status">{subscriptionAlertError}</p>{/if}
            <button type="button" class="button secondary small" disabled={Boolean(busy)} onclick={checkSubscriptions}>Check for new episodes</button>
            {#if subscriptionReport}
              <div role="status">
                <p>Checked {subscriptionReport.checked}; {subscriptionReport.failed} failed; {subscriptionReport.skipped} skipped.</p>
                {#if subscriptionReport.updates.length}
                  <ul>{#each subscriptionReport.updates as update}<li>{update.title}: {update.episodeCount} new episode{update.episodeCount === 1 ? '' : 's'}</li>{/each}</ul>
                  <button type="button" class="button secondary small" disabled={Boolean(busy)} onclick={acknowledgeSubscriptionUpdates}>Mark updates seen</button>
                {:else}<p>No new episodes found.</p>{/if}
              </div>
            {/if}
          {/if}
          <div class="library-toolbar">
            <div class="library-search-wrap">
              <span><Search size={14} /></span>
              <input type="text" placeholder="Search library…" bind:value={libraryQuery} />
            </div>
            <select class="library-sort-select" value={librarySort} onchange={(e) => onLibrarySortChange(e.currentTarget.value)}>
              <option value="updated">Recently updated</option>
              <option value="alpha_asc">A → Z</option>
              <option value="alpha_desc">Z → A</option>
              <option value="release_date">Release date</option>
              {#if hasScoredItems}<option value="score">Score</option>{/if}
            </select>
            {#if libraryTabFiltered.length > 0}
              <button class="button secondary small" onclick={() => (modal = 'clearLibrary')}>Clear list</button>
            {/if}
          </div>
          {#if sortedLibrary.length}<div class="media-grid">
              <Incremental items={sortedLibrary}>
                {#snippet children(saved, i)}<div class="library-card-wrapper">
                  <MediaCard
                    item={saved.item}
                    index={i}
                    onclick={() => openItem(saved.item)}
                  />
                  <div class="library-card-actions">
                    {#if saved.watchStatus && saved.watchStatus !== 'none'}<span class="status-badge {saved.watchStatus}">{saved.watchStatus.replace(/_/g, ' ')}</span>{/if}
                    {#if saved.isFavorite}<span class="icon-badge fav" title="Favorite"><Heart size={12} fill="currentColor" /></span>{/if}
                    {#if saved.isSubscribed}<span class="icon-badge sub" title="Subscribed"><Bell size={12} fill="currentColor" /></span>{/if}
                  </div>
                  <button class="library-remove-card-btn" title="Remove from library" onclick={() => removeLibraryItem(saved.id)}><X size={14} /></button>
                </div>{/snippet}
              </Incremental>
            </div>{:else}<div class="empty-state framed">
              <Bookmark size={40} strokeWidth={1} />
              {#if libraryQuery.trim()}
                <h3>No matches found.</h3>
                <p>Try a different search term.</p>
              {:else if libraryTab !== 'all'}
                <h3>Nothing in this list yet.</h3>
                <p>Change a title's status to see it here.</p>
              {:else}
                <h3>A home for your next watch.</h3>
                <p>Save a title from any source and it will be waiting here.</p>
                <button class="button primary" onclick={() => navigate('discover')}
                  >Explore your sources <ArrowRight size={16} /></button
                >
              {/if}
            </div>{/if}
        </section>
      {:else if page === 'extensions'}
        <div class="page-heading">
          <div>
            <span class="eyebrow">CONNECTED CONTENT</span>
            <h1>Extensions</h1>
            <p>Manage your Cloudstream repositories and extensions.</p>
          </div>
          <div style="display: flex; gap: 10px;">
            <button class="button secondary" onclick={() => (modal = 'localInstall')}
              ><Upload size={17} />Import local package</button
            >
            <button class="button primary" onclick={() => (modal = 'repository')}
              ><Plus size={17} />Add repository</button
            >
          </div>
        </div>
        <div class="extension-tabs">
          <button class:chosen={extensionTab === 'browse'} onclick={() => (extensionTab = 'browse')}
            >Browse repositories <span>{snapshot.repositories.length}</span></button
          ><button
            class:chosen={extensionTab === 'installed'}
            onclick={() => (extensionTab = 'installed')}
            >Installed <span>{installedCount}</span></button
          >
        </div>
        {#if extensionTab === 'browse'}
          {#if !snapshot.repositories.length}<div class="empty-state framed">
              <Globe size={40} strokeWidth={1} />
              <h3>Your sources start here.</h3>
              <p>Add an existing Cloudstream repository URL or short-code to browse its extensions.</p>
              <div style="display: flex; gap: 10px; justify-content: center; margin-top: 10px;">
                <button class="button secondary" onclick={() => (modal = 'localInstall')}
                  ><Upload size={17} />Import local package</button
                >
                <button class="button primary" onclick={() => (modal = 'repository')}
                  ><Plus size={17} />Add your first repository</button
                >
              </div>
              <button class="text-button" disabled={!!busy} onclick={addSample}
                >Or try the bundled open movie sample <ArrowRight size={15} /></button
              >
            </div>
          {:else}<div class="repository-list">
              {#each snapshot.repositories as repo}<div class="repository-row">
                  <span class="source-icon"><Globe size={22} /></span>
                  <div>
                    <strong>{repo.customName ? `${repo.customName} (${repo.manifest.name})` : repo.manifest.name}</strong><span
                      >{repo.plugins.length} extensions · {new URL(repo.url).hostname}</span
                    >
                  </div>
                  <button
                    class="icon-button"
                    aria-label={`Rename ${repo.manifest.name}`}
                    onclick={() => openRename(repo)}><Edit2 size={17} /></button
                  ><button
                    class="icon-button"
                    aria-label={`Share ${repo.manifest.name}`}
                    onclick={() => copyShareLink(repo)}><Share2 size={17} /></button
                  ><button
                    class="icon-button"
                    disabled={!!busy}
                    aria-label={`Refresh ${repo.manifest.name}`}
                    onclick={() =>
                      task('refresh', async () => {
                        await call('refresh_repository', { url: repo.url });
                        await refresh();
                        message('Repository refreshed.');
                      })}><RefreshCw size={17} /></button
                  ><button
                    class="icon-button"
                    disabled={!!busy}
                    aria-label={`Remove ${repo.manifest.name}`}
                    onclick={() =>
                      task('remove', async () => {
                        await call('remove_repository', { url: repo.url });
                        await refresh();
                        message('Repository and its extensions removed.');
                      })}><Trash2 size={17} /></button
                  >
                </div>{/each}
            </div>
            <div class="catalog-filter-bar">
              <label class="filter-input" style="flex: 1; min-width: 220px; margin-bottom: 0;">
                <Search size={17} />
                <input
                  aria-label="Filter extensions"
                  placeholder="Filter extensions by name or author"
                  bind:value={repoFilter}
                />
              </label>
              <select
                aria-label="Filter by language"
                class="filter-select"
                bind:value={languageFilter}
              >
                <option value="all">All languages</option>
                {#if sourceLanguagePrefs.length}<option value="mine">My languages</option>{/if}
                {#each availableLanguages.filter((l) => l !== 'all') as lang}
                  <option value={lang}>{lang.toUpperCase()}</option>
                {/each}
              </select>
              <div class="filter-button-group">
                <button
                  class="filter-pill"
                  class:active={typeFilter === 'all'}
                  onclick={() => (typeFilter = 'all')}
                >All types</button>
                <button
                  class="filter-pill"
                  class:active={typeFilter === 'provider'}
                  onclick={() => (typeFilter = 'provider')}
                >Providers</button>
                <button
                  class="filter-pill"
                  class:active={typeFilter === 'extractor'}
                  onclick={() => (typeFilter = 'extractor')}
                >Extractors</button>
              </div>
              <div class="filter-button-group">
                <button
                  class="filter-pill"
                  class:active={statusFilter === 'all'}
                  onclick={() => (statusFilter = 'all')}
                >All</button>
                <button
                  class="filter-pill"
                  class:active={statusFilter === 'installed'}
                  onclick={() => (statusFilter = 'installed')}
                >Installed</button>
                <button
                  class="filter-pill"
                  class:active={statusFilter === 'updates'}
                  onclick={() => (statusFilter = 'updates')}
                >Updates</button>
              </div>
            </div>
            <div class="extension-grid">
              {#each entries as { repo, entry }}{@const installed = snapshot.installed.find(
                  (p) =>
                    p.repositoryUrl === repo.url && p.entry.internalName === entry.internalName,
                )}
                <article class="extension-card">
                  <div class="extension-card-top">
                    <span class="extension-icon"><Puzzle size={25} /></span>
                    <span
                      class:android={!entry.jarUrl}
                      class="pill">{entry.jarUrl ? 'JVM' : 'Android'}</span
                    >
                  </div>
                  <h3>{entry.name}</h3>
                  {#if entry.authors?.length}
                    <span style="font-size: 11px; color: var(--subtle); margin-top: 2px;">by {entry.authors.join(', ')}</span>
                  {/if}
                  <p>{entry.description ?? `A source from ${repo.manifest.name}.`}</p>
                  <div class="extension-meta">
                    <span>{entry.language?.toUpperCase() ?? 'MULTI'}</span><span
                      >v{entry.version}</span
                    >{#if entry.jarFileSize || entry.fileSize}<span>{Math.round(((entry.jarFileSize ?? entry.fileSize ?? 0) / 1024))} KB</span>{/if}
                    {#if entry.status === 0}<span>Disabled upstream</span>{/if}
                  </div>
                  <button
                    class="button secondary full"
                    disabled={!!busy ||
                      entry.status === 0 ||
                      Boolean(installed && installed.entry.version === entry.version)}
                    onclick={() => install(repo.url, entry)}
                    >{#if busy === entry.internalName}<LoaderCircle
                        class="spin"
                        size={16}
                      />{:else if installed}<Check size={16} />{:else}<Plus
                        size={16}
                      />{/if}{installed
                      ? installed.entry.version !== entry.version
                        ? 'Update'
                        : 'Installed'
                      : 'Install'}</button
                  >
                </article>{/each}
            </div>
            {#if !entries.length}<p class="muted">No extensions match this filter.</p>{/if}{/if}
        {:else}{#if installedCount}
            <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 16px;">
              <span class="muted">{installedCount} installed {installedCount === 1 ? 'extension' : 'extensions'}</span>
              <div style="display: flex; gap: 8px;">
                {#if availableUpdatesCount > 0}
                  <button class="button primary small" onclick={updateAll} disabled={!!busy}>
                    {#if busy === 'updateAll'}<LoaderCircle class="spin" size={14} />{:else}<RefreshCw size={14} />{/if}
                    Update all ({availableUpdatesCount})
                  </button>
                {/if}
                <button class="button secondary small" onclick={checkForUpdates} disabled={!!busy}>
                  {#if busy === 'checkUpdates'}<LoaderCircle class="spin" size={14} />{:else}<RefreshCw size={14} />{/if}
                  Check for updates
                </button>
              </div>
            </div>
            <div class="installed-list">
              {#each snapshot.installed as plugin}{@const pluginProviders = (snapshot.providers ?? []).filter((p) => p.pluginId === plugin.id)}
                <article class="installed-row" style="flex-direction: column; align-items: stretch;">
                  <div style="display: flex; align-items: flex-start; gap: 16px;">
                    <span class="extension-icon"><Puzzle size={24} /></span>
                    <div class="installed-info">
                      <h3>{plugin.entry.name}</h3>
                      <p>
                        {snapshot.extensionStates?.find((s) => s.pluginId === plugin.id)?.loaded
                          ? `${plugin.entry.language?.toUpperCase() ?? 'MULTI'} · ${plugin.runtime === 'android' ? 'Desktop DEX compatibility' : 'JVM extension'} · Ready`
                          : plugin.runtime === 'android'
                            ? 'Unavailable on this desktop'
                            : `${plugin.entry.language?.toUpperCase() ?? 'MULTI'} · JVM extension · v${plugin.entry.version}`}
                      </p>
                      {#if snapshot.extensionStates?.find((s) => s.pluginId === plugin.id)?.error}<p
                          class="extension-failure"
                        >
                          {snapshot.extensionStates?.find((s) => s.pluginId === plugin.id)?.error}
                        </p>{/if}
                      <span class="verification"
                        >{#if plugin.verified}<ShieldCheck size={13} />Package hash verified{:else}<CircleHelp
                            size={13}
                          />Repository did not supply a package hash{/if}</span
                      >
                    </div>
                    <button
                      class="icon-button"
                      aria-label={`Remove ${plugin.entry.name}`}
                      disabled={!!busy}
                      onclick={() => removePlugin(plugin)}><Trash2 size={18} /></button
                    >
                  </div>
                  {#if pluginProviders.length > 0}
                    <div style="margin-top: 14px; border-top: 1px solid #282834; padding-top: 10px;">
                      <span style="font-size: 11px; text-transform: uppercase; color: var(--subtle); letter-spacing: 0.5px;">Registered Sources</span>
                      {#each pluginProviders as provider}
                        <div class="provider-toggle-row">
                          <div class="provider-meta-info">
                            <strong>{provider.name}</strong>
                            <span style="font-size: 11px; color: var(--subtle);">{provider.mainUrl}</span>
                            <div style="display: flex; gap: 6px; flex-wrap: wrap; margin-top: 4px;">
                              {#if provider.capabilities?.hasMainPage}<span class="tag-pill">Main page</span>{/if}
                              {#if provider.capabilities?.hasQuickSearch}<span class="tag-pill">Quick search</span>{/if}
                              {#if provider.capabilities?.vpnStatus && provider.capabilities.vpnStatus !== 'None'}<span class="tag-pill warning">VPN: {provider.capabilities.vpnStatus}</span>{/if}
                              {#if provider.usesWebView}<span class="tag-pill danger">Requires WebView (unsupported)</span>{/if}
                              {#if provider.cloneOf}<span class="tag-pill">Clone site</span>{/if}
                            </div>
                          </div>
                          <div style="display: flex; gap: 8px; align-items: center;">
                            <button
                              class="button small secondary"
                              aria-label={`Test ${provider.name}`}
                              onclick={() => runProviderTest(provider)}><FlaskConical size={13} />Test</button
                            >
                            <button
                              class="button small secondary"
                              aria-label={`Clone ${provider.name}`}
                              onclick={() => openClone(provider)}><Copy size={13} />Clone</button
                            >
                            <button
                              class="button small"
                              class:secondary={!provider.disabled}
                              aria-label={`Toggle ${provider.name}`}
                              onclick={() => toggleProvider(provider)}
                              >{provider.disabled ? 'Enable' : 'Disable'}</button
                            >
                          </div>
                        </div>
                      {/each}
                    </div>
                  {/if}
                </article>{/each}
            </div>{:else}<div class="empty-state framed">
              <Puzzle size={40} strokeWidth={1} />
              <h3>A little room to grow.</h3>
              <p>Your installed extensions will appear here.</p>
              <button class="button secondary" onclick={() => (extensionTab = 'browse')}
                >Browse repositories <ArrowRight size={16} /></button
              >
            </div>{/if}{/if}
        <div class="info-note">
          <CircleHelp size={18} />
          <p>
            JVM extensions and data-only .cs3 packages can run in the desktop worker. Extensions
            using Android UI, WebView or app-specific services still require additional support.
            Each installed extension shows its actual runtime status.
          </p>
        </div>
      {:else if page === 'settings'}
        <div class="page-heading">
          <div>
            <span class="eyebrow">PREFERENCES</span>
            <h1>{t('nav.settings')}</h1>
            <p>Your desktop, connected.</p>
          </div>
          <span class="pill">EARLY DEVELOPMENT</span>
        </div>
        <div class="settings-search" role="search">
          <label class="visually-hidden" for="settings-search-input">Search settings</label>
          <input
            id="settings-search-input"
            type="search"
            placeholder="Search settings (theme, subtitles, backup…)"
            bind:value={settingsQuery}
          />
          <span class="muted" role="status">{settingsQuery.trim() ? `${settingsMatches} ${settingsMatches === 1 ? 'section matches' : 'sections match'}` : ''}</span>
        </div>
        {#if settingsSections.length > 1}
          <!-- Not a <nav> element: the phone-width rules for the bottom bar style every nav button. -->
          <div class="settings-jump" role="navigation" aria-label="Settings sections">
            {#each settingsSections as section}<button type="button" onclick={() => jumpToSetting(section.id)}>{section.title}</button>{/each}
          </div>
        {/if}
        {#if settingsQuery.trim() && settingsMatches === 0}
          <div class="empty-state framed" role="status">
            <h3>No settings match “{settingsQuery.trim()}”.</h3>
            <p>Try a shorter word, or clear the search to see everything.</p>
          </div>
        {/if}
        <div class="settings-panels" bind:this={settingsPanels}>
        <section class="settings-panel" data-keywords="language locale translation region source languages setup welcome first run general">
          <div class="section-title"><h2>{t('settings.general')}</h2></div>
          <div class="preference-row">
            <div>
              <strong>{t('settings.interfaceLanguage')}</strong>
              <p>{t('settings.interfaceLanguage.hint')}</p>
            </div>
            <select
              class="filter-select"
              aria-label={t('settings.interfaceLanguage')}
              value={appearance.locale ?? 'system'}
              onchange={(e) => changeInterfaceLanguage(e.currentTarget.value)}
            >
              <option value="system">{t('settings.interfaceLanguage.system', { language: languageName(i18n.locale) })}</option>
              {#each supportedLocales as code}<option value={code}>{languageName(code)}</option>{/each}
            </select>
          </div>
          <div class="preference-row">
            <div>
              <strong>{t('settings.sourceLanguages')}</strong>
              <p>{t('settings.sourceLanguages.hint')}</p>
            </div>
            <div class="language-chips" role="group" aria-label={t('settings.sourceLanguages')}>
              {#each SOURCE_LANGUAGE_CHOICES as code}
                <button
                  type="button"
                  class="filter-pill"
                  class:active={sourceLanguagePrefs.includes(code)}
                  aria-pressed={sourceLanguagePrefs.includes(code)}
                  onclick={() => changeSourceLanguages(sourceLanguagePrefs.includes(code) ? sourceLanguagePrefs.filter((c) => c !== code) : [...sourceLanguagePrefs, code])}
                >{languageName(code)}</button>
              {/each}
            </div>
          </div>
          <div class="preference-row">
            <div>
              <strong>{t('settings.redoSetup')}</strong>
              <p>{t('settings.redoSetup.hint')}</p>
            </div>
            <button class="button secondary" onclick={() => (setupOpen = true)}>{t('settings.redoSetup')}</button>
          </div>
        </section>
        <section class="settings-panel" data-keywords="theme accent color poster size scale title badges motion animation keyboard arrow navigation density suggestions">
          <div class="section-title">
            <h2>Appearance</h2>
            <LayoutGrid size={20} />
          </div>
          <div class="preference-row">
            <div>
              <strong>Poster size</strong>
              <p>Choose how many titles fit on your screen.</p>
            </div>
            <div class="segmented-control" role="group" aria-label="Poster size">
              {#each ['comfortable', 'compact'] as density}<button
                  aria-pressed={posterDensity === density}
                  class:chosen={posterDensity === density}
                  onclick={() => setPosterDensity(density as PosterDensity)}
                  >{#if density === 'comfortable'}<LayoutGrid size={17} />{:else}<Grid2X2
                      size={17}
                    />{/if}{density === 'comfortable' ? 'Comfortable' : 'Compact'}</button
                >{/each}
            </div>
          </div>
          <div class="preference-row">
            <div>
              <strong>Search suggestions</strong>
              <p>Show query suggestions and quick search results as you type.</p>
            </div>
            <div class="segmented-control" role="group" aria-label="Search suggestions">
              <button
                type="button"
                aria-label="Search suggestions enabled"
                aria-pressed={snapshot.settings?.global?.appearance?.searchSuggestions ?? true}
                class:chosen={snapshot.settings?.global?.appearance?.searchSuggestions ?? true}
                onclick={() => updateSearchSuggestions(true)}
              >
                Enabled
              </button>
              <button
                type="button"
                aria-label="Search suggestions disabled"
                aria-pressed={!(snapshot.settings?.global?.appearance?.searchSuggestions ?? true)}
                class:chosen={!(snapshot.settings?.global?.appearance?.searchSuggestions ?? true)}
                onclick={() => updateSearchSuggestions(false)}
              >
                Disabled
              </button>
            </div>
          </div>
          <div class="preference-row">
            <div>
              <strong>Theme</strong>
              <p>Dark is the standard design. Black uses pure black surfaces and stronger borders.</p>
            </div>
            <div class="segmented-control" role="group" aria-label="Theme">
              {#each [['dark', 'Dark'], ['black', 'Black']] as [id, label]}
                <button aria-pressed={(appearance.theme ?? 'dark') === id} class:chosen={(appearance.theme ?? 'dark') === id} onclick={() => setAppearance({ theme: id as 'dark' | 'black' })}>{label}</button>
              {/each}
            </div>
          </div>
          <div class="preference-row">
            <div>
              <strong>Accent color</strong>
              <p>Used for buttons and highlights. Every choice keeps readable text.</p>
            </div>
            <div class="segmented-control" role="group" aria-label="Accent color">
              {#each ACCENTS as accent}
                <button
                  aria-pressed={(appearance.accent ?? 'indigo') === accent.id}
                  class:chosen={(appearance.accent ?? 'indigo') === accent.id}
                  onclick={() => setAppearance({ accent: accent.id })}
                  ><span class="swatch" style={`background:${accent.swatch}`} aria-hidden="true"></span>{accent.label}</button
                >
              {/each}
            </div>
          </div>
          <div class="preference-row">
            <div>
              <strong>Poster scale</strong>
              <p>Fine-tune poster width on top of the size above ({appearance.posterScale ?? 100}%).</p>
            </div>
            <input
              type="range"
              min="80"
              max="140"
              step="5"
              aria-label="Poster scale"
              value={appearance.posterScale ?? 100}
              onchange={(e) => setAppearance({ posterScale: Number(e.currentTarget.value) })}
            />
          </div>
          <div class="preference-row">
            <div>
              <strong>Title placement</strong>
              <p>Show titles under posters, or over their lower edge.</p>
            </div>
            <div class="segmented-control" role="group" aria-label="Title placement">
              {#each [['below', 'Below'], ['overlay', 'Over poster']] as [id, label]}
                <button aria-pressed={(appearance.titlePlacement ?? 'below') === id} class:chosen={(appearance.titlePlacement ?? 'below') === id} onclick={() => setAppearance({ titlePlacement: id as 'below' | 'overlay' })}>{label}</button>
              {/each}
            </div>
          </div>
          <div class="preference-row">
            <div>
              <strong>Type badges</strong>
              <p>Show Movie, Series, Anime and other type labels on posters. The type is always kept in the data.</p>
            </div>
            <div class="segmented-control" role="group" aria-label="Type badges">
              {#each [[true, 'Shown'], [false, 'Hidden']] as [value, label]}
                <button aria-pressed={(appearance.showBadges ?? true) === value} class:chosen={(appearance.showBadges ?? true) === value} onclick={() => setAppearance({ showBadges: value as boolean })}>{label}</button>
              {/each}
            </div>
          </div>
          <div class="preference-row">
            <div>
              <strong>Motion</strong>
              <p>Follow your system, or always reduce animation.</p>
            </div>
            <div class="segmented-control" role="group" aria-label="Motion">
              {#each [['system', 'Follow system'], ['reduced', 'Always reduce']] as [id, label]}
                <button aria-pressed={(appearance.motion ?? 'system') === id} class:chosen={(appearance.motion ?? 'system') === id} onclick={() => setAppearance({ motion: id as 'system' | 'reduced' })}>{label}</button>
              {/each}
            </div>
          </div>
          <div class="preference-row">
            <div>
              <strong>Keyboard navigation</strong>
              <p>With arrow-key navigation, the arrow keys move between posters and buttons, as on a remote. Press ? for all shortcuts.</p>
            </div>
            <div class="segmented-control" role="group" aria-label="Keyboard navigation">
              {#each [['pointer', 'Standard'], ['spatial', 'Arrow keys']] as [id, label]}
                <button aria-pressed={(appearance.navigation ?? 'pointer') === id} class:chosen={(appearance.navigation ?? 'pointer') === id} onclick={() => setAppearance({ navigation: id as 'pointer' | 'spatial' })}>{label}</button>
              {/each}
            </div>
          </div>
        </section>
        <section class="settings-panel subtitle-style-panel" data-keywords="subtitles captions font size color outline delay language">
          <div class="section-title"><h2>Subtitle appearance</h2><Film size={20} /></div>
          <p>Preview is approximate. Image subtitles keep their original appearance. ASS subtitles keep their authored style unless override is enabled; some ASS positioning may still differ.</p>
          <div class="subtitle-preview" aria-label="Subtitle appearance preview" style:align-items={subtitleStyle.alignY === 'top' ? 'flex-start' : subtitleStyle.alignY === 'center' ? 'center' : 'flex-end'} style:justify-content={subtitleStyle.alignX === 'left' ? 'flex-start' : subtitleStyle.alignX === 'right' ? 'flex-end' : 'center'} style:padding-block={`${Math.min(40, subtitleStyle.verticalMargin / 3)}px`}>
            <span style:font-family={previewFontFamily ?? subtitleStyle.font} style:font-size={`${Math.min(48, subtitleStyle.size)}px`} style:font-weight={subtitleStyle.bold ? 'bold' : 'normal'} style:font-style={subtitleStyle.italic ? 'italic' : 'normal'} style:color={subtitleStyle.color} style:-webkit-text-stroke={`${subtitleStyle.outlineSize / 2}px ${subtitleStyle.outlineColor}`} style:text-shadow={subtitleStyle.backgroundBox ? 'none' : `${subtitleStyle.shadowOffset}px ${subtitleStyle.shadowOffset}px 2px ${subtitlePreviewColor(subtitleStyle)}`} style:background-color={subtitleStyle.backgroundBox ? subtitlePreviewColor(subtitleStyle) : 'transparent'}>A subtitle preview · مرحبا</span>
          </div>
          {#if playerCapabilities?.capabilities.find((cap) => cap.id === 'subtitle_style' && !cap.supported)}
            <p role="status">Subtitle appearance is not supported by this mpv installation. Your settings will be saved for a compatible player.</p>
          {/if}
          <fieldset class="subtitle-style-grid" disabled={subtitleStyleSaving} aria-label="Subtitle appearance settings">
            <label>Font family<input aria-label="Subtitle font family" value={subtitleStyle.font} maxlength="120" onchange={(e) => updateSubtitleStyle({ font: e.currentTarget.value.trim(), fontId: null })} /></label>
            <label>Font size<input aria-label="Subtitle font size" type="number" min="10" max="120" step="1" value={subtitleStyle.size} onchange={(e) => updateSubtitleStyle({ size: Number(e.currentTarget.value) })} /></label>
            <label>Text color<input aria-label="Subtitle text color" type="color" value={subtitleStyle.color} onchange={(e) => updateSubtitleStyle({ color: e.currentTarget.value })} /></label>
            <label>Outline color<input aria-label="Subtitle outline color" type="color" value={subtitleStyle.outlineColor} onchange={(e) => updateSubtitleStyle({ outlineColor: e.currentTarget.value })} /></label>
            <label>Outline size<input aria-label="Subtitle outline size" type="number" min="0" max="10" step="0.05" value={subtitleStyle.outlineSize} onchange={(e) => updateSubtitleStyle({ outlineSize: Number(e.currentTarget.value) })} /></label>
            <label>Shadow offset<input aria-label="Subtitle shadow offset" type="number" min="0" max="10" step="0.5" value={subtitleStyle.shadowOffset} onchange={(e) => updateSubtitleStyle({ shadowOffset: Number(e.currentTarget.value) })} /></label>
            <label>Shadow / background color<input aria-label="Subtitle background color" type="color" value={subtitleStyle.backgroundColor} onchange={(e) => updateSubtitleStyle({ backgroundColor: e.currentTarget.value })} /></label>
            <label>Background opacity (%)<input aria-label="Subtitle background opacity" type="number" min="0" max="100" value={subtitleStyle.backgroundOpacity} onchange={(e) => updateSubtitleStyle({ backgroundOpacity: Number(e.currentTarget.value) })} /></label>
            <label>Horizontal alignment<select aria-label="Subtitle horizontal alignment" value={subtitleStyle.alignX} onchange={(e) => updateSubtitleStyle({ alignX: e.currentTarget.value as SubtitleStyle['alignX'] })}><option value="left">Left</option><option value="center">Center</option><option value="right">Right</option></select></label>
            <label>Vertical alignment<select aria-label="Subtitle vertical alignment" value={subtitleStyle.alignY} onchange={(e) => updateSubtitleStyle({ alignY: e.currentTarget.value as SubtitleStyle['alignY'] })}><option value="top">Top</option><option value="center">Center</option><option value="bottom">Bottom</option></select></label>
            <label>Vertical margin<input aria-label="Subtitle vertical margin" type="number" min="0" max="200" value={subtitleStyle.verticalMargin} onchange={(e) => updateSubtitleStyle({ verticalMargin: Number(e.currentTarget.value) })} /></label>
            <label><input aria-label="Bold subtitles" type="checkbox" checked={subtitleStyle.bold} onchange={(e) => updateSubtitleStyle({ bold: e.currentTarget.checked })} /> Bold</label>
            <label><input aria-label="Italic subtitles" type="checkbox" checked={subtitleStyle.italic} onchange={(e) => updateSubtitleStyle({ italic: e.currentTarget.checked })} /> Italic</label>
            <label><input aria-label="Subtitle background box" type="checkbox" checked={subtitleStyle.backgroundBox} onchange={(e) => updateSubtitleStyle({ backgroundBox: e.currentTarget.checked })} /> Background box</label>
            <label><input aria-label="Override ASS styles" type="checkbox" checked={subtitleStyle.overrideAss} onchange={(e) => updateSubtitleStyle({ overrideAss: e.currentTarget.checked })} /> Override ASS styles</label>
          </fieldset>
          <button type="button" class="button secondary" disabled={subtitleStyleSaving} onclick={chooseSubtitleFont}>Choose custom font file…</button>
          {#if subtitleStyle.fontId}<p>Using imported font: {subtitleStyle.font}. Some mpv versions may require a new playback to load a newly imported font.</p>{/if}
          {#if previewFontError}<p role="status">{previewFontError}</p>{/if}
          <p>Shadow and background share a color. With a background box, shadow offset controls its padding.</p>
          <div class="dialog-actions"><button type="button" class="button secondary" disabled={subtitleStyleSaving || !previousSubtitleStyle} onclick={() => previousSubtitleStyle && saveSubtitleStyle(previousSubtitleStyle)}>Undo</button><button type="button" class="button secondary" disabled={subtitleStyleSaving} onclick={() => saveSubtitleStyle(defaultSubtitleStyle())}>Reset subtitle appearance</button></div>
          <h3>Subtitle text cleanup</h3>
          <p>Applied to provider SRT, WebVTT, ASS or SSA tracks when playback starts, and when adding a local file or choosing one from a ZIP archive. Changes use a temporary copy; the original stays unchanged. Unsupported local formats cannot be added with cleanup enabled; unsupported provider tracks use their original with a notice. Embedded tracks are not cleaned.</p>
          <fieldset class="subtitle-style-grid" disabled={subtitleTransformsSaving} aria-label="Subtitle text cleanup">
            <label><input type="checkbox" checked={subtitleTransforms.removeCaptions} onchange={(e) => updateSubtitleTransforms({ removeCaptions: e.currentTarget.checked })} /> Remove square-bracketed captions</label>
            <label><input type="checkbox" checked={subtitleTransforms.removeMusic} onchange={(e) => updateSubtitleTransforms({ removeMusic: e.currentTarget.checked })} /> Remove lines marked with music notes</label>
            <label><input type="checkbox" checked={subtitleTransforms.removeBloat} onchange={(e) => updateSubtitleTransforms({ removeBloat: e.currentTarget.checked })} /> Remove web links and subtitle credits</label>
            <label><input type="checkbox" checked={subtitleTransforms.uppercase} onchange={(e) => updateSubtitleTransforms({ uppercase: e.currentTarget.checked })} /> Uppercase dialogue</label>
          </fieldset>
        </section>
        <section class="settings-panel">
          <div class="section-title">
            <h2>Playback</h2>
            <Film size={20} />
          </div>
          <div class="preference-row">
            <div><strong>AniSkip markers</strong><p>Show optional opening, ending and recap skip buttons for titles linked to MyAnimeList. The episode ID and duration are sent to AniSkip.</p></div>
            <input aria-label="AniSkip markers" type="checkbox" checked={aniskipEnabled} disabled={skipSaving} onchange={(e) => setAniskip(e.currentTarget.checked)} />
          </div>
          <div class="preference-row">
            <div><strong>Seek step</strong><p>Seconds for J/L and the back/forward buttons.</p></div>
            <input aria-label="Seek step in seconds" type="number" min="1" max="120" step="1" value={playerPreferences.seekStepSeconds ?? 10} onchange={(e) => updatePlayerPreferences({ seekStepSeconds: Number(e.currentTarget.value) })} />
          </div>
          <div class="preference-row">
            <div><strong>Speed while holding H</strong><p>With this app focused, release H to return to the previous speed.</p></div>
            <input aria-label="Hold speed" type="number" min="1" max="4" step="0.25" value={playerPreferences.holdSpeed ?? 2} onchange={(e) => updatePlayerPreferences({ holdSpeed: Number(e.currentTarget.value) })} />
          </div>
          <div class="preference-row">
            <div><strong>Source profile</strong><p>Choose how automatic playback ranks available mirrors for this local profile.</p></div>
            <select aria-label="Source profile" value={sourceConfig.activeProfileId} onchange={(e) => saveSourceConfig({ ...sourceConfig, activeProfileId: e.currentTarget.value })}>
              {#each sourceConfig.profiles as profile}<option value={profile.id}>{profile.name}</option>{/each}
            </select>
          </div>
          <div class="preference-row">
            <div><strong>New source profile</strong><p>Keep separate quality and source preferences.</p></div>
            <input aria-label="New source profile name" maxlength="60" bind:value={newSourceProfileName} />
            <button class="button secondary small" type="button" disabled={!newSourceProfileName.trim()} onclick={addSourceProfile}>Add profile</button>
            <button class="button secondary small" type="button" disabled={sourceConfig.profiles.length <= 1} onclick={removeSourceProfile}>Delete profile</button>
          </div>
          <div class="preference-row">
            <div><strong>Preferred language</strong><p>Used to break ties between equally ranked mirrors.</p></div>
            <input aria-label="Preferred stream language" value={sourceProfile.preferredLanguage ?? ''} placeholder="e.g. en" onchange={(e) => updateSourceProfile({ preferredLanguage: e.currentTarget.value.trim() || null })} />
          </div>
          <div class="preference-row">
            <div><strong>Hide low priority mirrors</strong><p>Exclude mirrors with a negative combined priority.</p></div>
            <input aria-label="Hide low priority mirrors" type="checkbox" checked={sourceProfile.hideNegativeSources} onchange={(e) => updateSourceProfile({ hideNegativeSources: e.currentTarget.checked })} />
          </div>
          <div class="preference-row">
            <div><strong>Hide failed mirrors</strong><p>Hide mirrors that failed during this playback.</p></div>
            <input aria-label="Hide failed mirrors" type="checkbox" checked={sourceProfile.hideErrorSources} onchange={(e) => updateSourceProfile({ hideErrorSources: e.currentTarget.checked })} />
          </div>
          {#each [['unknown', 4], ['144', 0], ['240', 2], ['360', 3], ['480', 4], ['720', 5], ['1080', 6], ['1440', 7], ['2160', 8]] as [quality, defaultPriority]}
            <div class="preference-row">
              <div><strong>{quality === 'unknown' ? 'Unknown or automatic' : `${quality}p`} priority</strong><p>Higher values are preferred.</p></div>
              <input aria-label={`${quality} quality priority`} type="number" min="-100" max="100" value={sourceProfile.qualityPriorities[quality] ?? defaultPriority} onchange={(e) => updateSourceProfile({ qualityPriorities: { ...sourceProfile.qualityPriorities, [quality]: Number(e.currentTarget.value) } })} />
            </div>
          {/each}
          {#each Object.entries(sourceProfile.sourcePriorities) as [name, priority]}
            <div class="preference-row">
              <div><strong>{name}</strong><p>Source priority</p></div>
              <input aria-label={`${name} source priority`} type="number" min="-100" max="100" value={priority} onchange={(e) => updateSourceProfile({ sourcePriorities: { ...sourceProfile.sourcePriorities, [name]: Number(e.currentTarget.value) } })} />
              <button class="button secondary small" type="button" aria-label={`Remove ${name} source priority`} onclick={() => removeSourcePriority(name)}>Remove</button>
            </div>
          {/each}
          <div class="preference-row">
            <div><strong>Add source priority</strong><p>Enter the source or extractor name shown on its streams.</p></div>
            <input aria-label="Source name" bind:value={newSourceName} />
            <button class="button secondary small" type="button" disabled={!newSourceName.trim()} onclick={addSourcePriority}>Add source</button>
          </div>
          <div class="preference-row">
            <div><strong>Subtitle preference</strong><p>Applied when starting a video. Choosing a track during playback updates this preference.</p></div>
            <select aria-label="Default subtitle mode" value={playerPreferences.subtitlesEnabled == null ? 'auto' : playerPreferences.subtitlesEnabled ? 'on' : 'off'} onchange={(e) => updatePlayerPreferences({ subtitlesEnabled: e.currentTarget.value === 'auto' ? null : e.currentTarget.value === 'on' })}>
              <option value="auto">Automatic</option><option value="on">On</option><option value="off">Off</option>
            </select>
          </div>
          <div class="preference-row">
            <div><strong>Preferred subtitle language</strong><p>Use a language tag such as en or pt-BR; leave blank for automatic selection.</p></div>
            <input aria-label="Preferred subtitle language" value={playerPreferences.subtitleLanguage ?? ''} maxlength="12" onchange={(e) => updatePlayerPreferences({ subtitleLanguage: e.currentTarget.value.trim() || null })} />
          </div>
          <div class="preference-row">
            <div><strong>Preferred audio language</strong><p>Applied when starting a video.</p></div>
            <input aria-label="Preferred audio language" value={playerPreferences.audioLanguage ?? ''} maxlength="12" onchange={(e) => updatePlayerPreferences({ audioLanguage: e.currentTarget.value.trim() || null, audioEnabled: null })} />
          </div>
          <div class="preference-row">
            <div><strong>Automatic mirror retries</strong><p>Try another stream when playback fails. Zero disables automatic retries.</p></div>
            <input aria-label="Automatic mirror retries" type="number" min="0" max="5" value={playbackPolicy.failoverAttempts ?? 2} onchange={(e) => updatePlaybackPolicy({ failoverAttempts: Number(e.currentTarget.value) })} />
          </div>
          <div class="preference-row">
            <div><strong>Video decoding</strong><p>Choose how mpv decodes video for new playback.</p></div>
            <select aria-label="Video decoding" value={playerPreferences.decoding ?? 'auto'} onchange={(e) => updatePlayerPreferences({ decoding: e.currentTarget.value })}><option value="auto">Automatic</option><option value="hardware">Hardware</option><option value="software">Software</option></select>
          </div>
          <div class="preference-row">
            <div><strong>Buffer memory (MiB)</strong><p>Zero uses mpv's default. Applies to new playback.</p></div>
            <input aria-label="Buffer memory in MiB" type="number" min="0" max="4096" value={playerPreferences.bufferRamMb ?? 0} onchange={(e) => updatePlayerPreferences({ bufferRamMb: Number(e.currentTarget.value) })} />
          </div>
          <div class="preference-row">
            <div><strong>Buffer duration (seconds)</strong><p>Zero uses mpv's default. Applies to new playback.</p></div>
            <input aria-label="Buffer duration in seconds" type="number" min="0" max="3600" value={playerPreferences.bufferSeconds ?? 0} onchange={(e) => updatePlayerPreferences({ bufferSeconds: Number(e.currentTarget.value) })} />
          </div>
          <div class="preference-row">
            <div><strong>Dynamic audio compression</strong><p>Reduce the difference between loud and quiet sounds.</p></div>
            <input aria-label="Dynamic audio compression" type="checkbox" checked={playerPreferences.audioCompressor ?? false} onchange={(e) => updatePlayerPreferences({ audioCompressor: e.currentTarget.checked })} />
          </div>
          {#if playerCapabilities}
            {#each playerCapabilities.capabilities.filter((cap) => !cap.supported) as cap}
              <p>{cap.label}: {cap.reason ?? 'Unsupported by this mpv installation.'}</p>
            {/each}
          {/if}
          <div class="preference-row">
            <div>
              <strong>Auto-play next episode</strong>
              <p>Automatically advance and play the next episode when the current one finishes.</p>
            </div>
            <div class="segmented-control" role="group" aria-label="Auto-play next episode">
              <button
                type="button"
                aria-label="Auto-play next episode enabled"
                aria-pressed={playbackPolicy.autoNext}
                class:chosen={playbackPolicy.autoNext}
                onclick={() => updatePlaybackPolicy({ autoNext: true })}
              >
                Enabled
              </button>
              <button
                type="button"
                aria-label="Auto-play next episode disabled"
                aria-pressed={!playbackPolicy.autoNext}
                class:chosen={!playbackPolicy.autoNext}
                onclick={() => updatePlaybackPolicy({ autoNext: false })}
              >
                Disabled
              </button>
            </div>
          </div>
          <div class="preference-row">
            <div>
              <strong>Start video paused</strong>
              <p>Open episodes in mpv paused rather than immediately playing.</p>
            </div>
            <div class="segmented-control" role="group" aria-label="Start video paused">
              <button
                type="button"
                aria-label="Start paused enabled"
                aria-pressed={playbackPolicy.startPaused}
                class:chosen={playbackPolicy.startPaused}
                onclick={() => updatePlaybackPolicy({ startPaused: true })}
              >
                Enabled
              </button>
              <button
                type="button"
                aria-label="Start paused disabled"
                aria-pressed={!playbackPolicy.startPaused}
                class:chosen={!playbackPolicy.startPaused}
                onclick={() => updatePlaybackPolicy({ startPaused: false })}
              >
                Disabled
              </button>
            </div>
          </div>
          <div class="preference-row">
            <div>
              <strong>Prefetch next episode</strong>
              <p>Resolve upcoming links in background for instantaneous transitions.</p>
            </div>
            <div class="segmented-control" role="group" aria-label="Prefetch next episode">
              <button
                type="button"
                aria-label="Prefetch next episode enabled"
                aria-pressed={playbackPolicy.prefetchNext}
                class:chosen={playbackPolicy.prefetchNext}
                onclick={() => updatePlaybackPolicy({ prefetchNext: true })}
              >
                Enabled
              </button>
              <button
                type="button"
                aria-label="Prefetch next episode disabled"
                aria-pressed={!playbackPolicy.prefetchNext}
                class:chosen={!playbackPolicy.prefetchNext}
                onclick={() => updatePlaybackPolicy({ prefetchNext: false })}
              >
                Disabled
              </button>
            </div>
          </div>
        </section>
        <section class="settings-panel">
          <div class="section-title">
            <h2>Profiles & Security</h2>
            <button class="text-button" onclick={openProfileManager}>
              <Users size={15} />Manage profiles
            </button>
          </div>
          <div class="preference-row">
            <div>
              <strong>Auto-select profile on startup</strong>
              <p>Automatically select the default profile when CloudStream starts.</p>
            </div>
            <div class="segmented-control" role="group" aria-label="Auto-select profile">
              <button
                type="button"
                aria-label="Auto-select profile enabled"
                aria-pressed={snapshot.settings?.global?.autoSelectProfile ?? true}
                class:chosen={snapshot.settings?.global?.autoSelectProfile ?? true}
                onclick={() => updateAutoSelectProfile(true)}
              >
                Enabled
              </button>
              <button
                type="button"
                aria-label="Auto-select profile disabled"
                aria-pressed={!(snapshot.settings?.global?.autoSelectProfile ?? true)}
                class:chosen={!(snapshot.settings?.global?.autoSelectProfile ?? true)}
                onclick={() => updateAutoSelectProfile(false)}
              >
                Disabled
              </button>
            </div>
          </div>
          <div class="preference-row">
            <div>
              <strong>Default startup profile</strong>
              <p>Profile used when auto-selecting on launch.</p>
            </div>
            <select
              aria-label="Default startup profile"
              class="filter-select"
              value={snapshot.settings?.global?.defaultProfileId ?? 'default'}
              onchange={(e) => updateDefaultProfile(e.currentTarget.value)}
            >
              {#each (snapshot.profiles || []) as prof}
                <option value={prof.id}>{prof.name}</option>
              {/each}
            </select>
          </div>
        </section>
        <MetadataSettings onchange={() => void refresh()} />
        {#key activeProfile}<SubtitleServicesSettings profileId={activeProfile} />{/key}
        <ExternalPlayerSettings onchange={(value) => (externalPlayer = value)} />
        <Backup profileId={activeProfile} onapplied={() => void refresh()} />
        <Diagnostics />
        <section class="settings-panel">
          <div class="section-title">
            <h2>Runtime health</h2>
            <button class="text-button" disabled={!!busy} onclick={() => task('health', refresh)}
              ><RefreshCw size={15} />Check again</button
            >
          </div>
          {#each [{ name: 'Extension worker', description: 'Runs cross-platform Cloudstream JARs', ok: snapshot.status.workerAvailable && snapshot.status.jvmAvailable, icon: Puzzle }, { name: 'mpv player', description: 'Native playback in a dedicated window', ok: snapshot.status.mpvAvailable, icon: MonitorPlay }, { name: 'Full Android runtime', description: 'Android UI and WebView · in development', ok: snapshot.status.androidAvailable, icon: Layers }] as runtime}<div
              class="runtime-row"
            >
              <runtime.icon size={22} />
              <div>
                <strong>{runtime.name}</strong>
                <p>{runtime.description}</p>
              </div>
              <span class="runtime-state" class:healthy={runtime.ok}
                >{#if runtime.ok}<CheckCircle2 size={15} />Ready{:else}<span class="small-dot"
                  ></span>{runtime.name === 'Full Android runtime'
                    ? 'In development'
                    : 'Unavailable'}{/if}</span
              >
            </div>{/each}
        </section>
        <section class="settings-panel">
          <h2>About this build</h2>
          <p>
            Cloudstream Desktop is an independent Linux client built with Tauri, Rust and the
            upstream Cloudstream extension library. This is an early development build.
          </p>
          <div class="about-grid">
            <div><span>Version</span><strong>0.1.0</strong></div>
            <div><span>Installed extensions</span><strong>{installedCount}</strong></div>
            <div><span>Available providers</span><strong>{providers.length}</strong></div>
            <div><span>License</span><strong>GPL-3.0</strong></div>
          </div>
          <div class="info-note">
            <HardDrive size={19} />
            <p>
              Your repositories, bookmarks and watch progress are saved locally. Playback uses your
              installed mpv player.
            </p>
          </div>
          <h3>Project and support</h3>
          <div class="link-list">
            {#each projectLinks as link}<button class="button secondary small" onclick={() => openProjectLink(link.id)}><ArrowUpRight size={14} />{LINK_LABELS[link.id] ?? link.id}</button>{/each}
          </div>
          <p class="muted">Opens in your web browser. Please do not use extensions that host copyrighted media; see the extension policy link.</p>
          <h3>Licenses</h3>
          <div class="link-list">
            <button class="button secondary small" onclick={() => showLegal('license', 'GPL-3.0 license')}>This app’s license</button>
            <button class="button secondary small" onclick={() => showLegal('notices', 'Notices')}>Notices</button>
            <button class="button secondary small" onclick={() => showLegal('third-party', 'Third-party licenses')}>Third-party licenses</button>
          </div>
          {#if legalDoc}
            <div class="legal-block">
              <div class="link-list"><strong>{legalDoc.name}</strong><button class="button secondary small" onclick={() => (legalDoc = null)}>Close</button></div>
              <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
              <!-- A scrollable region must be focusable so keyboard users can scroll it. -->
              <div class="legal-text" tabindex="0" role="region" aria-label={legalDoc.name}>{legalDoc.text}</div>
            </div>
          {/if}
        </section>
        <section class="settings-panel" data-keywords="accounts sign in login tracking anilist myanimelist kitsu simkl opensubtitles subdl">
          <h2>Accounts</h2>
          <p>
            Subtitle services (OpenSubtitles, Subdl, SubSource) are connected per profile under Online subtitles.
            Tracking services (AniList, MyAnimeList, Kitsu, Simkl) are not available in this version, so there is nothing
            to sign in to for them yet. Your library and progress are kept on this computer, and you can move them with
            Backup &amp; restore above.
          </p>
        </section>
        <section class="settings-panel">
          <h2>Try the compatibility sample</h2>
          <p>
            Install a small JVM extension with official Blender open movies. It uses the same worker
            interface as other Cloudstream JAR extensions.
          </p>
          <button class="button secondary" disabled={!!busy} onclick={addSample}
            ><Play size={16} />Install open movie sample</button
          >
          <button class="button secondary" disabled={!!busy} onclick={addTorrentio} style="margin-inline-start: 10px;"
            ><Sparkles size={16} />Install Torrentio (P2P)</button
          >
        </section>
        </div>
      {/if}
    </main>
    <footer class="main-footer">
      <span
        ><span class="connection-dot" class:online={providers.length > 0}></span>{providers.length}
        {providers.length === 1 ? 'source' : 'sources'} connected</span
      ><span>CloudStream Desktop<span class="footer-divider">·</span>Open source</span>
    </footer>
  </div>
</div>

{#if playerState?.active}<div class="player-bar">
    <span class="playing-icon"><Film size={20} /></span>
    <div>
      <strong>{playerState.itemName ?? 'Playing'}</strong>
      <span>
        {#if playerState.episodeName}
          {playerState.episodeName}
        {:else}
          {playerState.paused ? 'Paused in mpv' : 'Playing in mpv'}
        {/if}
        {#if playerState.queueCount && playerState.queueCount > 1}
          <span class="queue-badge">
            Episode {(playerState.queueIndex ?? 0) + 1} of {playerState.queueCount}
          </span>
        {/if}
      </span>
    </div>
    {#if !playerLocked}
    <details class="player-options">
      <summary>Controls</summary>
      <div class="player-options-panel">
        <label>Position {formatTime(playerState.position)} / {formatTime(playerState.duration)}
          <input aria-label="Playback position" type="range" min="0" max={playerState.duration || 0} step="1" value={playerState.position} disabled={!playerState.duration} onchange={(e) => sendPlayerCommand({ type: 'seek_absolute', seconds: Number(e.currentTarget.value) })} />
        </label>
        <label>Volume {Math.round(playerState.volume)}%
          <input aria-label="Volume" type="range" min="0" max="130" step="1" value={playerState.volume} onchange={(e) => sendPlayerCommand({ type: 'set_volume', volume: Number(e.currentTarget.value) })} />
        </label>
        <button onclick={() => sendPlayerCommand({ type: 'set_mute', muted: !playerState?.muted })}>{playerState.muted ? 'Unmute' : 'Mute'}</button>
        <label>Speed
          <select value={playerState.speed} onchange={(e) => sendPlayerCommand({ type: 'set_speed', speed: Number(e.currentTarget.value) })}>
            {#each [0.25, 0.5, 0.75, 1, 1.25, 1.5, 2, 3, 4] as speed}<option value={speed}>{speed}×</option>{/each}
          </select>
        </label>
        <label>Picture
          <select value={playerState.aspect ?? 'fit'} onchange={(e) => sendPlayerCommand({ type: 'set_aspect', mode: e.currentTarget.value as 'fit' | 'fill' | 'stretch' | '16:9' | '4:3' | '21:9' })}>
            <option value="fit">Fit</option><option value="fill">Fill</option><option value="stretch">Stretch</option><option value="16:9">16:9</option><option value="4:3">4:3</option><option value="21:9">21:9</option>{#if playerState.aspect === 'custom'}<option value="custom" disabled>Custom</option>{/if}
          </select>
        </label>
        {#if playerState.tracks?.some((track) => track.kind === 'audio')}
          <label>Audio track
            <select aria-label="Audio track" value={playerState.tracks.find((track) => track.kind === 'audio' && track.selected)?.id ?? 'off'} onchange={(e) => sendPlayerCommand({ type: 'set_audio_track', id: e.currentTarget.value === 'off' ? null : Number(e.currentTarget.value) })}>
              <option value="off">Off</option>
              {#each playerState.tracks.filter((track) => track.kind === 'audio') as track (track.id)}
                <option value={track.id}>{track.title || track.language || `Audio ${track.id}`}{track.codec ? ` · ${track.codec}` : ''}{track.external ? ' · External' : ''}</option>
              {/each}
            </select>
          </label>
        {/if}
        {#if playerState.tracks?.some((track) => track.kind === 'sub')}
          <label>Subtitles
            <select aria-label="Subtitle track" value={playerState.tracks.find((track) => track.kind === 'sub' && track.selected)?.id ?? 'off'} onchange={(e) => sendPlayerCommand({ type: 'set_subtitle_track', id: e.currentTarget.value === 'off' ? null : Number(e.currentTarget.value) })}>
              <option value="off">Off</option>
              {#each playerState.tracks.filter((track) => track.kind === 'sub') as track (track.id)}
                <option value={track.id}>{track.title || track.language || `Subtitle ${track.id}`}{track.codec ? ` · ${track.codec}` : ''}{track.forced ? ' · Forced' : ''}{track.hearingImpaired ? ' · SDH' : ''}{track.external ? ' · External' : ''}</option>
              {/each}
            </select>
          </label>
        {/if}
        {#each providerSubtitleArchives as archive (archive.index)}
          <button type="button" disabled={!!busy} onclick={() => chooseProviderArchive(archive.index)}>Open {archive.subtitle.lang || 'provider'} subtitle archive…</button>
        {/each}
        <button type="button" disabled={!playerState.sessionId} onclick={chooseLocalSubtitle}>Add subtitle file…</button>
        <button type="button" disabled={!playerState.sessionId} onclick={() => (onlineSubtitlesOpen = true)}>Search subtitles online…</button>
        {#if onlineSubtitlesOpen && playerState.sessionId}
          <OnlineSubtitles sessionId={playerState.sessionId} onclose={() => (onlineSubtitlesOpen = false)} onadded={() => message('Subtitle added.')} onarchive={(archive) => { subtitleArchive = archive; modal = 'subtitleArchive'; }} />
        {/if}
        <label>Audio delay (seconds)
          <input type="number" min="-30" max="30" step="0.1" value={playerState.audioDelay ?? 0} onchange={(e) => sendPlayerCommand({ type: 'set_audio_delay', seconds: Number(e.currentTarget.value) })} />
        </label>
        <label>Subtitle delay (seconds)
          <input type="number" min="-30" max="30" step="0.1" value={playerState.subtitleDelay ?? 0} onchange={(e) => sendPlayerCommand({ type: 'set_subtitle_delay', seconds: Number(e.currentTarget.value) })} />
        </label>
        <label>Rotation
          <select value={playerState.rotation ?? 0} onchange={(e) => sendPlayerCommand({ type: 'set_rotation', degrees: Number(e.currentTarget.value) })}><option value="0">0°</option><option value="90">90°</option><option value="180">180°</option><option value="270">270°</option></select>
        </label>
        <button onclick={() => sendPlayerCommand({ type: 'set_fullscreen', fullscreen: !playerState?.fullscreen })}>{playerState.fullscreen ? 'Exit fullscreen' : 'Fullscreen'}</button>
      </div>
    </details>
    {#each skipMarkers.filter((marker) => aniskipEnabled && playerState && skipMarkerSession === playerState.sessionId && playerState.position >= marker.start && playerState.position < marker.end && marker.end <= playerState.duration) as marker (`${marker.kind}:${marker.start}:${marker.end}`)}
      <button class="button secondary small" title={`Timing from ${marker.source}`} onclick={() => skipMarker(marker)}>{skipLabel(marker.kind)}</button>
    {/each}
    <button
      class="player-btn"
      aria-label="Previous episode"
      disabled={!playerState.hasPrev}
      onclick={() => playerAction('prev')}
    ><SkipBack size={18} /></button>
    <button
      class="player-btn"
      aria-label={`Seek back ${playerPreferences.seekStepSeconds ?? 10} seconds`}
      onclick={() => sendPlayerCommand({ type: 'seek_relative', seconds: -(playerPreferences.seekStepSeconds ?? 10) })}
    ><RotateCcw size={17} /></button>
    <button
      class="player-btn primary"
      aria-label={playerState.paused ? 'Resume playback' : 'Pause playback'}
      onclick={() => playerAction(playerState?.paused ? 'play' : 'pause')}
    >{#if playerState.paused}<Play size={20} />{:else}<Pause size={20} />{/if}</button>
    <button
      class="player-btn"
      aria-label={`Seek forward ${playerPreferences.seekStepSeconds ?? 10} seconds`}
      onclick={() => sendPlayerCommand({ type: 'seek_relative', seconds: playerPreferences.seekStepSeconds ?? 10 })}
    ><RotateCw size={17} /></button>
    <button
      class="player-btn"
      aria-label="Next episode"
      disabled={!playerState.hasNext}
      onclick={() => playerAction('next')}
    ><SkipForward size={18} /></button>
    <button
      class="player-btn"
      aria-label="Stop playback"
      onclick={() => playerAction('stop')}
    ><Square size={16} /></button>
    {/if}
    {#if playerLocked}<span>App controls locked</span>{/if}
    <button class="player-btn" aria-label={playerLocked ? 'Unlock player controls' : 'Lock player controls'} aria-pressed={playerLocked} onclick={() => { endSpeedHold(); playerLocked = !playerLocked; }}>
      {#if playerLocked}<LockKeyholeOpen size={16} />{:else}<LockKeyhole size={16} />{/if}
    </button>
  </div>{/if}
{#if setupOpen && snapshot.settings}
  <FirstRun settings={snapshot.settings} onchange={(updated) => (snapshot.settings = updated)} onfinish={finishSetup} />
{/if}
{#if shortcutsOpen}
  <dialog bind:this={shortcutsDialog} use:openModal class="app-dialog" aria-labelledby="shortcuts-title" onclose={() => (shortcutsOpen = false)}>
    <h2 id="shortcuts-title">Keyboard shortcuts</h2>
    <dl class="shortcut-list">
      <dt>/</dt><dd>Search</dd>
      <dt>Ctrl + K</dt><dd>Focus the search box</dd>
      <dt>Esc</dt><dd>Close a dialog or go back from a title</dd>
      <dt>?</dt><dd>Show this list</dd>
      <dt>Space or K</dt><dd>Pause or resume playback</dd>
      <dt>J / L</dt><dd>Back / forward {playerPreferences.seekStepSeconds ?? 10} seconds</dd>
      <dt>Hold H</dt><dd>Temporarily play at {playerPreferences.holdSpeed ?? 2}× speed</dd>
      <dt>Esc</dt><dd>Unlock player controls when locked</dd>
      <dt>M</dt><dd>Mute</dd>
      <dt>F</dt><dd>Fullscreen</dd>
      <dt>&lt; / &gt;</dt><dd>Slower / faster</dd>
      <dt>Arrow keys</dt><dd>Move between posters and buttons (when turned on in Appearance)</dd>
    </dl>
    <p class="muted">Playback keys work when no button or text box is focused and something is playing.</p>
    <div style="display:flex;justify-content:flex-end;margin-top:1rem"><button class="button primary" onclick={() => (shortcutsOpen = false)}>Close</button></div>
  </dialog>
{/if}
{#if exitPrompt}
  <dialog bind:this={exitDialog} use:openModal class="app-dialog" aria-labelledby="exit-title" onclose={() => (exitPrompt = null)}>
    <h2 id="exit-title">Quit Cloudstream Desktop?</h2>
    <p>
      {#if exitPrompt.playing && exitPrompt.downloads > 0}Something is playing and {exitPrompt.downloads} {exitPrompt.downloads === 1 ? 'download is' : 'downloads are'} running.
      {:else if exitPrompt.playing}Something is still playing.
      {:else}{exitPrompt.downloads} {exitPrompt.downloads === 1 ? 'download is' : 'downloads are'} still running.{/if}
      Quitting stops {exitPrompt.playing ? 'playback' : ''}{exitPrompt.playing && exitPrompt.downloads > 0 ? ' and ' : ''}{exitPrompt.downloads > 0 ? 'the downloads (they can be resumed next time)' : ''}.
    </p>
    <div style="display:flex;gap:0.5rem;justify-content:flex-end;margin-top:1rem;flex-wrap:wrap">
      <button class="button secondary" onclick={() => (exitPrompt = null)}>Keep it open</button>
      <button class="button primary" onclick={quitNow}>Quit</button>
    </div>
  </dialog>
{/if}
{#if entryConfirm}
  <EntryPrompt
    action={entryConfirm}
    latest={snapshot.history[0] ?? null}
    busy={entryBusy}
    onconfirm={confirmEntry}
    oncancel={cancelEntry}
  />
{/if}
{#if notice}<div class="toast" class:above-player={playerState?.active} role="status">
    <CheckCircle2 size={18} />{notice}<button
      aria-label="Dismiss notification"
      onclick={() => (notice = '')}><X size={16} /></button
    >
  </div>{/if}

<dialog
  bind:this={dialogElement}
  onclose={() => { cancelSubtitleArchive(); modal = null; }}
  oncancel={() => { cancelSubtitleArchive(); modal = null; }}
  class="app-dialog"
>
  <div class="dialog-heading">
    <span class="extension-icon"
      >{#if modal === 'repository' || modal === 'rename'}<Globe size={25} />{:else if modal === 'localInstall' || modal === 'clone'}<Puzzle size={25} />{:else if modal === 'clearLibrary' || modal === 'clearHistory' || modal === 'deleteProfile'}<Trash2 size={25} />{:else if modal === 'profiles' || modal === 'profileForm'}<Users size={25} />{:else if modal === 'profilePin'}<Lock size={25} />{:else}<Link size={25} />{/if}</span
    ><button class="icon-button" aria-label="Close dialog" onclick={() => (modal = null)}
      ><X size={20} /></button
    >
  </div>
  <h2>
    {modal === 'subtitleArchive'
      ? 'Choose a subtitle'
      : modal === 'repository'
      ? 'Bring your sources along.'
      : modal === 'rename'
        ? 'Rename repository'
        : modal === 'localInstall'
          ? 'Import local extension'
          : modal === 'clone'
            ? 'Configure clone site'
            : modal === 'test'
              ? 'Source Self-Tester'
              : modal === 'clearLibrary'
                ? 'Clear library'
                : modal === 'clearHistory'
                  ? 'Clear watch history'
                  : modal === 'profiles'
                    ? 'Manage profiles'
                    : modal === 'profileForm'
                      ? (editingProfile ? 'Edit profile' : 'Create profile')
                      : modal === 'profilePin'
                        ? 'Profile PIN required'
                        : modal === 'deleteProfile'
                          ? 'Delete profile'
                          : 'Something ready to play?'}
  </h2>
  <p>
    {modal === 'subtitleArchive'
      ? 'Choose a subtitle file from the archive.'
      : modal === 'repository'
      ? 'Paste an existing Cloudstream repository URL or short-code (!code, cutt.ly/code).'
      : modal === 'rename'
        ? 'Set a custom display name for this repository.'
        : modal === 'localInstall'
          ? 'Enter the path to a local .jar or .cs3 extension on your computer.'
          : modal === 'clone'
            ? 'Configure an independent mirror or domain clone for this source without mutating the original.'
            : modal === 'test'
              ? 'Verifies provider capabilities across Home, Search, Details, and Stream link resolution.'
              : modal === 'clearLibrary'
                ? 'Choose whether to clear titles in the current tab or your entire saved library.'
                : modal === 'clearHistory'
                  ? 'Are you sure you want to clear your local watch history? This cannot be undone.'
                  : modal === 'profiles'
                    ? 'Switch between profiles or manage preferences on this device.'
                    : modal === 'profileForm'
                      ? 'Customize profile details, avatar, and optional PIN gate.'
                      : modal === 'profilePin'
                        ? 'Enter the PIN to unlock and access this profile.'
                        : modal === 'deleteProfile'
                          ? 'Confirm profile deletion.'
                          : 'Open a direct HTTP or HTTPS media URL in your native player.'}
  </p>
  {#if error}<div class="dialog-error" role="alert">{error}</div>{/if}
  {#if modal === 'subtitleArchive' && subtitleArchive}
    <div class="subtitle-archive-list">
      {#each subtitleArchive.entries as entry (entry.index)}
        <button class="button secondary" type="button" disabled={!!busy} onclick={() => selectArchivedSubtitle(entry.index)}>{entry.name} <span>({Math.ceil(entry.size / 1024)} KiB)</span></button>
      {/each}
    </div>
    <div class="dialog-actions"><button class="button secondary" type="button" onclick={() => { cancelSubtitleArchive(); modal = null; }}>Cancel</button></div>
  {:else if modal === 'repository'}<form onsubmit={addRepository}>
      <label for="repo-url">Repository URL</label><input
        id="repo-url"
        bind:value={repoUrl}
        placeholder="https://example.com/repo.json or !shortcode"
        required
      /><span class="field-help">Repository URLs, cloudstreamrepo:// links, and short-codes (!code, cutt.ly) are supported.</span>
      <div class="dialog-actions">
        <button type="button" class="button secondary" onclick={() => (modal = null)}>Cancel</button
        ><button class="button primary" disabled={!!busy || !repoUrl.trim()}
          >{#if busy === 'repository'}<LoaderCircle class="spin" size={16} />{:else}<Plus
              size={16}
            />{/if}Add repository</button
        >
      </div>
    </form>
  {:else if modal === 'rename'}<form onsubmit={submitRename}>
      <label for="repo-rename-input">Display Name</label><input
        id="repo-rename-input"
        bind:value={renameValue}
        placeholder="My Custom Repo Name"
        required
      />
      <div class="dialog-actions">
        <button type="button" class="button secondary" onclick={() => (modal = null)}>Cancel</button
        ><button class="button primary" disabled={!!busy || !renameValue.trim()}>Save name</button>
      </div>
    </form>
  {:else if modal === 'localInstall'}<form onsubmit={submitLocalInstall}>
      <label for="local-package-input">Local Package Path (.jar or .cs3)</label><input
        id="local-package-input"
        bind:value={localPackagePath}
        placeholder="/home/.../extension.jar"
        required
      /><span class="field-help">Validates package integrity and loads the extension into the desktop runtime.</span>
      <div class="dialog-actions">
        <button type="button" class="button secondary" onclick={() => (modal = null)}>Cancel</button
        ><button class="button primary" disabled={!!busy || !localPackagePath.trim()}
          >{#if busy === 'localInstall'}<LoaderCircle class="spin" size={16} />{:else}<Upload
              size={16}
            />{/if}Install package</button
        >
      </div>
    </form>
  {:else if modal === 'clone'}<form onsubmit={submitClone}>
      <label for="clone-name-input">Clone Site Name</label><input
        id="clone-name-input"
        bind:value={cloneName}
        placeholder="Mirror 2"
        required
      />
      <label for="clone-url-input">Custom Mirror URL</label><input
        id="clone-url-input"
        bind:value={cloneUrl}
        placeholder="https://mirror.example.com"
        required
      /><span class="field-help">Creates an independent source identity that does not alter the original provider.</span>
      <div class="dialog-actions">
        <button type="button" class="button secondary" onclick={() => (modal = null)}>Cancel</button
        ><button class="button primary" disabled={!!busy || !cloneName.trim() || !cloneUrl.trim()}
          >Create clone</button
        >
      </div>
    </form>
  {:else if modal === 'test'}
    <div style="display: flex; flex-direction: column; gap: 14px; margin: 16px 0;">
      <div style="display: flex; align-items: center; gap: 10px;">
        <FlaskConical size={20} />
        <div>
          <strong>Testing: {testingProvider?.name}</strong>
          <span style="display: block; font-size: 12px; color: var(--subtle);">{testingProvider?.mainUrl}</span>
        </div>
      </div>
      {#if busy === 'testing' && !testReport}
        <div style="display: flex; align-items: center; gap: 10px; padding: 20px 0; justify-content: center;">
          <LoaderCircle class="spin" size={24} />
          <span>Executing test stages (Home, Search, Details, Links)…</span>
        </div>
      {/if}
      {#if testReport}
        <div style="display: grid; gap: 10px;">
          {#each testReport.stages as stage}
            <div style="display: flex; justify-content: space-between; align-items: center; background: #1a1a22; padding: 10px 14px; border-radius: 8px; border: 1px solid #2e2e3a;">
              <div>
                <strong style="text-transform: capitalize;">{stage.stage} Stage</strong>
                {#if stage.message}<span style="display: block; font-size: 11px; color: var(--subtle); margin-top: 2px;">{stage.message}</span>{/if}
              </div>
              <div style="display: flex; align-items: center; gap: 10px;">
                <span style="font-size: 11px; color: var(--subtle);">{stage.durationMs}ms</span>
                <span class="pill" class:success={stage.status === 'passed'} class:danger={stage.status === 'failed'} class:warning={stage.status === 'skipped'}>
                  {stage.status.toUpperCase()}
                </span>
              </div>
            </div>
          {/each}
          <div style="display: flex; justify-content: space-between; align-items: center; padding-top: 10px; border-top: 1px solid #2e2e3a; font-size: 13px;">
            <span>Total Duration: {testReport.totalDurationMs}ms</span>
            <span class="pill" class:success={testReport.passed} class:danger={!testReport.passed}>
              {testReport.passed ? 'ALL PASSED' : 'TESTS FAILED'}
            </span>
          </div>
        </div>
      {/if}
      <div class="dialog-actions">
        <button type="button" class="button secondary" onclick={cancelProviderTest}>
          {busy === 'testing' ? 'Cancel test' : 'Close'}
        </button>
      </div>
    </div>
  {:else if modal === 'clearLibrary'}
    <div style="display: flex; flex-direction: column; gap: 14px; margin: 16px 0;">
      {#if ['watching', 'plan_to_watch', 'completed', 'on_hold', 'dropped'].includes(libraryTab)}
        <p>You can clear only items with status "<strong>{libraryTab.replace(/_/g, ' ')}</strong>", or clear all saved library titles.</p>
        <div class="dialog-actions">
          <button type="button" class="button secondary" onclick={() => (modal = null)}>Cancel</button>
          <button type="button" class="button secondary" onclick={() => confirmClearLibrary('tab')}>
            Clear "{libraryTab.replace(/_/g, ' ')}"
          </button>
          <button type="button" class="button danger" onclick={() => confirmClearLibrary('all')}>
            <Trash2 size={16} />Clear all
          </button>
        </div>
      {:else}
        <p>This will remove all saved titles from your local library.</p>
        <div class="dialog-actions">
          <button type="button" class="button secondary" onclick={() => (modal = null)}>Cancel</button>
          <button type="button" class="button danger" onclick={() => confirmClearLibrary('all')}>
            <Trash2 size={16} />Clear entire library
          </button>
        </div>
      {/if}
    </div>
  {:else if modal === 'clearHistory'}
    <div style="display: flex; flex-direction: column; gap: 14px; margin: 16px 0;">
      <p>Are you sure you want to clear your continue watching history? All progress items will be removed from this device.</p>
      <div class="dialog-actions">
        <button type="button" class="button secondary" onclick={() => (modal = null)}>Cancel</button>
        <button type="button" class="button danger" onclick={confirmClearHistory}>
          <Trash2 size={16} />Clear history
        </button>
      </div>
    </div>
  {:else if modal === 'profiles'}
    <div class="profile-management-modal">
      <div class="profile-cards-list">
        {#each (snapshot.profiles || []) as p}
          <div class="profile-list-item" class:active-profile-card={p.id === activeProfile}>
            <div class="profile-item-left">
              <span class="profile avatar-badge">
                {#if p.avatar === 'popcorn'}🍿
                {:else if p.avatar === 'film'}🎬
                {:else if p.avatar === 'sparkles'}✨
                {:else if p.avatar === 'heart'}❤️
                {:else if p.avatar === 'lock'}🔒
                {:else}{p.name ? p.name.charAt(0).toUpperCase() : 'P'}
                {/if}
              </span>
              <div>
                <div class="profile-name-row">
                  <strong>{p.name}</strong>
                  {#if p.hasPin}
                    <span class="pin-indicator" title="PIN protected">🔒</span>
                  {/if}
                  {#if p.id === activeProfile}
                    <span class="pill">Current</span>
                  {/if}
                </div>
                <span class="field-help">{p.id === 'default' ? 'Default profile' : 'Local profile'}</span>
              </div>
            </div>
            <div class="profile-item-actions">
              {#if p.id !== activeProfile}
                <button
                  type="button"
                  class="button secondary small"
                  onclick={() => requestSwitchProfile(p.id)}
                >
                  Switch
                </button>
              {/if}
              <button
                type="button"
                class="icon-button"
                aria-label={`Edit profile ${p.name}`}
                onclick={() => startEditProfile(p)}
              >
                <Edit2 size={16} />
              </button>
              {#if (snapshot.profiles?.length ?? 0) > 1}
                <button
                  type="button"
                  class="icon-button"
                  aria-label={`Delete profile ${p.name}`}
                  onclick={() => startDeleteProfile(p.id)}
                >
                  <Trash2 size={16} />
                </button>
              {/if}
            </div>
          </div>
        {/each}
      </div>
      <div class="dialog-actions" style="margin-top: 16px;">
        <button type="button" class="button secondary" onclick={() => (modal = null)}>Close</button>
        <button type="button" class="button primary" onclick={startCreateProfile}>
          <Plus size={16} />Add new profile
        </button>
      </div>
    </div>
  {:else if modal === 'profileForm'}
    <form onsubmit={(e) => { e.preventDefault(); void saveProfileForm(); }}>
      <label for="profile-name-input">Profile Name</label>
      <input
        id="profile-name-input"
        bind:value={profileNameInput}
        placeholder="e.g. Living Room, Kids, Anime"
        required
      />

      <span class="field-label">Choose Avatar</span>
      <div class="avatar-picker" role="radiogroup" aria-label="Avatar options">
        {#each [
          { id: 'default', label: 'Default', icon: '👤' },
          { id: 'popcorn', label: 'Popcorn', icon: '🍿' },
          { id: 'film', label: 'Film', icon: '🎬' },
          { id: 'sparkles', label: 'Sparkles', icon: '✨' },
          { id: 'heart', label: 'Heart', icon: '❤️' },
          { id: 'lock', label: 'Lock', icon: '🔒' }
        ] as av}
          <button
            type="button"
            class="avatar-option"
            class:selected={profileAvatarInput === av.id}
            onclick={() => (profileAvatarInput = av.id)}
            title={av.label}
            aria-label={av.label}
          >
            {av.icon}
          </button>
        {/each}
      </div>

      <label for="profile-pin-input">PIN Gate (Optional 4-digit code)</label>
      <input
        id="profile-pin-input"
        type="password"
        maxlength="8"
        bind:value={profilePinInput}
        placeholder={editingProfile?.hasPin ? 'Leave blank to keep existing PIN' : 'Optional numeric PIN'}
      />
      {#if editingProfile?.hasPin}
        <label style="display: flex; align-items: center; gap: 8px; margin-top: 8px; cursor: pointer;">
          <input type="checkbox" bind:checked={profileRemovePin} />
          <span>Remove PIN security</span>
        </label>
      {/if}

      <div class="dialog-actions">
        <button type="button" class="button secondary" onclick={() => (modal = 'profiles')}>Back</button>
        <button type="submit" class="button primary" disabled={!profileNameInput.trim()}>
          {editingProfile ? 'Save changes' : 'Create profile'}
        </button>
      </div>
    </form>
  {:else if modal === 'profilePin'}
    <form onsubmit={(e) => { e.preventDefault(); void submitProfilePin(); }}>
      <label for="profile-pin-entry">Enter Profile PIN</label>
      <input
        id="profile-pin-entry"
        type="password"
        maxlength="8"
        bind:value={pinInput}
        placeholder="Enter 4-digit PIN"
        required
      />
      <div class="dialog-actions">
        <button type="button" class="button secondary" onclick={() => (modal = 'profiles')}>Cancel</button>
        <button type="submit" class="button primary" disabled={!pinInput.trim()}>
          <Lock size={16} />Unlock profile
        </button>
      </div>
    </form>
  {:else if modal === 'deleteProfile'}
    <div class="delete-confirmation-content">
      <p style="color: var(--text-primary); margin-bottom: 16px;">
        Are you sure you want to delete profile <strong>"{snapshot.profiles?.find((p) => p.id === deleteTargetProfileId)?.name ?? 'this profile'}"</strong>?
      </p>
      <div class="info-note" style="margin-bottom: 20px;">
        <AlertCircle size={18} />
        <p>This will permanently delete this profile's watch history, bookmarks, and preferences. Installed extensions and device packages will remain intact.</p>
      </div>
      <div class="dialog-actions">
        <button type="button" class="button secondary" onclick={() => (modal = 'profiles')}>Cancel</button>
        <button type="button" class="button danger" onclick={executeDeleteProfile}>
          <Trash2 size={16} />Delete profile
        </button>
      </div>
    </div>
  {:else}<form onsubmit={directPlay}>
      <label for="stream-url">Media URL</label><input
        id="stream-url"
        type="url"
        bind:value={directUrl}
        placeholder="https://example.com/video.mp4"
        required
      /><span class="field-help">Playback opens in a dedicated mpv window.</span>
      <div class="dialog-actions">
        <button type="button" class="button secondary" onclick={() => (modal = null)}>Cancel</button
        ><button class="button primary" disabled={!!busy || !directUrl.trim()}
          ><Play size={16} />Play stream</button
        >
      </div>
    </form>{/if}
</dialog>
