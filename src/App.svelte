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
  let modal = $state<'repository' | 'direct' | null>(null);
  let repoUrl = $state('');
  let directUrl = $state('');
  let repoFilter = $state('');
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
  const entries = $derived(
    snapshot.repositories
      .flatMap((repo) => repo.plugins.map((entry) => ({ repo, entry })))
      .filter(({ entry }) =>
        `${entry.name} ${entry.language ?? ''} ${entry.description ?? ''}`
          .toLowerCase()
          .includes(repoFilter.toLowerCase()),
      ),
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
          <button class="button primary" onclick={() => (modal = 'repository')}
            ><Plus size={17} />Add repository</button
          >
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
              <p>Add an existing Cloudstream repository URL to browse its extensions.</p>
              <button class="button primary" onclick={() => (modal = 'repository')}
                ><Plus size={17} />Add your first repository</button
              ><button class="text-button" disabled={!!busy} onclick={addSample}
                >Or try the bundled open movie sample <ArrowRight size={15} /></button
              >
            </div>
          {:else}<div class="repository-list">
              {#each snapshot.repositories as repo}<div class="repository-row">
                  <span class="source-icon"><Globe size={22} /></span>
                  <div>
                    <strong>{repo.manifest.name}</strong><span
                      >{repo.plugins.length} extensions · {new URL(repo.url).hostname}</span
                    >
                  </div>
                  <button
                    class="icon-button"
                    disabled={!!busy}
                    aria-label={`Refresh ${repo.manifest.name}`}
                    onclick={() =>
                      task('refresh', async () => {
                        await call('add_repository', { url: repo.url });
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
            <label class="filter-input"
              ><Search size={17} /><input
                aria-label="Filter extensions"
                placeholder="Filter extensions by name or language"
                bind:value={repoFilter}
              /></label
            >
            <div class="extension-grid">
              {#each entries as { repo, entry }}{@const installed = snapshot.installed.find(
                  (p) =>
                    p.repositoryUrl === repo.url && p.entry.internalName === entry.internalName,
                )}
                <article class="extension-card">
                  <div class="extension-card-top">
                    <span class="extension-icon"><Puzzle size={25} /></span><span
                      class:android={!entry.jarUrl}
                      class="pill">{entry.jarUrl ? 'JVM' : 'Android'}</span
                    >
                  </div>
                  <h3>{entry.name}</h3>
                  <p>{entry.description ?? `A source from ${repo.manifest.name}.`}</p>
                  <div class="extension-meta">
                    <span>{entry.language?.toUpperCase() ?? 'MULTI'}</span><span
                      >v{entry.version}</span
                    >{#if entry.status === 0}<span>Disabled upstream</span>{/if}
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
        {:else}{#if installedCount}<div class="installed-list">
              {#each snapshot.installed as plugin}<article class="installed-row">
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
      >{#if modal === 'repository'}<Globe size={25} />{:else}<Link size={25} />{/if}</span
    ><button class="icon-button" aria-label="Close dialog" onclick={() => (modal = null)}
      ><X size={20} /></button
    >
  </div>
  <h2>{modal === 'repository' ? 'Bring your sources along.' : 'Something ready to play?'}</h2>
  <p>
    {modal === 'repository'
      ? 'Paste an existing Cloudstream repository URL. You can choose which extensions to install next.'
      : 'Open a direct HTTP or HTTPS media URL in your native player.'}
  </p>
  {#if error}<div class="dialog-error" role="alert">{error}</div>{/if}
  {#if modal === 'repository'}<form onsubmit={addRepository}>
      <label for="repo-url">Repository URL</label><input
        id="repo-url"
        bind:value={repoUrl}
        placeholder="https://example.com/repo.json"
        required
      /><span class="field-help">Cloudstream repository links are supported too.</span>
      <div class="dialog-actions">
        <button type="button" class="button secondary" onclick={() => (modal = null)}>Cancel</button
        ><button class="button primary" disabled={!!busy || !repoUrl.trim()}
          >{#if busy === 'repository'}<LoaderCircle class="spin" size={16} />{:else}<Plus
              size={16}
            />{/if}Add repository</button
        >
      </div>
    </form>
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
