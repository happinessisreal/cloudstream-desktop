<script lang="ts">
  import { onMount, onDestroy, tick } from 'svelte';
  import {
    Compass,
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
  } from 'lucide-svelte';
  import { call, nativeAvailable, emptySnapshot, getSettings, updateSettings, toErrorMessage } from './lib/api';
  import MediaCard from './lib/MediaCard.svelte';
  import MediaRail from './lib/MediaRail.svelte';
  import Spotlight from './lib/Spotlight.svelte';
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
  } from './lib/types';

  type Page = 'discover' | 'library' | 'extensions' | 'settings';
  let page = $state<Page>('discover');
  let searchView = $state(false);
  let contentFilter = $state('All');
  let featuredIndex = $state(0);
  let expandedCategory = $state<number | null>(null);
  let categoryPages = $state<Record<number, number>>({});
  let posterDensity = $state<PosterDensity>(
    (localStorage.getItem('cloudstream.poster-density') as PosterDensity) === 'compact'
      ? 'compact'
      : 'comfortable',
  );
  let snapshot = $state<Snapshot>(structuredClone(emptySnapshot));
  let providers = $state<Provider[]>([]);
  let selectedProvider = $state('');
  let home = $state<Home>({ rows: [] });
  let query = $state('');
  let searchItems = $state<MediaItem[]>([]);
  let searchActive = $state(false);
  let submittedQuery = $state('');
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
    'repository' | 'direct' | 'rename' | 'localInstall' | 'clone' | 'test' | null
  >(null);
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
  let selectedEpisode = $state<Episode | null>(null);
  let links = $state<Links | null>(null);
  let nowPlaying = $state('');
  let paused = $state(false);
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

        if (languageFilter !== 'all') {
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
  const detailSaved = $derived(
    Boolean(
      details &&
      snapshot.library.some(
        (saved) =>
          saved.item.providerId === details?.item.providerId &&
          saved.item.url === details?.item.url,
      ),
    ),
  );

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
  async function refresh() {
    if (!nativeAvailable()) {
      ready = true;
      return;
    }
    snapshot = await call<Snapshot>('snapshot');
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
    if (!providers.some((p) => p.id === selectedProvider))
      selectedProvider = providers[0]?.id ?? '';
    ready = true;
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
  async function chooseProvider(id: string) {
    selectedProvider = id;
    searchActive = false;
    query = '';
    home = { rows: [] };
    featuredIndex = 0;
    contentFilter = 'All';
    expandedCategory = null;
    categoryPages = {};
    ++searchVersion;
    const version = ++providerVersion;
    loadingContent = false;
    if (!id || !providers.find((p) => p.id === id)?.hasMainPage) return;
    loadingContent = true;
    try {
      const result = await call<Home>('provider_home', { providerId: id, page: 1 });
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
      const nextPage = (categoryPages[index] ?? 1) + 1;
      const result = await call<Home>('provider_home', {
        providerId: selectedProvider,
        page: nextPage,
        section: index,
      });
      const next = result.rows[0];
      const current = home.rows[index];
      const existing = new Set(current.items.map((item) => item.url));
      const added = next?.items.filter((item) => !existing.has(item.url)) ?? [];
      home.rows[index] = {
        ...current,
        items: [...current.items, ...added],
        hasNext: added.length > 0 && Boolean(next?.hasNext),
      };
      categoryPages[index] = nextPage;
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
  async function runSearch(event?: SubmitEvent, nextPage = 1) {
    event?.preventDefault();
    if (page === 'extensions') {
      repoFilter = query.trim();
      extensionTab = 'browse';
      return;
    }
    if (!selectedProvider || !query.trim()) {
      searchActive = false;
      if (!selectedProvider)
        error =
          'No active media sources. Open Extensions → Installed to see which sources can run.';
      return;
    }
    searchAbortController?.abort();
    searchAbortController = new AbortController();
    const currentSignal = searchAbortController.signal;

    const version = ++searchVersion;
    ++providerVersion;
    submittedQuery = query.trim();
    searchActive = true;
    loadingContent = true;
    error = '';
    page = 'discover';
    searchView = true;
    details = null;
    try {
      const result = await call<SearchResult>(
        'search',
        {
          providerId: selectedProvider,
          query: query.trim(),
          page: nextPage,
        },
        { signal: currentSignal },
      );
      if (version === searchVersion && !currentSignal.aborted) {
        searchItems = nextPage === 1 ? result.items : [...searchItems, ...result.items];
        searchPage = nextPage;
        searchHasNext = result.hasNext;
      }
    } catch (e) {
      if (currentSignal.aborted) return;
      if (version === searchVersion) showError(e);
    } finally {
      if (version === searchVersion) loadingContent = false;
    }
  }
  async function openItem(item: MediaItem) {
    detailsAbortController?.abort();
    detailsAbortController = new AbortController();
    const currentSignal = detailsAbortController.signal;

    await task('details', async () => {
      details = null;
      links = null;
      selectedEpisode = null;
      details = await call<Details>(
        'load_details',
        { providerId: item.providerId, url: item.url },
        { signal: currentSignal },
      );
    });
  }
  async function resolve(episode: Episode) {
    if (!details) return;
    const item = details.item;
    resolveAbortController?.abort();
    resolveAbortController = new AbortController();
    const currentSignal = resolveAbortController.signal;

    await task('resolve', async () => {
      selectedEpisode = episode;
      links = await call<Links>(
        'resolve_links',
        {
          providerId: item.providerId,
          data: episode.data,
        },
        { signal: currentSignal },
      );
      if (!links.streams.length) message('This extension returned no playable streams.');
    });
  }
  async function playStream(stream: Stream, resume = 0) {
    await task('play', async () => {
      await call('play', {
        stream,
        subtitles: links?.subtitles ?? [],
        item: details?.item ?? null,
        episode: selectedEpisode,
        resume,
      });
      nowPlaying = details?.item.name ?? 'Direct stream';
      paused = false;
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
      await call('play', { stream, subtitles: [], item: null, episode: null, resume: 0 });
      nowPlaying = 'Direct stream';
      paused = false;
      modal = null;
      message('Playback opened in mpv.');
    });
  }
  async function toggleBookmark() {
    if (!details) return;
    const item = details.item;
    await task('bookmark', async () => {
      const existing = snapshot.library.find(
        (saved) => saved.item.providerId === item.providerId && saved.item.url === item.url,
      );
      if (existing) await call('remove_bookmark', { id: existing.id });
      else await call('bookmark', { item });
      await refresh();
      message(existing ? 'Removed from your library.' : 'Saved to your library.');
    });
  }
  async function resume(watch: WatchEntry) {
    await task('resume', async () => {
      const loaded = await call<Details>('load_details', {
        providerId: watch.item.providerId,
        url: watch.item.url,
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
      await call('play', {
        stream: resolved.streams[0],
        subtitles: resolved.subtitles,
        item: loaded.item,
        episode,
        resume: watch.position,
      });
      details = loaded;
      selectedEpisode = episode;
      links = resolved;
      nowPlaying = loaded.item.name;
      paused = false;
      await refresh();
    });
  }
  async function playerAction(action: string) {
    try {
      await call('player_action', { action });
      if (action === 'stop') {
        nowPlaying = '';
        await refresh();
      }
      if (action === 'pause') paused = !paused;
    } catch (e) {
      showError(e);
    }
  }
  function formatTime(seconds: number) {
    const minutes = Math.floor(seconds / 60);
    return `${minutes}:${String(Math.floor(seconds % 60)).padStart(2, '0')}`;
  }
  function shortcut(event: KeyboardEvent) {
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
    if ((event.ctrlKey || event.metaKey) && event.key === 'k') {
      event.preventDefault();
      if (!selectedProvider) page = 'extensions';
      void tick().then(() => searchInput?.focus());
    }
  }
  onMount(() => {
    void refresh()
      .then(() => {
        if (selectedProvider) return chooseProvider(selectedProvider);
      })
      .catch((e) => {
        ready = true;
        showError(e);
      });
    return () => clearTimeout(toastTimer);
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

<svelte:window onkeydown={shortcut} />

<a class="skip-link" href="#main-content">Skip to content</a>
<div class="app-shell" class:compact={posterDensity === 'compact'}>
  <aside class="sidebar">
    <a
      href="#home"
      class="brand"
      onclick={(event) => {
        event.preventDefault();
        navigate('discover');
        searchActive = false;
      }}
      aria-label="Cloudstream home"
      ><img src="/cloudstream-mark.svg" alt="" />
      <div>CloudStream<span>DESKTOP</span></div></a
    >
    <span class="nav-label">BROWSE</span>
    <nav aria-label="Main navigation">
      <button
        class:active={page === 'discover' && !searchView}
        aria-current={page === 'discover' && !searchView ? 'page' : undefined}
        onclick={() => {
          navigate('discover');
          searchActive = false;
        }}><HomeIcon size={21} /><span>Home</span></button
      >
      <button
        class:active={page === 'discover' && searchView}
        aria-label="Search page"
        aria-current={page === 'discover' && searchView ? 'page' : undefined}
        onclick={openSearch}><Search size={21} /><span>Search</span><kbd>/</kbd></button
      >
      <button
        class:active={page === 'library'}
        aria-current={page === 'library' ? 'page' : undefined}
        onclick={() => navigate('library')}
        ><Library size={21} /><span>My library</span>{#if snapshot.library.length}<span
            class="nav-count">{snapshot.library.length}</span
          >{/if}</button
      >
      <button
        class:active={page === 'extensions'}
        aria-current={page === 'extensions' ? 'page' : undefined}
        onclick={() => navigate('extensions')}><Puzzle size={21} /><span>Extensions</span></button
      >
      <button
        class="mobile-settings"
        class:active={page === 'settings'}
        onclick={() => navigate('settings')}><Settings size={21} /><span>Settings</span></button
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
        ><Settings size={20} /><span>Settings</span></button
      >
      <div class="local-profile">
        <span class="profile">L</span>
        <div><strong>Local profile</strong><span>Saved on this device</span></div>
      </div>
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
              ? 'Search'
              : 'Home'
            : page === 'library'
              ? 'Library'
              : page === 'extensions'
                ? 'Extensions'
                : 'Settings'}
      </div>
      <form class="search-bar" onsubmit={runSearch}>
        <Search size={19} /><input
          bind:this={searchInput}
          bind:value={query}
          aria-label={page === 'extensions' ? 'Search extensions' : 'Search your selected source'}
          placeholder={page === 'extensions'
            ? 'Search extensions…'
            : activeProvider
              ? 'Search movies, series, anime…'
              : 'Connect a source to search'}
          disabled={!selectedProvider && page !== 'extensions'}
          oninput={() => {
            if (page === 'extensions') repoFilter = query;
          }}
        /><kbd>Ctrl K</kbd><button
          class="search-submit"
          type="submit"
          aria-label="Search"
          disabled={(!selectedProvider && page !== 'extensions') || !query.trim()}
          ><ArrowRight size={18} /></button
        >
      </form>
      {#if providers.length}<div class="topbar-source">
          <span class="provider-avatar"><Globe size={16} /></span>
          <div>
            <label for="source-select">YOUR SOURCE</label><select
              id="source-select"
              aria-label="Your source"
              value={selectedProvider}
              onchange={(event) => chooseProvider(event.currentTarget.value)}
              >{#each providers as source}<option value={source.id}>{source.name}</option
                >{/each}</select
            >
          </div>
        </div>{/if}
      <button
        class="icon-button"
        aria-label="Refresh source"
        title="Refresh source"
        disabled={!selectedProvider || loadingContent}
        onclick={() => chooseProvider(selectedProvider)}><RefreshCw size={19} /></button
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
        <button
          class="back-button"
          onclick={() => {
            details = null;
            links = null;
          }}><ChevronLeft size={17} />Back to {page === 'library' ? 'library' : 'browse'}</button
        >
        <section class="details-layout">
          <div class="detail-poster">
            <Film size={70} strokeWidth={0.8} /><span>FROM YOUR SOURCES</span
            >{#if details.item.posterUrl}<img
                alt={details.item.name}
                src={details.item.posterUrl}
                referrerpolicy="no-referrer"
              />{/if}
          </div>
          <div class="detail-copy">
            <span class="eyebrow"
              >{details.item.mediaType ?? 'VIDEO'}{details.item.year
                ? ` · ${details.item.year}`
                : ''}</span
            >
            <h1>{details.item.name}</h1>
            <p class="detail-plot">
              {details.plot ?? 'This source did not provide a description.'}
            </p>
            <div class="tags">
              {#each details.tags as tag}<span>{tag}</span>{/each}
            </div>
            <div class="button-row">
              <button
                class="button primary"
                disabled={!!busy || !details.episodes.length}
                onclick={() => resolve(details!.episodes[0])}
                >{#if busy === 'resolve'}<LoaderCircle class="spin" size={18} />{:else}<Play
                    size={18}
                    fill="currentColor"
                  />{/if}Find a stream</button
              ><button class="button secondary" disabled={!!busy} onclick={toggleBookmark}
                >{#if detailSaved}<Check size={18} />{:else}<Bookmark size={18} />{/if}{detailSaved
                  ? 'Saved to library'
                  : 'Save to library'}</button
              >
            </div>
          </div>
        </section>
        {#if links}<section class="section streams-section">
            <div class="section-title">
              <h2>Choose a stream</h2>
              <span>{links.streams.length} available</span>
            </div>
            {#each links.streams as stream}<button
                class="stream-row"
                disabled={!!busy}
                onclick={() => playStream(stream)}
                ><span class="source-icon"><Play size={18} /></span>
                <div>
                  <strong>{stream.name}</strong><span
                    >{stream.quality > 0 ? `${stream.quality}p · ` : ''}{stream.streamType}</span
                  >
                </div>
                <span class="stream-play">Play in mpv <ArrowUpRight size={16} /></span></button
              >{/each}{#if !links.streams.length}<p class="muted">
                No streams returned. Try another episode or source.
              </p>{/if}
          </section>{/if}
        <section class="section">
          <div class="section-title">
            <h2>{details.episodes.length > 1 ? 'Episodes' : 'Watch'}</h2>
            <span>{details.episodes.length} {details.episodes.length === 1 ? 'item' : 'items'}</span
            >
          </div>
          {#each details.episodes as episode, i}<button
              class="episode-row"
              disabled={!!busy}
              onclick={() => resolve(episode)}
              ><span class="episode-number">{String(i + 1).padStart(2, '0')}</span><span
                >{episode.name}</span
              >{#if episode.season}<small>Season {episode.season}</small>{/if}<Play
                size={17}
              /></button
            >{/each}
        </section>
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
            {#if searchActive}<section class="section search-results">
                <div class="section-title">
                  <h2>Results for “{submittedQuery}”</h2>
                  <span>{searchItems.length} {searchItems.length === 1 ? 'title' : 'titles'}</span>
                </div>
                <div class="media-grid">
                  {#each searchItems as item, i}<MediaCard
                      {item}
                      index={i}
                      onclick={() => openItem(item)}
                    />{/each}
                </div>
                {#if !searchItems.length}<div class="empty-state">
                    <Search size={42} strokeWidth={1.3} />
                    <h3>No matches this time</h3>
                    <p>Try another title or choose a different source above.</p>
                  </div>{/if}{#if searchHasNext}<button
                    class="button secondary load-more"
                    onclick={() => runSearch(undefined, searchPage + 1)}
                    >Load more results <ChevronRight size={17} /></button
                  >{/if}
              </section>
            {:else}<div class="empty-state search-empty">
                <div class="empty-search-icon"><Search size={42} strokeWidth={1.3} /></div>
                <h2>What would you like to watch?</h2>
                <p>Search {activeProvider?.name} for a movie, series or anime.</p>
                <span class="shortcut-hint"><kbd>Ctrl</kbd> + <kbd>K</kbd> to focus search</span>
              </div>{/if}
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
                ondetails={() => openItem(featuredItem)}
                onsave={() => saveItem(featuredItem)}
                onprevious={() =>
                  (featuredIndex =
                    (featuredIndex - 1 + featuredItems.length) % featuredItems.length)}
                onnext={() => (featuredIndex = (featuredIndex + 1) % featuredItems.length)}
              />{/if}
            <div id="home-catalog">
              {#each home.rows as row, i}{#if row.items.some(matchesFilter)}<MediaRail
                    title={row.name}
                    items={row.items.filter(matchesFilter)}
                    onopen={openItem}
                    onexpand={() => (expandedCategory = i)}
                  />{/if}{/each}
            </div>
            {#if !featuredItems.length}<div class="empty-state">
                <Film size={42} strokeWidth={1.3} />
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
      {:else if page === 'library'}
        <div class="page-heading">
          <div>
            <span class="eyebrow">YOUR COLLECTION</span>
            <h1>My library</h1>
            <p>Saved titles and everything you’re watching.</p>
          </div>
          <span class="subtle-count">{snapshot.library.length} saved</span>
        </div>
        {#if snapshot.history.length}<section class="section">
            <div class="section-title">
              <h2>Continue watching</h2>
              <span>Saved on this device</span>
            </div>
            <div class="history-grid">
              {#each snapshot.history as watch}<button
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
                >{/each}
            </div>
          </section>{/if}
        <section class="section">
          <div class="section-title">
            <h2>Saved for later</h2>
            <Bookmark size={18} />
          </div>
          {#if snapshot.library.length}<div class="media-grid">
              {#each snapshot.library as saved, i}<MediaCard
                  item={saved.item}
                  index={i}
                  onclick={() => openItem(saved.item)}
                />{/each}
            </div>{:else}<div class="empty-state framed">
              <Bookmark size={40} strokeWidth={1} />
              <h3>A home for your next watch.</h3>
              <p>Save a title from any source and it will be waiting here.</p>
              <button class="button primary" onclick={() => navigate('discover')}
                >Explore your sources <ArrowRight size={16} /></button
              >
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
            <h1>Settings</h1>
            <p>Your desktop, connected.</p>
          </div>
          <span class="pill">EARLY DEVELOPMENT</span>
        </div>
        <section class="settings-panel">
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
        </section>
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
        </section>
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

{#if nowPlaying}<div class="player-bar">
    <span class="playing-icon"><Film size={20} /></span>
    <div><strong>{nowPlaying}</strong><span>Playing in mpv</span></div>
    <button aria-label="Seek back ten seconds" onclick={() => playerAction('back')}
      ><SkipBack size={19} /></button
    ><button
      aria-label={paused ? 'Resume playback' : 'Pause playback'}
      onclick={() => playerAction('pause')}
      >{#if paused}<Play size={20} />{:else}<Pause size={20} />{/if}</button
    ><button aria-label="Seek forward ten seconds" onclick={() => playerAction('forward')}
      ><SkipForward size={19} /></button
    ><button aria-label="Stop playback" onclick={() => playerAction('stop')}
      ><Square size={17} /></button
    >
  </div>{/if}
{#if notice}<div class="toast" role="status">
    <CheckCircle2 size={18} />{notice}<button
      aria-label="Dismiss notification"
      onclick={() => (notice = '')}><X size={16} /></button
    >
  </div>{/if}

<dialog
  bind:this={dialogElement}
  onclose={() => (modal = null)}
  oncancel={() => (modal = null)}
  class="app-dialog"
>
  <div class="dialog-heading">
    <span class="extension-icon"
      >{#if modal === 'repository' || modal === 'rename'}<Globe size={25} />{:else if modal === 'localInstall' || modal === 'clone'}<Puzzle size={25} />{:else}<Link size={25} />{/if}</span
    ><button class="icon-button" aria-label="Close dialog" onclick={() => (modal = null)}
      ><X size={20} /></button
    >
  </div>
  <h2>
    {modal === 'repository'
      ? 'Bring your sources along.'
      : modal === 'rename'
        ? 'Rename repository'
        : modal === 'localInstall'
          ? 'Import local extension'
          : modal === 'clone'
            ? 'Configure clone site'
            : modal === 'test'
              ? 'Source Self-Tester'
              : 'Something ready to play?'}
  </h2>
  <p>
    {modal === 'repository'
      ? 'Paste an existing Cloudstream repository URL or short-code (!code, cutt.ly/code).'
      : modal === 'rename'
        ? 'Set a custom display name for this repository.'
        : modal === 'localInstall'
          ? 'Enter the path to a local .jar or .cs3 extension on your computer.'
          : modal === 'clone'
            ? 'Configure an independent mirror or domain clone for this source without mutating the original.'
            : modal === 'test'
              ? 'Verifies provider capabilities across Home, Search, Details, and Stream link resolution.'
              : 'Open a direct HTTP or HTTPS media URL in your native player.'}
  </p>
  {#if error}<div class="dialog-error" role="alert">{error}</div>{/if}
  {#if modal === 'repository'}<form onsubmit={addRepository}>
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
