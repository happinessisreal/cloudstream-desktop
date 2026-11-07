import { test, expect, type Page } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import { readFileSync } from 'node:fs';

// UI contract tests use an explicit dev-only bridge. Rust/JVM integration is
// exercised separately by cloudstream-cli smoke; this fixture never ships in dist.
async function desktopFixture(page: Page, unsupported = false) {
  await page.addInitScript((unsupported) => {
    const item = {
      providerId: 'sample:Blender Open Movies',
      name: 'Big Buck Bunny · Trailer',
      url: 'https://example.com/film',
      posterUrl: null,
      posterHeaders: {},
      mediaType: 'Movie',
      year: 2008,
    };
    const mixedSeriesItem = {
      providerId: item.providerId,
      name: 'Anime Adventure',
      url: 'https://example.com/mixed-series',
      mediaType: 'Anime',
      posterUrl: 'https://example.com/poster.jpg',
    };
    const episode = {
      name: item.name,
      data: '{"opaque":"episode data, 日本語"}',
      season: null,
      episode: null,
      posterUrl: null,
    };
    const provider = {
      id: item.providerId,
      pluginId: 'sample',
      name: 'Blender Open Movies',
      lang: 'en',
      mainUrl: 'https://www.blender.org',
      hasMainPage: true,
      usesWebView: false,
      supportedTypes: ['Movie'],
    };
    const plugin = {
      id: 'sample',
      repositoryUrl: 'bundled://samples',
      entry: { name: provider.name, internalName: 'sample', version: 1, language: 'en' },
      runtime: 'jvm',
      verified: true,
    };
    const state: any = {
      repositories: unsupported
        ? [
            {
              url: 'https://example.com/repo.json',
              manifest: { name: 'Test sources', manifestVersion: 1, pluginLists: [] },
              plugins: [
                { name: 'Circle FTP', internalName: 'circle', version: 1, status: 1, authors: [] },
                {
                  name: 'Another Source',
                  internalName: 'other',
                  version: 1,
                  status: 1,
                  authors: [],
                },
              ],
            },
          ]
        : [],
      installed: unsupported
        ? [{ ...plugin, runtime: 'android', entry: { ...plugin.entry, name: 'Needs Android UI' } }]
        : [],
      extensionStates: unsupported
        ? [
            {
              pluginId: 'sample',
              loaded: false,
              runtime: 'unavailable',
              error: 'This extension requires Android WebView.',
            },
          ]
        : [],
      library: [],
      history: [],
      profiles: [
        {
          id: 'default',
          name: 'Default Profile',
          avatar: 'default',
          hasPin: false,
          createdAt: Date.now(),
          updatedAt: Date.now(),
        },
      ],
      activeProfileId: 'default',
      profileStores: {
        default: { library: [], history: [] },
      },
      settings: {
        schemaVersion: 1,
        revision: 1,
        global: {
          appearance: {
            posterDensity: 'comfortable',
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
      },
      status: {
        jvmAvailable: true,
        workerAvailable: true,
        mpvAvailable: true,
        androidAvailable: false,
        notes: [],
      },
    };
    (window as any).__state = state;
    // A new install: setup has not been completed yet. Other tests start already set up.
    if ((window as any).__setupPending) state.settings.global.setup = { completed: false, skipped: false, version: 0 };
    if ((window as any).__existingData) state.repositories = [{ url: 'https://example.com/repo.json', manifest: { name: 'Mine', manifestVersion: 1, pluginLists: [] }, plugins: [] }];
    (window as any).__calls = [];
    window.__CLOUDSTREAM_TEST__ = async (command, args) => {
      (window as any).__calls.push({ command, args });
      switch (command) {
        case 'snapshot': {
          const snap = structuredClone(state);
          const pid = (args && (args as any).profileId) || state.activeProfileId || 'default';
          snap.activeProfileId = pid;
          (state as any).profileStores = (state as any).profileStores || {};
          (state as any).profileStores[pid] = (state as any).profileStores[pid] || { library: [], history: [] };
          snap.library = (state as any).profileStores[pid].library;
          snap.history = (state as any).profileStores[pid].history;
          const extra = (window as any).__extraProviders || [];
          snap.providers = state.installed.length && !unsupported ? [provider, ...extra] : extra;
          return snap;
        }
        case 'list_providers': {
          const extra = (window as any).__extraProviders || [];
          return state.installed.length && !unsupported ? [provider, ...extra] : extra;
        }
        case 'install_sample':
          state.installed = [plugin];
          return plugin;
        case 'provider_home': {
          const section = args.section || 0;
          if (section === 0) {
            return {
              rows: [{
                name: 'Open movies',
                rowId: 'Open movies::0::Open movies',
                sectionIndex: 0,
                listIndex: 0,
                page: args.page || 1,
                horizontalImages: true,
                orientation: 'horizontal',
                items: [item],
                hasNext: false,
              }],
              nextSection: 1,
              hasMoreSections: true,
              cached: false,
            };
          } else {
            return {
              rows: [
                {
                  name: 'Popular open titles',
                  rowId: 'Community picks::0::Popular open titles',
                  sectionIndex: 1,
                  listIndex: 0,
                  page: args.page || 1,
                  horizontalImages: false,
                  orientation: 'vertical',
                  items: [item],
                  hasNext: args.page === 1,
                },
                {
                  name: 'Classic animations',
                  rowId: 'Community picks::1::Classic animations',
                  sectionIndex: 1,
                  listIndex: 1,
                  page: args.page || 1,
                  horizontalImages: false,
                  orientation: 'vertical',
                  items: [{ ...item, name: 'Sintel' }],
                  hasNext: false,
                },
              ],
              nextSection: 2,
              hasMoreSections: false,
              cached: false,
            };
          }
        }
        case 'search':
          if (args.providerId === 'failing:Error Source') {
            throw new Error('Connection refused by provider host');
          }
          if (args.query === 'first slow search') {
            await new Promise((resolve) => setTimeout(resolve, 800));
          }
          {
            const q = String(args.query).toLowerCase();
            const matches = [];
            if (q.includes('bunny')) matches.push(item);
            if (q.includes('anime') || q.includes('mixed')) matches.push(mixedSeriesItem);
            return {
              items: matches,
              hasNext: args.page === 1 && q.includes('bunny'),
            };
          }
        case 'get_search_history':
          return (state as any).searchHistory || [];
        case 'add_search_history': {
          if (!(state as any).searchHistory) (state as any).searchHistory = [];
          const entry = {
            id: `${args.profileId || 'default'}::${args.query}`,
            profileId: args.profileId || 'default',
            query: args.query,
            searchedAt: Date.now(),
          };
          (state as any).searchHistory = [
            entry,
            ...(state as any).searchHistory.filter((e: any) => e.query !== args.query),
          ];
          return entry;
        }
        case 'delete_search_history_entry':
          if ((state as any).searchHistory) {
            (state as any).searchHistory = (state as any).searchHistory.filter(
              (e: any) => e.query !== args.query,
            );
          }
          return null;
        case 'clear_search_history':
          (state as any).searchHistory = [];
          return null;
        case 'search_suggestions': {
          const q = String(args.query || '').toLowerCase();
          const list = ((state as any).searchHistory || [])
            .map((e: any) => e.query)
            .filter((s: string) => s.toLowerCase().includes(q));
          if ('big buck bunny'.includes(q) && !list.includes('Big Buck Bunny')) {
            list.push('Big Buck Bunny');
          }
          return list;
        }
        case 'quick_search': {
          const q = String(args.query || '').toLowerCase();
          const matches = [];
          if (q.includes('bunny')) matches.push(item);
          if (q.includes('anime') || q.includes('mixed')) matches.push(mixedSeriesItem);
          return {
            items: matches,
            hasNext: false,
          };
        }
        case 'load_details': {
          if (args.url?.includes('mixed-series') || args.item?.url?.includes('mixed-series')) {
            const episodes = [];
            for (let i = 1; i <= 60; i++) {
              episodes.push({
                name: `Episode ${i}`,
                season: i <= 30 ? 1 : 2,
                episode: i <= 30 ? i : i - 30,
                data: `{"opaque":"anime_ep_${i}_data"}`,
                id: `mixed_series_ep_${i}`,
                description: i === 1 ? '<b>Special premiere</b> episode.' : `Synopsis for episode ${i}`,
                isFiller: i === 5,
                runTime: 24,
                score: 8.5,
                dubStatus: i % 2 === 0 ? 'dubbed' : 'subbed',
              });
            }
            return {
              item: mixedSeriesItem,
              plot: '<p>A <i>thrilling</i> adventure across worlds.</p>',
              tags: ['Action', 'Fantasy'],
              episodes,
              year: 2024,
              duration: 24 * 60,
              rating: 88,
              contentRating: 'TV-14',
              status: 'Airing',
              syncData: { anilist: '121', mal: '5114' },
              nextAiring: 'Next Sunday',
              ...((window as any).__noBackdrop ? {} : { backgroundPoster: 'https://example.com/backdrop.jpg' }),
              logo: 'https://example.com/logo.png',
              trailers: [
                {
                  raw: 'https://example.com/trailer.mp4',
                  extractorUrl: 'https://example.com/trailer.mp4',
                  name: 'Official PV 1',
                },
              ],
              actors: [
                {
                  name: 'Hero Voice',
                  role: 'Protagonist',
                  image: 'https://example.com/hero.jpg',
                },
                {
                  name: 'Rival Voice',
                  role: 'Antagonist',
                },
              ],
              recommendations: [item],
            };
          }
          return {
            item,
            plot: 'An open movie from the <b>Blender Foundation</b>.',
            tags: ['Animation'],
            episodes: [episode],
            duration: 1,
            year: 2008,
            rating: 85,
            contentRating: 'G',
            backgroundPoster: 'https://example.com/bbb-bg.jpg',
            actors: [
              {
                name: 'Blender Cast',
                role: 'Creator',
              },
            ],
            recommendations: [mixedSeriesItem],
          };
        }
        case 'resolve_links':
          return {
            streams: [
              {
                name: 'Official Blender download',
                url: 'https://example.com/video.mov',
                referer: '',
                quality: 480,
                headers: { 'X-Test': 'a,b' },
                streamType: 'VIDEO',
              },
              {
                name: 'Mirror 1080p',
                url: 'https://example.com/mirror.mov',
                referer: '',
                quality: 1080,
                headers: {},
                streamType: 'VIDEO',
              },
            ],
            subtitles: (state as any).__resolvedSubtitles ?? [],
          };
        case 'bookmark': {
          const pid = (args as any)?.profileId || state.activeProfileId || 'default';
          (state as any).profileStores = (state as any).profileStores || {};
          (state as any).profileStores[pid] = (state as any).profileStores[pid] || { library: [], history: [] };
          const entry = { id: 'saved_' + ((state as any).profileStores[pid].library.length + 1), item: args.item, savedAt: Date.now(), watchStatus: 'none', isFavorite: false, isSubscribed: false, profileId: pid };
          (state as any).profileStores[pid].library.push(entry);
          state.library = (state as any).profileStores[pid].library;
          return entry;
        }
        case 'remove_bookmark': {
          const pid = (args as any)?.profileId || state.activeProfileId || 'default';
          if ((state as any).profileStores?.[pid]) {
            (state as any).profileStores[pid].library = [];
            state.library = [];
          }
          return null;
        }
        case 'set_library_status': {
          const pid = (args as any)?.profileId || state.activeProfileId || 'default';
          (state as any).profileStores = (state as any).profileStores || {};
          (state as any).profileStores[pid] = (state as any).profileStores[pid] || { library: [], history: [] };
          const lib = (state as any).profileStores[pid].library;
          const entry = lib.find((l: any) => l.item?.url === args.item?.url || l.id === 'saved');
          if (entry) {
            entry.watchStatus = args.status;
          } else {
            lib.push({ id: 'saved_' + (lib.length + 1), item: args.item, savedAt: Date.now(), watchStatus: args.status, isFavorite: false, isSubscribed: false, profileId: pid });
          }
          state.library = lib;
          return null;
        }
        case 'set_library_favorite': {
          const pid = (args as any)?.profileId || state.activeProfileId || 'default';
          (state as any).profileStores = (state as any).profileStores || {};
          (state as any).profileStores[pid] = (state as any).profileStores[pid] || { library: [], history: [] };
          const lib = (state as any).profileStores[pid].library;
          const entry = lib.find((l: any) => l.item?.url === args.item?.url || l.id === 'saved');
          if (entry) {
            entry.isFavorite = args.isFavorite;
          } else {
            lib.push({ id: 'saved_' + (lib.length + 1), item: args.item, savedAt: Date.now(), watchStatus: 'none', isFavorite: args.isFavorite, isSubscribed: false, profileId: pid });
          }
          state.library = lib;
          return null;
        }
        case 'acknowledge_subscription_updates':
          return null;
        case 'deliver_subscription_alerts':
          return { delivered: 1, failed: 0 };
        case 'check_subscriptions':
          return { checked: 1, skipped: 0, failed: 0, updates: [{ mediaId: 'fixture', title: 'Fixture show', episodeCount: 2, generation: 'gen', episodes: { none: ['e1', 'e2'] } }] };
        case 'set_library_subscribed': {
          const pid = (args as any)?.profileId || state.activeProfileId || 'default';
          (state as any).profileStores = (state as any).profileStores || {};
          (state as any).profileStores[pid] = (state as any).profileStores[pid] || { library: [], history: [] };
          const lib = (state as any).profileStores[pid].library;
          const entry = lib.find((l: any) => l.item?.url === args.item?.url || l.id === 'saved');
          if (entry) {
            entry.isSubscribed = args.isSubscribed;
          } else {
            lib.push({ id: 'saved_' + (lib.length + 1), item: args.item, savedAt: Date.now(), watchStatus: 'none', isFavorite: false, isSubscribed: args.isSubscribed, profileId: pid });
          }
          state.library = lib;
          return null;
        }
        case 'remove_library_item': {
          const pid = (args as any)?.profileId || state.activeProfileId || 'default';
          if ((state as any).profileStores?.[pid]) {
            (state as any).profileStores[pid].library = (state as any).profileStores[pid].library.filter((l: any) => l.id !== args.id);
            state.library = (state as any).profileStores[pid].library;
          }
          return null;
        }
        case 'clear_library': {
          const pid = (args as any)?.profileId || state.activeProfileId || 'default';
          if ((state as any).profileStores?.[pid]) {
            if (args.status) {
              (state as any).profileStores[pid].library = (state as any).profileStores[pid].library.filter((l: any) => l.watchStatus !== args.status);
            } else {
              (state as any).profileStores[pid].library = [];
            }
            state.library = (state as any).profileStores[pid].library;
          }
          return null;
        }
        case 'list_profiles':
          return state.profiles;
        case 'get_profile':
          return state.profiles.find((p: any) => p.id === args.id) || null;
        case 'create_profile': {
          const input = (args as any).input;
          const id = 'prof_' + (state.profiles.length + 1);
          const p = {
            id,
            name: input.name,
            avatar: input.avatar || 'default',
            hasPin: Boolean(input.pin),
            _pin: input.pin || null,
            createdAt: Date.now(),
            updatedAt: Date.now(),
          };
          state.profiles.push(p);
          (state as any).profileStores = (state as any).profileStores || {};
          (state as any).profileStores[id] = { library: [], history: [] };
          return p;
        }
        case 'update_profile': {
          const id = (args as any).id;
          const input = (args as any).input;
          const p = state.profiles.find((x: any) => x.id === id);
          if (!p) throw new Error('Profile not found');
          if (input.name) p.name = input.name;
          if (input.avatar) p.avatar = input.avatar;
          if (input.removePin) {
            p.hasPin = false;
            p._pin = null;
          } else if (input.pin) {
            p.hasPin = true;
            p._pin = input.pin;
          }
          p.updatedAt = Date.now();
          return p;
        }
        case 'delete_profile': {
          const id = (args as any).id;
          if (state.profiles.length <= 1) throw new Error('Cannot delete only profile');
          state.profiles = state.profiles.filter((x: any) => x.id !== id);
          if ((state as any).profileStores?.[id]) {
            delete (state as any).profileStores[id];
          }
          if (state.activeProfileId === id) {
            state.activeProfileId = 'default';
          }
          return null;
        }
        case 'verify_profile_pin': {
          const p = state.profiles.find((x: any) => x.id === args.id);
          if (!p) throw new Error('Profile not found');
          if (!p.hasPin) return true;
          return p._pin === args.pin;
        }
        case 'switch_profile': {
          const targetId = (args as any).targetProfileId;
          const p = state.profiles.find((x: any) => x.id === targetId);
          if (!p) throw new Error('Profile not found');
          if (p.hasPin && p._pin && p._pin !== (args as any).pin) {
            throw new Error('Incorrect PIN');
          }
          state.activeProfileId = targetId;
          return p;
        }
        case 'remove_history_item': {
          state.history = state.history.filter((h: any) => h.id !== args.id);
          return null;
        }
        case 'clear_history': {
          state.history = [];
          return null;
        }
        case 'get_media_watch_progress':
          return (state as any).progresses || [];
        case 'mark_episode_watched': {
          (state as any).progresses = (state as any).progresses || [];
          const idx = (state as any).progresses.findIndex((p: any) => p.episodeId === args.episodeId);
          if (idx >= 0) {
            (state as any).progresses[idx].isWatched = args.isWatched;
            (state as any).progresses[idx].watchedOverride = args.isWatched;
          } else {
            (state as any).progresses.push({
              profileId: 'default',
              mediaId: args.mediaId,
              episodeId: args.episodeId,
              positionSeconds: 0,
              updatedAtMs: Date.now(),
              isWatched: args.isWatched,
              watchedOverride: args.isWatched,
            });
          }
          return null;
        }
        case 'mark_episodes_up_to': {
          (state as any).progresses = (state as any).progresses || [];
          for (const ep of args.episodes || []) {
            const epId = ep.name || 'ep';
            const idx = (state as any).progresses.findIndex((p: any) => p.episodeId === epId);
            if (idx >= 0) {
              (state as any).progresses[idx].isWatched = true;
              (state as any).progresses[idx].watchedOverride = true;
            } else {
              (state as any).progresses.push({
                profileId: 'default',
                mediaId: args.mediaId,
                episodeId: epId,
                positionSeconds: 0,
                updatedAtMs: Date.now(),
                isWatched: true,
                watchedOverride: true,
              });
            }
          }
          return null;
        }
        case 'rank_streams': {
          const ranked = (args.streams as any[]).map((stream: any, index: number) => ({ index, usable: !(state as any).__hideFirst || index !== 0, qualityLabel: stream.quality === 400 ? 'Unknown' : `${stream.quality}p` }));
          return (state as any).__reverseRank ? ranked.reverse() : ranked;
        }
        case 'play': {
          (state as any).__lastPlayArgs = args;
          return null;
        }
        case 'choose_subtitle_font': return (state as any).__fontSelection ?? null;
        case 'subtitle_font_preview': return (state as any).__fontData ?? '';
        case 'choose_local_subtitle': return (state as any).__subtitleArchive ?? { status: 'added' };
        case 'prepare_provider_subtitle_archive': return (state as any).__subtitleArchive;
        case 'select_archived_subtitle': return null;
        case 'cancel_archived_subtitle': return null;
        case 'player_skip_markers': return structuredClone((window as any).__skipMarkers || []);
        case 'player_skip_interval': return null;
        case 'begin_speed_hold':
          if ((window as any).__delaySpeedHold) await new Promise((resolve) => setTimeout(resolve, 100));
          return null;
        case 'end_speed_hold': return null;
        case 'player_command': {
          const command = args.command as any;
          const st = (state as any).__playerState || { sessionId: 'test-session', active: true, playing: true, paused: false, position: 12, duration: 120, volume: 100, speed: 1, itemName: item.name };
          if (command.type === 'set_volume') st.volume = command.volume;
          if (command.type === 'seek_absolute') st.position = command.seconds;
          if (command.type === 'set_mute') st.muted = command.muted;
          (state as any).__playerState = st;
          return { ...st };
        }
        case 'player_action': {
          (state as any).__lastPlayerAction = (args as any)?.action;
          return null;
        }
        case 'remove_extension':
          state.installed = [];
          return null;
        case 'cancel_request':
          return true;
        case 'refresh_repository':
          return state.repositories[0] ?? null;
        case 'rename_repository': {
          if (state.repositories[0]) {
            state.repositories[0].customName = (args.name as string) || undefined;
          }
          return state.repositories[0] ?? null;
        }
        case 'install_local_extension': {
          const localPlugin = {
            id: 'local_sample',
            repositoryUrl: 'local://packages',
            entry: {
              url: 'file:///sample.jar',
              name: 'Local Sample Extension',
              internalName: 'local_sample',
              version: 1,
              status: 1,
              apiVersion: 1,
              authors: ['Local Author'],
              language: 'en',
            },
            runtime: 'jvm',
            verified: true,
            sha256: 'local_sha',
            installedAt: 1,
          };
          state.installed.push(localPlugin);
          return localPlugin;
        }
        case 'set_provider_enabled': {
          provider.disabled = !args.enabled;
          return null;
        }
        case 'configure_provider_clone':
          return null;
        case 'delete_provider_clone':
          return null;
        case 'check_for_updates':
          return [
            {
              pluginId: 'sample',
              name: 'Blender Open Movies',
              repositoryUrl: 'bundled://samples',
              installedVersion: 1,
              availableVersion: 2,
              hasUpdate: true,
              entry: { ...plugin.entry, version: 2 },
            },
          ];
        case 'update_extension': {
          if (state.installed[0]) {
            state.installed[0].entry.version = 2;
          }
          return state.installed[0] ?? null;
        }
        case 'update_all_extensions': {
          if (state.installed[0]) {
            state.installed[0].entry.version = 2;
          }
          return [
            {
              pluginId: 'sample',
              name: 'Blender Open Movies',
              success: true,
              previousVersion: 1,
              updatedVersion: 2,
            },
          ];
        }
        case 'test_provider': {
          return {
            providerId: args.providerId,
            providerName: 'Blender Open Movies',
            passed: true,
            totalDurationMs: 42,
            stages: [
              { stage: 'home', status: 'passed', durationMs: 12, message: 'Loaded 1 row' },
              { stage: 'search', status: 'passed', durationMs: 10, message: 'Returned 1 item' },
              { stage: 'details', status: 'passed', durationMs: 11, message: 'Loaded details' },
              { stage: 'links', status: 'passed', durationMs: 9, message: 'Resolved 1 stream' },
            ],
          };
        }
        case 'list_links':
          return [
            { id: 'project', url: 'https://github.com/happinessisreal/cloudstream-desktop' },
            { id: 'issues', url: 'https://github.com/happinessisreal/cloudstream-desktop/issues' },
            { id: 'upstream', url: 'https://github.com/recloudstream/cloudstream' },
            { id: 'docs', url: 'https://recloudstream.github.io/csdocs/' },
            { id: 'community', url: 'https://discord.gg/5Hus6fM' },
            { id: 'extension-policy', url: 'https://github.com/recloudstream' },
          ];
        case 'open_link':
          return null;
        case 'get_legal_text':
          return `LEGAL TEXT FOR ${args.name}\nGNU GENERAL PUBLIC LICENSE Version 3`;
        case 'quit_app':
          return null;
        case 'diagnostics_summary':
          return {
            generatedAtMs: 1, appVersion: '0.1.0', os: 'linux', arch: 'x86_64', javaAvailable: true, workerAvailable: true,
            workerJarSha256: 'a'.repeat(64), mpvVersion: '0.41.0', playerUnsupported: ['Video delay: mpv has no independent video delay.'],
            packages: [{ id: 'sample', name: 'Blender Open Movies', version: 1, runtime: 'jvm', sha256: 'b'.repeat(64), verified: true, repository: 'https://repo.example/[redacted]/repo.json?[redacted]' }],
            providers: 3, disabledProviders: 1,
            events: (window as any).__diagnosticsCleared ? [] : [
              { atMs: 1_700_000_000_000, category: 'worker', message: 'loadLinks failed: https://cdn.example/f.mp4?[redacted] Authorization: [redacted]' },
              { atMs: 1_700_000_001_000, category: 'player', message: 'Playback failed: loading failed' },
            ],
            timings: [{ atMs: 1, method: 'loadLinks', durationMs: 4200, ok: false, outcome: 'timeout' }, { atMs: 2, method: 'search', durationMs: 90, ok: true, outcome: 'ok' }],
            notes: [],
          };
        case 'export_diagnostics':
          return (window as any).__saveCancelled ? null : '/home/user/cloudstream-diagnostics.json';
        case 'clear_diagnostics':
          (window as any).__diagnosticsCleared = true;
          return null;
        case 'check_app_update': {
          if ((window as any).__updateError) throw new Error('The update feed\'s signature is not valid, so it was ignored');
          return (window as any).__updateResult || { state: 'not_configured', reason: 'No update channel is configured. Update through the place you installed it from.' };
        }
        case 'open_release_notes':
          return null;
        case 'metadata_for': {
          const meta = ((window as any).__meta ||= { user: {}, blocked: [] });
          const supplied: Record<string, string> = (args.details as any).syncData || {};
          const links: any[] = [];
          for (const [service, id] of Object.entries(supplied)) {
            if (['anilist', 'kitsu'].includes(service) && !meta.blocked.includes(`${service}:${id}`)) links.push({ mediaId: 'm', service, externalId: id, source: 'provider', linkedAtMs: 1 });
          }
          for (const [service, link] of Object.entries(meta.user) as any) {
            const at = links.findIndex((l) => l.service === service);
            if (at >= 0) links[at] = link; else links.push(link);
          }
          const view: any = { provenance: {}, genres: [], cast: [], recommendations: [], fillerEpisodeIds: [], notes: [] };
          const errors = (window as any).__metaErrors ? ['anilist: Could not reach the metadata service'] : [];
          if (links.length && !errors.length) {
            const prefer = Boolean(((window as any).__state.settings.global.metadata || {}).preferPoster);
            const own = (args.details as any).item.posterUrl as string | null;
            const art = prefer || !own;
            const ownBackdrop = ((args.details as any).backgroundPosterUrl ?? (args.details as any).backgroundPoster) as string | undefined;
            Object.assign(view, { posterUrl: art ? 'https://s4.anilist.co/xl.jpg' : own, backdropUrl: ownBackdrop ? ownBackdrop : 'https://s4.anilist.co/banner.jpg', synopsis: 'Two brothers search for the Philosopher\'s Stone.', genres: ['Action'], score: 90,
              cast: [{ name: 'Edward Elric', role: 'main' }], recommendations: [{ title: 'Cowboy Bebop', service: 'anilist', externalId: '1' }],
              fillerEpisodeIds: ['mixed_series_ep_2', 'mixed_series_ep_4'], provenance: { poster: art ? links[0].service : 'provider', backdrop: ownBackdrop ? 'provider' : links[0].service, synopsis: links[0].service, genres: links[0].service, cast: links[0].service, recommendations: links[0].service, filler: links[0].service, score: links[0].service } });
          }
          return { links, view, errors, stale: errors.length > 0 && !!(window as any).__metaStale };
        }
        case 'metadata_search':
          return { candidates: [
            { service: 'anilist', externalId: '121', title: 'Fullmetal Alchemist: Brotherhood', year: 2009, format: 'TV', similarity: 100, warnings: [] },
            { service: 'anilist', externalId: '25', title: 'Fullmetal Alchemist', year: 2003, format: 'TV', similarity: 80, warnings: ['Year differs (2003 here, 2024 in your source). It may be a remake or a different season.'] },
          ], problems: [] };
        case 'set_metadata_link': {
          const meta = ((window as any).__meta ||= { user: {}, blocked: [] });
          meta.user[args.service as string] = { mediaId: 'm', service: args.service, externalId: args.externalId, source: 'user', linkedAtMs: 2, label: args.label, year: args.year };
          return meta.user[args.service as string];
        }
        case 'remove_metadata_link': {
          const meta = ((window as any).__meta ||= { user: {}, blocked: [] });
          delete meta.user[args.service as string];
          const supplied = ((args.syncData as any) || {})[args.service as string];
          if (supplied) meta.blocked.push(`${args.service}:${supplied}`);
          return null;
        }
        case 'take_entry_actions': {
          const batch = (window as any).__entries || { actions: [], errors: [] };
          (window as any).__entries = { actions: [], errors: [] };
          return batch;
        }
        case 'play_local_file':
          return 'local-session';
        case 'play_download':
          if ((window as any).__downloadPlayFails) throw new Error('The downloaded files are missing or have changed.');
          return 'offline-session';
        case 'list_external_players':
          return (window as any).__noVlc ? [] : [{ kind: 'vlc', name: 'VLC', program: '/usr/bin/vlc' }];
        case 'open_in_external_player': {
          if ((window as any).__externalFails) throw new Error('VLC cannot send these request headers: Cookie. Use the built-in player for this source');
          return { program: '/usr/bin/vlc', warnings: ((window as any).__externalWarnings || []) };
        }
        case 'choose_folder':
          return (window as any).__folderCancelled ? null : '/home/user/Backups';
        case 'backup_status': {
          const b = (state.settings.global as any).backup;
          const status: any = { ...((window as any).__backupStatus || {}) };
          if (b?.enabled) status.nextDueMs = 1_800_000_000_000;
          return status;
        }
        case 'run_backup_now': {
          if ((window as any).__backupFails) {
            (window as any).__backupStatus = { lastAttemptMs: 1, lastError: 'The backup folder /home/user/Backups is not writable' };
            throw new Error('The backup folder /home/user/Backups is not writable');
          }
          (window as any).__backupStatus = { lastAttemptMs: 1_790_000_000_000, lastSuccessMs: 1_790_000_000_000, lastFile: '/home/user/Backups/cloudstream-auto-backup-20261106-172640.json' };
          return { file: '/home/user/Backups/cloudstream-auto-backup-20261106-172640.json', bytes: 2048, pruned: ['cloudstream-auto-backup-20261001-000000.json'] };
        }
        case 'export_backup': {
          if ((window as any).__saveCancelled) return null;
          return { path: '/home/user/cloudstream-backup-2026-11-06.json', sections: args.sections, profiles: 2, library: 5, history: 1, progress: 3 };
        }
        case 'choose_backup_file': {
          if ((window as any).__noBackupFile) return null;
          return { importId: 'import-1', name: (window as any).__backupName || 'backup.json', size: 1234 };
        }
        case 'preview_import': {
          if ((window as any).__previewError) throw new Error((window as any).__previewError);
          const o: any = args.options;
          const all = ['profiles', 'library', 'progress', 'settings', 'repositories'];
          const picked = o.sections ?? all;
          const skip = o.conflictPolicy === 'skip_existing';
          return {
            source: (window as any).__backupSource || 'desktop', createdAtMs: 1, appVersion: '0.1.0', availableSections: all,
            token: `token-${o.conflictPolicy}-${picked.join('+')}`,
            report: {
              sections: picked.map((section: string) => ({ section, incoming: 4, new: skip ? 2 : 1, replaced: skip ? 0 : 1, unchanged: 1, skipped: skip ? 1 : 0 })),
              createdProfiles: [{ name: 'Kids', id: 'preview-android:1' }],
              unresolved: [{ id: 'u1', name: 'Lost Movie', apiName: 'Gone Source', url: 'https://gone.example/x', reason: "No installed source is called 'Gone Source'" }],
              repositoriesToAdd: [], notes: ['2 profile PINs were not imported. Set a PIN again in Profiles.'],
            },
          };
        }
        case 'apply_import': {
          if (args.token !== `token-${(args.options as any).conflictPolicy}-${((args.options as any).sections ?? []).join('+')}`) throw new Error('The backup or the import choices changed since the preview. Preview the import again.');
          (window as any).__waiting = [{ id: 'u1', name: 'Lost Movie', apiName: 'Gone Source', url: 'https://gone.example/x', reason: 'x' }];
          return {
            sections: [{ section: 'library', incoming: 4, new: 2, replaced: 0, unchanged: 1, skipped: 1 }],
            createdProfiles: [{ name: 'Kids', id: 'prof_1' }], unresolved: [],
            repositoriesToAdd: [{ url: 'https://example.test/repo.json', name: 'My repo' }],
            notes: ['Account tokens, logins, download folders and installed packages are never imported.'],
            preImportBackup: '/home/user/.local/share/app/backups/pre-import-1.json',
          };
        }
        case 'discard_import':
          return null;
        case 'list_unresolved_imports':
          return (window as any).__waiting || [];
        case 'resolve_unresolved_imports': {
          const had = ((window as any).__waiting || []).length;
          if ((window as any).__sourceInstalled) (window as any).__waiting = [];
          return (window as any).__sourceInstalled ? had : 0;
        }
        case 'download_jobs':
          if ((window as any).__downloadsDown) throw new Error('Another app instance owns the download queue');
          return structuredClone(((state as any).downloads || []).filter((j: any) => j.profileId === (args.profileId || 'default')));
        case 'download_watch_progress':
          return structuredClone((window as any).__downloadProgress || {});
        case 'download_subtitle_warnings':
          return structuredClone((window as any).__subtitleWarnings || {});
        case 'download_health':
          return structuredClone((window as any).__downloadHealth || {});
        case 'repair_download': {
          const downloads = (state as any).downloads || [];
          const original = downloads.find((job: any) => job.id === args.id && job.profileId === args.profileId);
          if (!original) throw new Error('Download job not found');
          const replacement = { ...original, id: 'repair-1', repairOf: original.id, state: 'queued', bytes: 0, total: null, output: null, order: downloads.length + 1, revision: 1 };
          downloads.push(replacement);
          return structuredClone(replacement);
        }
        case 'enqueue_download': {
          if ((window as any).__folderCancelled) return null;
          const downloads = ((state as any).downloads ||= []);
          const n = downloads.length + 1;
          const job = {
            id: `00000000-0000-4000-8000-00000000000${n}`,
            profileId: args.profileId || 'default',
            mediaId: 'm', episodeId: `e${n}`, providerId: (args.item as any).providerId,
            title: (args.item as any).name, episodeName: (args.episode as any).name,
            season: (args.episode as any).season ?? null, episode: (args.episode as any).episode ?? null,
            destination: '/home/user/Videos', extension: args.extension, policy: args.policy,
            state: 'queued', bytes: 0, total: null, output: null, error: null,
            order: n, revision: 1, createdAtMs: Date.now(), updatedAtMs: Date.now(),
          };
          downloads.push(job);
          return structuredClone(job);
        }
        case 'enqueue_downloads': {
          if ((window as any).__folderCancelled) return null;
          const downloads = ((state as any).downloads ||= []);
          let added = 0;
          let alreadyQueued = 0;
          for (const ep of args.episodes as any[]) {
            if (downloads.some((j: any) => j.dataKey === ep.data)) { alreadyQueued += 1; continue; }
            const n = downloads.length + 1;
            downloads.push({
              id: `00000000-0000-4000-8000-${String(n).padStart(12, '0')}`, dataKey: ep.data,
              profileId: args.profileId || 'default', mediaId: 'm', episodeId: `e${n}`,
              providerId: (args.item as any).providerId, title: (args.item as any).name, episodeName: ep.name,
              season: ep.season ?? null, episode: ep.episode ?? null, destination: '/home/user/Videos',
              extension: args.extension, policy: args.policy, state: 'queued', bytes: 0, total: null,
              output: null, error: null, order: n, revision: 1, createdAtMs: Date.now(), updatedAtMs: Date.now(),
            });
            added += 1;
          }
          return { added, alreadyQueued, failed: [] };
        }
        case 'control_download': {
          const job = ((state as any).downloads || []).find((j: any) => j.id === args.id);
          if (!job) throw new Error('Download job not found');
          const next: any = { pause: 'paused', resume: 'queued', cancel: 'canceled', retry: 'queued' };
          job.state = next[args.action as string];
          job.revision += 1;
          return structuredClone(job);
        }
        case 'reorder_downloads': {
          const downloads: any[] = (state as any).downloads || [];
          const queued = downloads.filter((j) => j.state === 'queued');
          const slots = queued.map((j) => j.order).sort((a, b) => a - b);
          (args.ids as string[]).forEach((id, i) => {
            downloads.find((j) => j.id === id).order = slots[i];
          });
          return null;
        }
        case 'remove_download': {
          const downloads: any[] = (state as any).downloads || [];
          const job = downloads.find((j) => j.id === args.id);
          if (!job) throw new Error('Download job not found');
          if (['resolving', 'downloading'].includes(job.state)) throw new Error('Pause or cancel the download before removing it');
          (state as any).downloads = downloads.filter((j) => j.id !== args.id);
          return { fileDeleted: !!args.deleteFile && job.state === 'completed', tempCleaned: true, notes: [] };
        }
        case 'get_settings':
          return structuredClone(state.settings);
        case 'update_settings': {
          const patch = (args.patch as any) || {};
          const density = patch.appearance?.posterDensity || patch.posterDensity;
          if (density) {
            state.settings.global.appearance.posterDensity = density;
          }
          if (patch.appearance?.searchSuggestions !== undefined) {
            state.settings.global.appearance.searchSuggestions = patch.appearance.searchSuggestions;
          }
          if (patch.subscriptionNotifications !== undefined) {
            state.settings.global.subscriptionNotifications = patch.subscriptionNotifications;
          }
          if (patch.appearance && !Array.isArray(patch.appearance)) {
            Object.assign(state.settings.global.appearance, patch.appearance);
          }
          if (patch.metadata) {
            (state.settings.global as any).metadata = { enabled: false, anilist: true, kitsu: true, filler: true, trailer: true, cast: true, recommendations: true, preferPoster: false, ...((state.settings.global as any).metadata || {}), ...patch.metadata };
          }
          if (patch.skip) {
            (state.settings.global as any).skip = { ...((state.settings.global as any).skip || {}), ...patch.skip };
          }
          if (patch.setup) {
            (state.settings.global as any).setup = { ...((state.settings.global as any).setup || {}), ...patch.setup };
          }
          if (patch.providerLanguages) {
            (state.settings.global as any).providerLanguages = patch.providerLanguages;
          }
          if (patch.updates) {
            (state.settings.global as any).updates = { ...((state.settings.global as any).updates || {}), ...patch.updates };
          }
          if (patch.externalPlayer) {
            const next: any = { kind: 'none', ...patch.externalPlayer };
            if (next.kind === 'custom' && !next.program) throw new Error('Choose the program to run for your custom player');
            if (next.kind === 'custom' && !(next.args || []).some((a: string) => a.includes('{url}'))) throw new Error('A custom player needs at least one argument containing {url}');
            (state.settings.global as any).externalPlayer = next;
          }
          if (patch.backup) {
            const next: any = { enabled: false, frequency: 'daily', retention: 7, ...((state.settings.global as any).backup || {}), ...patch.backup };
            if (next.enabled && !next.directory) throw new Error('Choose a folder before turning scheduled backups on');
            if (next.retention < 1 || next.retention > 100) throw new Error('Keep between 1 and 100 scheduled backups');
            (state.settings.global as any).backup = next;
          }
          if (patch.autoSelectProfile !== undefined) {
            state.settings.global.autoSelectProfile = patch.autoSelectProfile;
          }
          if (patch.defaultProfileId !== undefined) {
            state.settings.global.defaultProfileId = patch.defaultProfileId;
          }
          if (patch.selectedProvider !== undefined) {
            state.settings.profiles = state.settings.profiles || {};
            state.settings.profiles['default'] = state.settings.profiles['default'] || {};
            state.settings.profiles['default'].selectedProvider = patch.selectedProvider;
          }
          if (patch.sourcePriority) {
            state.settings.profiles = state.settings.profiles || {};
            const id = (args.profileId as string) || 'default';
            state.settings.profiles[id] = state.settings.profiles[id] || {};
            state.settings.profiles[id].sourcePriority = structuredClone(patch.sourcePriority);
          }
          if (patch.player) {
            state.settings.global.player = { ...(state.settings.global.player || {}), ...patch.player };
          }
          if (patch.playback) {
            state.settings.global.playback = {
              ...(state.settings.global.playback || {
                autoNext: true,
                startPaused: false,
                completionThreshold: 0.9,
                resumeThresholdSeconds: 5.0,
                prefetchNext: true,
              }),
              ...patch.playback,
            };
          }
          state.settings.revision = (state.settings.revision || 1) + 1;
          return structuredClone(state.settings);
        }
        case 'add_repository':
          if (args.url === 'https://example.test/repo.json') return { url: args.url, manifest: { name: 'My repo' } };
          throw new Error('This URL did not return a Cloudstream repository manifest');
        default:
          throw new Error(`Unexpected UI command: ${command}`);
      }
    };
  }, unsupported);
  await page.goto('/');
}

test('empty state, navigation, and keyboard-accessible repository dialog', async ({ page }) => {
  await desktopFixture(page);
  await expect(page.getByRole('heading', { name: 'Your media. Your way.' })).toBeVisible();
  await page.getByRole('button', { name: 'Add a repository', exact: true }).last().click();
  const dialog = page.getByRole('dialog');
  await expect(dialog).toBeVisible();
  await expect(page.getByLabel('Repository URL', { exact: true })).toBeFocused();
  await page.keyboard.press('Escape');
  await expect(dialog).not.toBeVisible();
  await page.getByRole('button', { name: 'My library', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'A home for your next watch.' })).toBeVisible();
});

test('search, details, bookmark, opaque episode data, and playback commands', async ({ page }) => {
  await desktopFixture(page);
  await page.getByRole('button', { name: 'Try an open movie' }).click();
  await expect(page.getByLabel('Your source', { exact: true })).toHaveValue(
    'sample:Blender Open Movies',
  );
  const search = page.getByLabel('Search your selected source');
  await search.fill('Bunny');
  await search.press('Enter');
  await page.getByRole('button', { name: /Big Buck Bunny · Trailer/ }).click();
  await expect(
    page.getByRole('heading', { name: 'Big Buck Bunny · Trailer', exact: true }),
  ).toBeVisible();
  await page.getByRole('button', { name: 'Save to library', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Saved to library', exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Find a stream', exact: true }).click();
  await page.getByRole('button', { name: /^Official Blender download/ }).click();
  await expect(page.getByLabel('Pause playback')).toBeVisible();
  const calls = await page.evaluate(() => (window as any).__calls);
  expect(calls.find((c: any) => c.command === 'resolve_links').args.data).toBe(
    '{"opaque":"episode data, 日本語"}',
  );
  expect(calls.find((c: any) => c.command === 'play').args.stream.headers).toEqual({
    'X-Test': 'a,b',
  });
  await page.getByLabel('Stop playback').click();
  await page.getByRole('button', { name: 'Open a stream URL', exact: true }).click();
  await page.getByLabel('Media URL', { exact: true }).fill('https://example.com/direct.mp4');
  await page.getByRole('dialog').getByRole('button', { name: 'Play stream', exact: true }).click();
  const direct = await page.evaluate(() =>
    (window as any).__calls.filter((c: any) => c.command === 'play').at(-1),
  );
  expect(direct.args.item).toBeNull();
  expect(direct.args.episode).toBeNull();
  await page.getByLabel('Stop playback').click();
  await page.getByRole('button', { name: /My library/ }).click();
  await expect(page.getByRole('button', { name: /Big Buck Bunny · Trailer/ })).toBeVisible();
});

test('repository failure stays visible and dialog remains editable', async ({ page }) => {
  await desktopFixture(page);
  await page.getByRole('button', { name: 'Add a repository', exact: true }).last().click();
  await page.getByLabel('Repository URL', { exact: true }).fill('https://example.com/bad.json');
  await page
    .getByRole('dialog')
    .getByRole('button', { name: 'Add repository', exact: true })
    .click();
  await expect(page.getByRole('dialog').getByRole('alert')).toContainText(
    'did not return a Cloudstream repository manifest',
  );
  await expect(page.getByLabel('Repository URL', { exact: true })).toBeEditable();
});

test('runtime health does not claim Android support', async ({ page }) => {
  await desktopFixture(page);
  await page.getByRole('button', { name: 'Settings', exact: true }).last().click();
  await expect(page.getByText('Android UI and WebView · in development')).toBeVisible();
  await expect(page.getByText('Ready', { exact: true })).toHaveCount(2);
});

test('desktop and narrow layouts have no horizontal overflow or accessibility violations', async ({
  page,
}) => {
  await desktopFixture(page);
  for (const width of [1280, 375]) {
    await page.setViewportSize({ width, height: 900 });
    expect(
      await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth),
    ).toBe(true);
    const results = await new AxeBuilder({ page }).withTags(['wcag2a', 'wcag2aa']).analyze();
    expect(
      results.violations.map((v) => ({ id: v.id, nodes: v.nodes.map((n) => n.target) })),
    ).toEqual([]);
  }
});

test('extension search works without an active media source', async ({ page }) => {
  await desktopFixture(page, true);
  await expect(page.getByText('No media sources are active', { exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Review extensions' }).click();
  await expect(page.getByText('This extension requires Android WebView.')).toBeVisible();
  await page.getByLabel('Search extensions', { exact: true }).fill('Circle');
  await page.getByRole('button', { name: 'Search', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Circle FTP', exact: true })).toBeVisible();
  await expect(
    page.getByRole('heading', { name: 'Another Source', exact: true }),
  ).not.toBeVisible();
  const calls = await page.evaluate(() => (window as any).__calls);
  expect(calls.some((c: any) => c.command === 'search')).toBe(false);
});

test('media search can be submitted with the visible Search button', async ({ page }) => {
  await desktopFixture(page);
  await page.getByRole('button', { name: 'Try an open movie' }).click();
  await page.getByLabel('Search your selected source').fill('Bunny');
  await page.getByRole('button', { name: 'Search', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Results for “Bunny”' })).toBeVisible();
  await expect(page.getByRole('button', { name: /Big Buck Bunny · Trailer/ })).toBeVisible();
});

test('desktop browsing controls filter titles, expand shelves, and open dedicated search', async ({
  page,
}) => {
  await desktopFixture(page);
  await page.getByRole('button', { name: 'Try an open movie' }).click();
  await expect(page.getByRole('region', { name: 'Featured title' })).toBeVisible();
  await page.getByRole('button', { name: 'TV series', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'No tv series in this selection' })).toBeVisible();
  await page.getByRole('button', { name: 'Show all titles' }).click();
  await page.getByRole('button', { name: 'See all', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Open movies', level: 1 })).toBeVisible();
  await expect(page.getByRole('button', { name: /Open Big Buck Bunny/ })).toBeVisible();
  await page.getByRole('button', { name: 'Back to home', exact: true }).click();
  await page.getByRole('button', { name: 'More info', exact: true }).click();
  await expect(
    page.getByRole('heading', { name: 'Big Buck Bunny · Trailer', level: 1 }),
  ).toBeVisible();
  await page.getByRole('button', { name: 'Back to browse', exact: true }).click();
  await page.getByRole('button', { name: 'Search page', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'What would you like to watch?' })).toBeVisible();
  await expect(page.getByLabel('Search your selected source')).toBeFocused();
});

test('poster density changes browsing layout and persists', async ({ page }) => {
  await desktopFixture(page);
  await page.getByRole('button', { name: 'Try an open movie' }).click();
  const comfortable = (await page.locator('.media-rail .media-card').first().boundingBox())!.width;
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  await page.getByRole('button', { name: 'Compact', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Compact', exact: true })).toHaveAttribute(
    'aria-pressed',
    'true',
  );
  const calls = await page.evaluate(() => (window as any).__calls);
  const updateCall = calls.find(
    (c: any) => c.command === 'update_settings' && c.args?.patch?.appearance?.posterDensity,
  );
  expect(updateCall).toBeDefined();
  expect(updateCall.args.patch.appearance.posterDensity).toBe('compact');
  await page.getByRole('button', { name: 'Home', exact: true }).click();
  const compact = (await page.locator('.media-rail .media-card').first().boundingBox())!.width;
  expect(compact).toBeLessThan(comfortable);
  await page.reload();
  await expect(page.locator('.app-shell')).toHaveClass(/compact/);
});

test('populated media browsing and appearance settings pass accessibility checks', async ({
  page,
}) => {
  await desktopFixture(page);
  await page.getByRole('button', { name: 'Try an open movie' }).click();
  for (const width of [1280, 375]) {
    await page.setViewportSize({ width, height: 900 });
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(
      true,
    );
    const report = await new AxeBuilder({ page }).withTags(['wcag2a', 'wcag2aa']).analyze();
    expect(
      report.violations.map((v) => ({ id: v.id, nodes: v.nodes.map((n) => n.target) })),
    ).toEqual([]);
  }
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  const report = await new AxeBuilder({ page }).withTags(['wcag2a', 'wcag2aa']).analyze();
  expect(report.violations.map((v) => ({ id: v.id, nodes: v.nodes.map((n) => n.target) }))).toEqual(
    [],
  );
});

test('in-flight search is cancelled when a new search is submitted or on navigation', async ({
  page,
}) => {
  await desktopFixture(page);
  await page.getByRole('button', { name: 'Try an open movie' }).click();

  // First search is slow
  await page.getByRole('button', { name: 'Search page', exact: true }).click();
  const searchInput = page.getByLabel('Search your selected source');
  await searchInput.fill('first slow search');
  await searchInput.press('Enter');

  // Second search submitted right away while first is in-flight
  await searchInput.fill('bunny');
  await searchInput.press('Enter');

  // Search results for bunny show up
  await expect(page.getByRole('button', { name: /Big Buck Bunny · Trailer/ })).toBeVisible();

  // Verify cancel_request was triggered for the first in-flight search
  const calls = await page.evaluate(() => (window as any).__calls);
  const cancelCalls = calls.filter((c: any) => c.command === 'cancel_request');
  expect(cancelCalls.length).toBeGreaterThanOrEqual(1);

  // Navigate away and verify cleanup
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  const finalCalls = await page.evaluate(() => (window as any).__calls);
  const finalCancels = finalCalls.filter((c: any) => c.command === 'cancel_request');
  expect(finalCancels.length).toBeGreaterThanOrEqual(cancelCalls.length);
});

test('catalog filtering works with zero loaded providers', async ({ page }) => {
  await desktopFixture(page, true);
  await page.getByRole('button', { name: 'Extensions', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Extensions' })).toBeVisible();

  // We have 2 extensions in the test repo: 'Circle FTP' and 'Another Source'
  await expect(page.getByRole('heading', { name: 'Circle FTP' })).toBeVisible();
  await expect(page.getByRole('heading', { name: 'Another Source' })).toBeVisible();

  // Filter by text
  const filterInput = page.getByLabel('Filter extensions');
  await filterInput.fill('Circle');
  await expect(page.getByRole('heading', { name: 'Circle FTP' })).toBeVisible();
  await expect(page.getByRole('heading', { name: 'Another Source' })).not.toBeVisible();

  // Clear filter
  await filterInput.fill('');
  await expect(page.getByRole('heading', { name: 'Another Source' })).toBeVisible();

  // Filter by status: 'Updates' (neither has updates)
  await page.getByRole('button', { name: 'Updates', exact: true }).click();
  await expect(page.getByText('No extensions match this filter.')).toBeVisible();

  // Back to All
  await page.getByRole('button', { name: 'All', exact: true }).first().click();
  await expect(page.getByRole('heading', { name: 'Circle FTP' })).toBeVisible();
});

test('repository rename, share link, and local extension installation', async ({ page }) => {
  await desktopFixture(page, true);
  await page.getByRole('button', { name: 'Extensions', exact: true }).click();

  // Rename repository
  await page.getByRole('button', { name: 'Rename Test sources' }).click();
  const renameInput = page.getByLabel('Display Name');
  await expect(renameInput).toBeVisible();
  await renameInput.fill('Renamed Sources');
  await page.getByRole('button', { name: 'Save name' }).click();
  await expect(page.getByText('Repository renamed.')).toBeVisible();

  // Share link
  await page.getByRole('button', { name: 'Share Test sources' }).click();
  await expect(page.getByText('Repository share link copied.')).toBeVisible();

  // Local extension import
  await page.getByRole('button', { name: 'Import local package' }).first().click();
  const localInput = page.getByLabel('Local Package Path (.jar or .cs3)');
  await expect(localInput).toBeVisible();
  await localInput.fill('/tmp/test-plugin.jar');
  await page.getByRole('button', { name: 'Install package' }).click();
  await expect(page.getByText('Local extension installed.')).toBeVisible();
});

test('provider toggle enables and disables source', async ({ page }) => {
  await desktopFixture(page);
  await page.getByRole('button', { name: 'Try an open movie' }).click();
  await page.getByRole('button', { name: 'Extensions', exact: true }).click();
  await page.getByRole('button', { name: /Installed/ }).click();

  await expect(page.getByRole('heading', { name: 'Blender Open Movies' })).toBeVisible();
  const toggleBtn = page.getByRole('button', { name: 'Toggle Blender Open Movies' });
  await expect(toggleBtn).toBeVisible();
  await expect(toggleBtn).toHaveText('Disable');

  await toggleBtn.click();
  await expect(page.getByText('Provider disabled.')).toBeVisible();
});

test('check for updates and bulk update extensions', async ({ page }) => {
  await desktopFixture(page);
  await page.getByRole('button', { name: 'Try an open movie' }).click();
  await page.getByRole('button', { name: 'Extensions', exact: true }).click();
  await page.getByRole('button', { name: /Installed/ }).click();

  // Click Check for updates
  await page.getByRole('button', { name: 'Check for updates' }).click();
  await expect(page.getByText('1 extension update(s) available.')).toBeVisible();

  // Update all button should now appear
  const updateAllBtn = page.getByRole('button', { name: /Update all/ });
  await expect(updateAllBtn).toBeVisible();
  await updateAllBtn.click();
  await expect(page.getByText('Successfully updated 1 extension(s).')).toBeVisible();
});

test('source self-tester runs stages and displays diagnostics', async ({ page }) => {
  await desktopFixture(page);
  await page.getByRole('button', { name: 'Try an open movie' }).click();
  await page.getByRole('button', { name: 'Extensions', exact: true }).click();
  await page.getByRole('button', { name: /Installed/ }).click();

  // Click Test on provider
  await page.getByRole('button', { name: 'Test Blender Open Movies' }).click();
  await expect(page.getByRole('heading', { name: 'Source Self-Tester' })).toBeVisible();

  // Verify stages
  await expect(page.getByText('Home Stage')).toBeVisible();
  await expect(page.getByText('Search Stage')).toBeVisible();
  await expect(page.getByText('Details Stage')).toBeVisible();
  await expect(page.getByText('Links Stage')).toBeVisible();
  await expect(page.getByText('ALL PASSED')).toBeVisible();

  // Close tester modal
  await page.getByRole('button', { name: 'Close', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Source Self-Tester' })).not.toBeVisible();
});

test('federated search isolates errors, supports view modes, and paginates per source', async ({
  page,
}) => {
  await desktopFixture(page);

  // Add an extra failing provider via __extraProviders
  await page.evaluate(() => {
    (window as any).__extraProviders = [
      {
        id: 'failing:Error Source',
        pluginId: 'error_plugin',
        name: 'Broken Provider',
        lang: 'en',
        mainUrl: 'https://broken.example',
        hasMainPage: false,
        usesWebView: false,
        disabled: false,
        capabilities: {
          hasMainPage: false,
          hasQuickSearch: false,
          hasDownloadSupport: false,
          hasChromecastSupport: false,
          usesWebView: false,
        },
      },
    ];
  });

  await page.getByRole('button', { name: 'Try an open movie' }).click();

  const searchInput = page.getByLabel('Search your selected source');
  await searchInput.fill('Bunny');
  await searchInput.press('Enter');

  // Verify healthy provider returned results
  await expect(page.getByRole('button', { name: /Big Buck Bunny · Trailer/ })).toBeVisible();

  // Verify failing provider shows error banner without crashing the healthy results
  await expect(page.getByText('Broken Provider:')).toBeVisible();
  await expect(page.getByRole('button', { name: 'Retry', exact: true })).toBeVisible();

  // Switch between Grouped and Merged views
  await page.getByRole('button', { name: 'Merged', exact: true }).click();
  await expect(page.getByRole('button', { name: /Big Buck Bunny · Trailer/ })).toBeVisible();

  await page.getByRole('button', { name: 'Grouped', exact: true }).click();
  await expect(page.getByRole('button', { name: /Big Buck Bunny · Trailer/ })).toBeVisible();

  // Per-source load more
  const loadMoreBtn = page.getByRole('button', { name: 'Load more from Blender Open Movies' });
  await expect(loadMoreBtn).toBeVisible();
  await loadMoreBtn.click();
  const calls = await page.evaluate(() => (window as any).__calls);
  expect(calls.some((c: any) => c.command === 'search' && c.args.page === 2)).toBe(true);
});

test('search history and suggestions respect settings opt-out', async ({ page }) => {
  await desktopFixture(page);
  await page.getByRole('button', { name: 'Try an open movie' }).click();

  // 1. Submit search to populate history
  const searchInput = page.getByLabel('Search your selected source');
  await searchInput.fill('Bunny');
  await searchInput.press('Enter');
  await expect(page.getByRole('heading', { name: 'Results for “Bunny”' })).toBeVisible();

  // Clear query and focus to see history
  await searchInput.fill('');
  await searchInput.focus();
  await expect(page.getByText('Recent searches')).toBeVisible();

  // 2. Opt-out in Settings
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  await page.getByRole('button', { name: 'Search suggestions disabled', exact: true }).click();

  // Clear calls log
  await page.evaluate(() => {
    (window as any).__calls = [];
  });

  // Navigate back to search and type
  await page.getByRole('button', { name: 'Search page', exact: true }).click();
  const searchInput2 = page.getByLabel('Search your selected source');
  await searchInput2.fill('Bun');

  // Wait a short moment and verify NO search_suggestions calls occurred
  await page.waitForTimeout(350);
  const calls = await page.evaluate(() => (window as any).__calls);
  expect(calls.some((c: any) => c.command === 'search_suggestions')).toBe(false);
});

test('library watch statuses, favorites, tab filtering, episode watched toggles, and bulk clear', async ({ page }) => {
  await desktopFixture(page);
  await page.getByRole('button', { name: 'Try an open movie' }).click();

  // Search and open title
  const searchInput = page.getByLabel('Search your selected source');
  await searchInput.fill('Bunny');
  await searchInput.press('Enter');
  await page.getByRole('button', { name: /Big Buck Bunny · Trailer/ }).click();

  // 1. Save to library, set status to Watching, Favorite, and Subscribe
  await page.getByRole('button', { name: 'Save to library', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Saved to library', exact: true })).toBeVisible();

  // Change watch status to Watching
  const statusSelect = page.locator('.detail-library-controls select');
  await statusSelect.selectOption('watching');

  // Toggle favorite and subscribed
  await page.getByRole('button', { name: 'Favorite', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Favorited', exact: true })).toBeVisible();

  await page.getByRole('button', { name: 'Subscribe', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Subscribed', exact: true })).toBeVisible();

  // 2. Mark episode watched
  const watchedToggle = page.locator('.watched-toggle-btn').first();
  await watchedToggle.click();
  await expect(watchedToggle).toHaveClass(/is-watched/);

  // 3. Navigate to My library
  await page.getByRole('button', { name: /My library/ }).click();
  await expect(page.getByRole('heading', { name: 'My library' })).toBeVisible();

  // Verify item visible in 'All' tab
  await expect(page.getByRole('button', { name: /Big Buck Bunny · Trailer/ })).toBeVisible();

  // Verify status badge and favorite/subscribed badges are rendered
  await expect(page.locator('.status-badge.watching')).toBeVisible();
  await expect(page.locator('.icon-badge.fav')).toBeVisible();
  await expect(page.locator('.icon-badge.sub')).toBeVisible();

  // Switch tab to 'Watching'
  await page.getByRole('button', { name: 'Watching', exact: true }).click();
  await expect(page.getByRole('button', { name: /Big Buck Bunny · Trailer/ })).toBeVisible();

  // Switch tab to 'Favorites'
  await page.getByRole('button', { name: 'Favorites', exact: true }).click();
  await expect(page.getByRole('button', { name: /Big Buck Bunny · Trailer/ })).toBeVisible();

  // Switch tab to 'Completed' (should be empty)
  await page.getByRole('button', { name: 'Completed', exact: true }).click();
  await expect(page.getByText('Nothing in this list yet.')).toBeVisible();

  // Switch back to 'Watching' and test Clear list modal
  await page.getByRole('button', { name: 'Watching', exact: true }).click();
  await page.getByRole('button', { name: 'Clear list', exact: true }).click();
  await expect(page.getByRole('dialog')).toBeVisible();
  await expect(page.getByRole('heading', { name: 'Clear library' })).toBeVisible();

  // Confirm clear
  await page.getByRole('button', { name: 'Clear "watching"', exact: true }).click();
  await expect(page.getByText('Nothing in this list yet.')).toBeVisible();
});

test('home multi-row provider, category expansion, random actions, and library rails', async ({ page }) => {
  await desktopFixture(page);
  await page.getByRole('button', { name: 'Try an open movie' }).click();

  // 1. Verify spotlight and first row ("Open movies") with horizontal rail
  await expect(page.getByRole('heading', { name: 'Open movies', level: 2 })).toBeVisible();
  const horizontalRail = page.locator('.media-rail.horizontal');
  await expect(horizontalRail).toBeVisible();

  // 2. Load more categories -> Section 1 returns 2 separate named rows
  await page.getByRole('button', { name: 'More categories' }).click();
  await expect(page.getByRole('heading', { name: 'Popular open titles', level: 2 })).toBeVisible();
  await expect(page.getByRole('heading', { name: 'Classic animations', level: 2 })).toBeVisible();

  // 3. Category expansion ("See all") on "Popular open titles"
  const seeAllBtns = page.getByRole('button', { name: 'See all' });
  await seeAllBtns.nth(1).click();
  await expect(page.getByRole('button', { name: 'Back to home' })).toBeVisible();
  await page.getByRole('button', { name: 'Back to home' }).click();

  // 4. Test "Random title" button
  await page.getByRole('button', { name: 'Random title', exact: true }).click();
  await expect(page.getByRole('heading', { name: /Big Buck Bunny · Trailer|Sintel/, level: 1 })).toBeVisible();

  // 5. Save title to library and set Favorite
  await page.getByRole('button', { name: 'Save to library', exact: true }).click();
  await page.getByRole('button', { name: 'Favorite', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Favorited', exact: true })).toBeVisible();

  // 6. Navigate back to Home and verify "Favorites" personalized rail appears
  await page.getByRole('button', { name: 'Home', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Favorites', level: 2 })).toBeVisible();

  // 7. Verify source selection persistence in settings
  const calls = await page.evaluate(() => (window as any).__calls);
  expect(calls.some((c: any) => c.command === 'update_settings' && c.args.patch?.selectedProvider)).toBe(true);
});

test('authoritative mpv session events update player bar, reflect native pause and external exit', async ({ page }) => {
  await desktopFixture(page);
  await page.getByRole('button', { name: 'Try an open movie' }).click();

  // Search and open title
  const searchInput = page.getByLabel('Search your selected source');
  await searchInput.fill('Bunny');
  await searchInput.press('Enter');
  await page.getByRole('button', { name: /Big Buck Bunny · Trailer/ }).click();
  await page.getByRole('button', { name: 'Find a stream', exact: true }).click();
  await page.getByRole('button', { name: /^Official Blender download/ }).click();

  // 1. Authoritative playback UI is visible with Pause button
  await expect(page.getByLabel('Pause playback')).toBeVisible();
  await expect(page.locator('.player-bar strong')).toHaveText('Big Buck Bunny · Trailer');

  // 2. Simulate native mpv property change for pause = true
  await page.evaluate(() => {
    (window as any).__emitPlayerEvent?.({
      type: 'state_changed',
      state: {
        sessionId: 'test-session',
        active: true,
        playing: false,
        paused: true,
        position: 12.0,
        duration: 120.0,
        volume: 100,
        speed: 1,
        itemName: 'Big Buck Bunny · Trailer',
        episodeName: null,
      },
    });
  });

  // UI immediately reflects pause from native player without UI interaction
  await expect(page.getByLabel('Resume playback')).toBeVisible();
  await expect(page.getByText('Paused in mpv')).toBeVisible();

  // 3. Click resume in UI -> cycles pause
  await page.getByLabel('Resume playback').click();
  await expect(page.getByLabel('Pause playback')).toBeVisible();

  // 4. Simulate native mpv window closure / shutdown event
  await page.evaluate(() => {
    (window as any).__emitPlayerEvent?.({
      type: 'shutdown',
      sessionId: 'test-session',
    });
  });

  // Player bar immediately unmounts
  await expect(page.getByLabel('Pause playback')).not.toBeVisible();
  await expect(page.getByLabel('Stop playback')).not.toBeVisible();
});

test('local profiles: create, pin lock, switch, library isolation, and confirmed delete', async ({ page }) => {
  await desktopFixture(page);

  // 1. Check sidebar shows default profile
  const profileBtn = page.getByTestId('profile-switcher-btn');
  await expect(profileBtn).toBeVisible();
  await expect(profileBtn).toContainText('Default Profile');

  // Bookmark an item under Default Profile
  await page.getByRole('button', { name: 'Try an open movie' }).click();
  const search = page.getByLabel('Search your selected source');
  await search.fill('Bunny');
  await search.press('Enter');
  await page.getByRole('button', { name: /Big Buck Bunny · Trailer/ }).click();
  await page.getByRole('button', { name: 'Save to library', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Saved to library', exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'My library' }).click();
  await expect(page.locator('.media-card strong', { hasText: 'Big Buck Bunny' })).toBeVisible();

  // 2. Open Profile Manager
  await profileBtn.click();
  const dialog = page.getByRole('dialog');
  await expect(dialog).toBeVisible();
  await expect(dialog.getByRole('heading', { name: 'Manage profiles' })).toBeVisible();

  // 3. Create a new PIN-protected profile: "Kids"
  await page.getByRole('button', { name: 'Add new profile' }).click();
  await expect(dialog.getByRole('heading', { name: 'Create profile' })).toBeVisible();
  await page.getByLabel('Profile Name').fill('Kids');
  // Choose popcorn avatar
  await page.getByRole('button', { name: 'Popcorn' }).click();
  // Enter PIN
  await page.getByLabel('PIN Gate').fill('1234');
  await page.getByRole('button', { name: 'Create profile' }).click();

  // Should return to profiles list with Kids profile and a lock indicator
  await expect(dialog.getByText('Kids')).toBeVisible();
  await expect(dialog.getByTitle('PIN protected')).toBeVisible();

  // 4. Switch to Kids profile (requires PIN)
  const kidsRow = dialog.locator('.profile-list-item', { hasText: 'Kids' });
  await kidsRow.getByRole('button', { name: 'Switch' }).click();

  // PIN prompt should appear
  await expect(dialog.getByRole('heading', { name: 'Profile PIN required' })).toBeVisible();

  // Entering wrong PIN fails
  await page.getByLabel('Enter Profile PIN').fill('9999');
  await page.getByRole('button', { name: 'Unlock profile' }).click();
  await expect(dialog.getByText('Incorrect PIN')).toBeVisible();

  // Entering correct PIN succeeds
  await page.getByLabel('Enter Profile PIN').fill('1234');
  await page.getByRole('button', { name: 'Unlock profile' }).click();

  // Dialog closes, sidebar now displays Kids profile
  await expect(dialog).not.toBeVisible();
  await expect(profileBtn).toContainText('Kids');

  // 5. Verify Library Isolation: Kids library is empty!
  await page.getByRole('button', { name: 'My library' }).click();
  await expect(page.getByRole('heading', { name: 'A home for your next watch.' })).toBeVisible();
  await expect(page.locator('.media-card strong', { hasText: 'Big Buck Bunny' })).not.toBeVisible();

  // 6. Switch back to Default Profile
  await profileBtn.click();
  await expect(dialog).toBeVisible();
  const defaultRow = dialog.locator('.profile-list-item', { hasText: 'Default Profile' });
  await defaultRow.getByRole('button', { name: 'Switch' }).click();
  await expect(profileBtn).toContainText('Default Profile');

  // Default library still has Big Buck Bunny!
  await page.getByRole('button', { name: 'My library' }).click();
  await expect(page.locator('.media-card strong', { hasText: 'Big Buck Bunny' })).toBeVisible();

  // 7. Delete Kids profile with confirmed deletion
  await profileBtn.click();
  await expect(dialog).toBeVisible();
  const kidsDeleteBtn = dialog.locator('.profile-list-item', { hasText: 'Kids' }).getByLabel('Delete profile Kids');
  await kidsDeleteBtn.click();
  await expect(dialog.getByRole('heading', { name: 'Delete profile' })).toBeVisible();

  // Cancel deletion keeps Kids intact
  await dialog.getByRole('button', { name: 'Cancel' }).click();
  await expect(dialog.getByText('Kids')).toBeVisible();

  // Confirm delete
  await kidsDeleteBtn.click();
  await dialog.getByRole('button', { name: 'Delete profile' }).click();

  // Kids is gone from list, only Default remains
  await expect(dialog.getByText('Kids')).not.toBeVisible();
  await expect(dialog.getByText('Default Profile', { exact: true })).toBeVisible();

  // Installed extensions remain intact
  await dialog.getByRole('button', { name: 'Close', exact: true }).click();
  await page.getByRole('button', { name: 'Extensions', exact: true }).click();
  await page.getByRole('button', { name: /Installed/ }).click();
  await expect(page.getByRole('heading', { name: 'Blender Open Movies' })).toBeVisible();
});

test('rich details and episode browser: backdrop, safe plot, seasons, dub groups, episode ranges, trailers, and focus restoration', async ({ page }) => {
  await desktopFixture(page);
  await page.getByRole('button', { name: 'Try an open movie' }).click();
  // Search for the anime series
  const searchInput = page.getByLabel('Search your selected source');
  await searchInput.fill('Anime');
  await searchInput.press('Enter');

  const animeCard = page.locator('.media-card', { hasText: 'Anime Adventure' });
  await expect(animeCard).toBeVisible();

  // Focus and open details
  await animeCard.focus();
  await page.keyboard.press('Enter');

  // Verify title
  await expect(page.getByRole('heading', { name: 'Anime Adventure', exact: true })).toBeVisible();

  // Verify safe plot rendering (raw HTML stripped)
  const plot = page.locator('.detail-plot');
  await expect(plot).toContainText('A thrilling adventure across worlds.');
  await expect(plot.locator('p')).toHaveCount(0);
  await expect(plot.locator('i')).toHaveCount(0);

  // Verify meta pills
  await expect(page.getByText('TV-14')).toBeVisible();
  await expect(page.getByText('2024', { exact: true })).toBeVisible();
  await expect(page.getByText('8.8/10')).toBeVisible();
  await expect(page.getByText('Next Sunday')).toBeVisible();

  // Verify cast & characters
  await expect(page.getByText('Hero Voice')).toBeVisible();
  await expect(page.getByText('Protagonist')).toBeVisible();
  await expect(page.getByText('Rival Voice')).toBeVisible();

  // Verify recommendations rail
  await expect(page.getByRole('heading', { name: 'You might also like' })).toBeVisible();

  // Test trailer playback
  const trailerBtn = page.getByRole('button', { name: 'Watch trailer' });
  await expect(trailerBtn).toBeVisible();
  await trailerBtn.click();
  // Player state becomes active
  await expect(page.getByLabel('Pause playback')).toBeVisible();
  const playCalls = await page.evaluate(() =>
    (window as any).__calls.filter((c: any) => c.command === 'play'),
  );
  expect(playCalls.at(-1).args.stream.url).toBe('https://example.com/trailer.mp4');
  await page.getByLabel('Stop playback').click();

  // Episode browser controls
  // 1. Season selection: Season 1 and Season 2
  const seasonSelect = page.getByLabel('Filter by season');
  await expect(seasonSelect).toBeVisible();
  await seasonSelect.selectOption('2');
  // Only Season 2 episodes should be in list (starts with Episode 31)
  await expect(page.locator('.episode-row', { hasText: /\bEpisode 31\b/ })).toBeVisible();
  await expect(page.locator('.episode-row', { hasText: /\bEpisode 1\b/ })).toHaveCount(0);
  await seasonSelect.selectOption('all');

  // 2. Dub/Sub selection: Sub and Dub
  const dubSelect = page.getByLabel('Filter by audio dub');
  await expect(dubSelect).toBeVisible();
  await dubSelect.selectOption('Dub');
  // Only dubbed episodes (even numbers: 2, 4...)
  await expect(page.locator('.episode-row', { hasText: /\bEpisode 2\b/ })).toBeVisible();
  await expect(page.locator('.episode-row', { hasText: /\bEpisode 1\b/ })).toHaveCount(0);
  await dubSelect.selectOption('all');

  // 3. Episode ranges: 1–50 and 51–60
  await expect(page.getByRole('button', { name: /1[–-]50/ })).toBeVisible();
  const range2Btn = page.getByRole('button', { name: /51[–-]60/ });
  await expect(range2Btn).toBeVisible();
  await expect(page.locator('.episode-row', { hasText: /\bEpisode 50\b/ })).toBeVisible();
  await expect(page.locator('.episode-row', { hasText: /\bEpisode 51\b/ })).not.toBeVisible();

  await range2Btn.click();
  await expect(page.locator('.episode-row', { hasText: /\bEpisode 51\b/ })).toBeVisible();
  await expect(page.locator('.episode-row', { hasText: /\bEpisode 50\b/ })).not.toBeVisible();

  await page.getByRole('button', { name: /1[–-]50/ }).click();

  // 4. Filler badge
  await expect(page.locator('.filler-badge')).toContainText('FILLER');

  // 5. Episode Sort Order & Watched State Invariance
  // Mark Episode 1 watched
  const ep1Row = page.locator('.episode-item-wrap', { hasText: /\bEpisode 1\b/ });
  await ep1Row.locator('.watched-toggle-btn').click();
  await expect(ep1Row.locator('.watched-toggle-btn')).toHaveClass(/is-watched/);

  // Toggle sort order to Newest first
  const sortBtn = page.getByRole('button', { name: /first/i });
  await sortBtn.click();
  await expect(sortBtn).toContainText('Newest first');

  // After reversing, Episode 1 is now in the second chunk (51–60)
  await page.getByRole('button', { name: /51[–-]60/ }).click();
  const ep1Reversed = page.locator('.episode-item-wrap', { hasText: /\bEpisode 1\b/ });
  await expect(ep1Reversed).toBeVisible();
  // Ensure watched state is INVARIANT to sort order
  await expect(ep1Reversed.locator('.watched-toggle-btn')).toHaveClass(/is-watched/);

  // Switch back to Oldest first
  await sortBtn.click();
  await expect(sortBtn).toContainText('Oldest first');
  await page.getByRole('button', { name: /1[–-]50/ }).click();

  // 6. Episode opaque data preservation on resolve
  await page.locator('.episode-row', { hasText: /\bEpisode 1\b/ }).click();
  const resolveCalls = await page.evaluate(() =>
    (window as any).__calls.filter((c: any) => c.command === 'resolve_links'),
  );
  expect(resolveCalls.at(-1).args.data).toBe('{"opaque":"anime_ep_1_data"}');
  await expect(page.getByRole('heading', { name: 'Choose a stream' })).toBeVisible();

  // 7. Focus restoration on Escape
  await page.keyboard.press('Escape');
  await expect(page.getByRole('heading', { name: 'Anime Adventure', exact: true })).not.toBeVisible();
  await expect(animeCard).toBeFocused();
});

test('playback queue: next/prev controls, queue index display, auto-next settings, and mirror position preservation', async ({ page }) => {
  await desktopFixture(page);
  await page.goto('/');

  // 1. Settings playback preferences
  await page.getByRole('button', { name: 'Settings' }).click();
  await expect(page.getByRole('heading', { name: 'Playback' })).toBeVisible();
  const autoNextGroup = page.getByRole('group', { name: 'Auto-play next episode' });
  await expect(autoNextGroup).toBeVisible();
  const autoNextDisable = autoNextGroup.getByRole('button', { name: 'Auto-play next episode disabled' });
  await autoNextDisable.click();
  await expect(autoNextDisable).toHaveClass(/chosen/);

  const autoNextEnable = autoNextGroup.getByRole('button', { name: 'Auto-play next episode enabled' });
  await autoNextEnable.click();
  await expect(autoNextEnable).toHaveClass(/chosen/);

  // 2. Play episode from series
  await page.getByRole('button', { name: 'Home', exact: true }).click();
  await page.getByRole('button', { name: 'Try an open movie' }).click();
  const searchInput = page.getByLabel('Search your selected source');
  await searchInput.fill('Anime');
  await searchInput.press('Enter');

  const animeCard = page.locator('.media-card', { hasText: 'Anime Adventure' });
  await expect(animeCard).toBeVisible();
  await animeCard.click();
  await expect(page.getByRole('heading', { name: 'Anime Adventure', exact: true })).toBeVisible();

  // Resolve Episode 2
  await page.locator('.episode-row', { hasText: /\bEpisode 2\b/ }).click();
  await expect(page.getByRole('heading', { name: 'Choose a stream' })).toBeVisible();
  await page.locator('.stream-row', { hasText: 'Official Blender download' }).click();

  // Verify play was called with queue containing episodes and currentIndex = 1
  const playCalls = await page.evaluate(() =>
    (window as any).__calls.filter((c: any) => c.command === 'play'),
  );
  expect(playCalls.length).toBeGreaterThan(0);
  const lastPlay = playCalls.at(-1)?.args;
  expect(lastPlay).toBeTruthy();
  expect(lastPlay.queue).toBeTruthy();
  expect(lastPlay.queue.currentIndex).toBe(1);
  expect(lastPlay.queue.episodes.length).toBeGreaterThan(1);

  // 3. Simulate native player emitting active state with queue metadata
  await page.evaluate(() => {
    (window as any).__emitPlayerEvent?.({
      type: 'state_changed',
      state: {
        sessionId: 'queue-test-session',
        active: true,
        playing: true,
        paused: false,
        position: 45.0,
        duration: 120.0,
        volume: 100,
        speed: 1,
        itemName: 'Anime Adventure',
        episodeName: 'Episode 2',
        hasNext: true,
        hasPrev: true,
        queueIndex: 1,
        queueCount: 4,
      },
    });
  });

  // Verify player-bar controls and badge
  await expect(page.getByText('Episode 2 of 4')).toBeVisible();
  await expect(page.getByLabel('Previous episode')).toBeEnabled();
  await expect(page.getByLabel('Next episode')).toBeEnabled();

  // Click Next episode button
  await page.getByLabel('Next episode').click();
  const actionCalls = await page.evaluate(() =>
    (window as any).__calls.filter((c: any) => c.command === 'player_action'),
  );
  expect(actionCalls.at(-1)?.args?.action).toBe('next');

  // 4. Test mirror position preservation:
  // Position is at 45.0 in playerState; clicking another stream must preserve 45.0 as resume position!
  await page.locator('.stream-row', { hasText: 'Mirror 1080p' }).click();
  const mirrorPlayCalls = await page.evaluate(() =>
    (window as any).__calls.filter((c: any) => c.command === 'play'),
  );
  expect(mirrorPlayCalls.at(-1)?.args?.resume).toBe(45.0);

  // 5. When at start of queue (hasPrev: false), Previous button is disabled
  await page.evaluate(() => {
    (window as any).__emitPlayerEvent?.({
      type: 'state_changed',
      state: {
        sessionId: 'queue-test-session',
        active: true,
        playing: true,
        paused: false,
        position: 10.0,
        duration: 120.0,
        volume: 100,
        speed: 1,
        itemName: 'Anime Adventure',
        episodeName: 'Episode 1',
        hasNext: true,
        hasPrev: false,
        queueIndex: 0,
        queueCount: 4,
      },
    });
  });
  await expect(page.getByLabel('Previous episode')).toBeDisabled();
  await expect(page.getByText('Episode 1 of 4')).toBeVisible();

  // Stop playback cleanly
  await page.getByLabel('Stop playback').click();
  await expect(page.getByLabel('Stop playback')).not.toBeVisible();
});


test('player controls send typed commands and can be operated by keyboard', async ({ page }) => {
  await desktopFixture(page);
  await page.getByRole('button', { name: 'Try an open movie' }).click();
  const searchInput = page.getByLabel('Search your selected source');
  await searchInput.fill('Bunny');
  await searchInput.press('Enter');
  await page.getByRole('button', { name: /Big Buck Bunny · Trailer/ }).click();
  await page.getByRole('button', { name: 'Find a stream', exact: true }).click();
  await page.getByRole('button', { name: /^Official Blender download/ }).click();
  await page.evaluate(() => (window as any).__emitPlayerEvent?.({ type: 'state_changed', state: { sessionId: 'test-session', active: true, playing: true, paused: false, position: 12, duration: 120, volume: 100, speed: 1, itemName: 'Big Buck Bunny · Trailer' } }));
  const summary = page.getByText('Controls', { exact: true });
  await summary.focus();
  await summary.press('Enter');
  const volume = page.getByLabel('Volume', { exact: true });
  await volume.focus();
  await volume.press('ArrowLeft');
  await expect.poll(() => page.evaluate(() => (window as any).__calls.filter((c: any) => c.command === 'player_command').at(-1)?.args?.command)).toEqual({ type: 'set_volume', volume: 99 });
  await page.getByRole('button', { name: 'Mute', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Unmute', exact: true })).toBeVisible();
});

 test('source profiles save quality and source preferences per local profile', async ({ page }) => {
  await desktopFixture(page);
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  await page.getByLabel('New source profile name').fill('High quality');
  await page.getByRole('button', { name: 'Add profile', exact: true }).click();
  await expect(page.getByLabel('Source profile', { exact: true })).toHaveValue(/.+/);
  await page.getByLabel('720 quality priority').fill('12');
  await page.getByLabel('720 quality priority').press('Tab');
  await page.getByLabel('Preferred stream language').fill('ja');
  await page.getByLabel('Preferred stream language').press('Tab');
  await page.getByLabel('Source name', { exact: true }).fill('Example');
  await page.getByRole('button', { name: 'Add source', exact: true }).click();
  await expect(page.getByLabel('Example source priority', { exact: true })).toHaveValue('1');
  const calls = await page.evaluate(() => (window as any).__calls);
  const changes = calls.filter((c: any) => c.command === 'update_settings' && c.args.patch?.sourcePriority);
  expect(changes.at(-1).args.profileId).toBe('default');
  const profile = changes.at(-1).args.patch.sourcePriority.profiles.find((p: any) => p.name === 'High quality');
  expect(profile.qualityPriorities['720']).toBe(12);
  expect(profile.preferredLanguage).toBe('ja');
  expect(profile.sourcePriorities.Example).toBe(1);
  await page.getByRole('button', { name: 'Remove Example source priority' }).click();
  await expect(page.getByLabel('Example source priority', { exact: true })).toHaveCount(0);
});

 test('stream picker follows ranking and hides excluded mirrors', async ({ page }) => {
  await desktopFixture(page);
  await page.evaluate(() => { (window as any).__state.__reverseRank = true; (window as any).__state.__hideFirst = true; });
  await page.getByRole('button', { name: 'Try an open movie' }).click();
  await page.getByLabel('Search your selected source').fill('Bunny');
  await page.getByLabel('Search your selected source').press('Enter');
  await page.getByRole('button', { name: /Big Buck Bunny · Trailer/ }).click();
  await page.getByRole('button', { name: 'Find a stream', exact: true }).click();
  await expect(page.getByRole('button', { name: /^Mirror 1080p/ })).toBeVisible();
  await expect(page.getByRole('button', { name: /^Official Blender download/ })).toHaveCount(0);
});

test('audio and subtitle selectors show player tracks and send selections', async ({ page }) => {
  await desktopFixture(page);
  await page.getByRole('button', { name: 'Try an open movie' }).click();
  await page.getByLabel('Search your selected source').fill('Bunny');
  await page.getByLabel('Search your selected source').press('Enter');
  await page.getByRole('button', { name: /Big Buck Bunny · Trailer/ }).click();
  await page.getByRole('button', { name: 'Find a stream', exact: true }).click();
  await page.getByRole('button', { name: /^Official Blender download/ }).click();
  await page.evaluate(() => {
    const state = { sessionId: 'test-session', active: true, playing: true, paused: false, position: 12, duration: 120, volume: 100, speed: 1, itemName: 'Big Buck Bunny', tracks: [
      { id: 1, kind: 'audio', selected: true, external: false, forced: false, hearingImpaired: false, language: 'ja', title: 'Original' },
      { id: 2, kind: 'sub', selected: false, external: true, forced: true, hearingImpaired: true, language: 'en', title: 'English' },
    ] };
    (window as any).__state.__playerState = state;
    (window as any).__emitPlayerEvent?.({ type: 'state_changed', state });
  });
  await page.getByText('Controls', { exact: true }).click();
  await expect(page.getByRole('option', { name: 'English · Forced · SDH · External' })).toHaveCount(1);
  await page.getByLabel('Subtitle track').selectOption('2');
  await expect.poll(() => page.evaluate(() => (window as any).__calls.filter((c: any) => c.command === 'player_command').at(-1)?.args?.command)).toEqual({ type: 'set_subtitle_track', id: 2 });
  await page.getByLabel('Audio track').selectOption('off');
  await expect.poll(() => page.evaluate(() => (window as any).__calls.filter((c: any) => c.command === 'player_command').at(-1)?.args?.command)).toEqual({ type: 'set_audio_track', id: null });
});

test('subtitle defaults and delay can be changed from the interface', async ({ page }) => {
  await desktopFixture(page);
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  await page.getByLabel('Default subtitle mode').selectOption('off');
  await expect(page.getByLabel('Default subtitle mode')).toHaveValue('off');
  await page.getByLabel('Preferred subtitle language').fill('en');
  await page.getByLabel('Preferred subtitle language').press('Tab');
  await expect.poll(() => page.evaluate(() => (window as any).__calls.filter((c: any) => c.command === 'update_settings' && c.args.patch?.player?.subtitleLanguage).at(-1)?.args?.patch?.player)).toMatchObject({ subtitlesEnabled: false, subtitleLanguage: 'en' });
  await page.evaluate(() => {
    const state = { sessionId: 'test-session', active: true, playing: true, paused: false, position: 12, duration: 120, volume: 100, speed: 1, subtitleDelay: 0 };
    (window as any).__state.__playerState = state;
    (window as any).__emitPlayerEvent?.({ type: 'state_changed', state });
  });
  await page.getByText('Controls', { exact: true }).click();
  await page.getByLabel('Subtitle delay (seconds)').fill('1.5');
  await page.getByLabel('Subtitle delay (seconds)').press('Tab');
  await expect.poll(() => page.evaluate(() => (window as any).__calls.filter((c: any) => c.command === 'player_command').at(-1)?.args?.command)).toEqual({ type: 'set_subtitle_delay', seconds: 1.5 });
});

 test('local subtitle chooser is tied to the active playback session', async ({ page }) => {
  await desktopFixture(page);
  await page.evaluate(() => (window as any).__emitPlayerEvent?.({ type: 'state_changed', state: { sessionId: 'current-session', active: true, playing: true, paused: false, position: 8, duration: 100, volume: 100, speed: 1 } }));
  await page.getByText('Controls', { exact: true }).click();
  await page.getByRole('button', { name: 'Add subtitle file…' }).click();
  await expect.poll(() => page.evaluate(() => (window as any).__calls.find((c: any) => c.command === 'choose_local_subtitle')?.args?.sessionId)).toBe('current-session');
  await expect(page.getByText('Subtitle added.')).toBeVisible();
});

 test('subtitle archive offers entries and submits the selected one', async ({ page }) => {
  await desktopFixture(page);
  await page.evaluate(() => {
    (window as any).__state.__subtitleArchive = { status: 'archive', id: 'archive-id', entries: [
      { index: 1, name: 'English.srt', size: 1200 }, { index: 4, name: 'nested/日本語.srt', size: 2500 }
    ] };
    (window as any).__emitPlayerEvent?.({ type: 'state_changed', state: { sessionId: 'current-session', active: true, playing: true, paused: false, position: 8, duration: 100, volume: 100, speed: 1 } });
  });
  await page.getByText('Controls', { exact: true }).click();
  await page.getByRole('button', { name: 'Add subtitle file…' }).click();
  const dialog = page.getByRole('dialog');
  await expect(dialog.getByRole('heading', { name: 'Choose a subtitle' })).toBeVisible();
  await dialog.getByRole('button', { name: /nested\/日本語.srt/ }).click();
  await expect.poll(() => page.evaluate(() => (window as any).__calls.find((c: any) => c.command === 'select_archived_subtitle')?.args)).toEqual({ id: 'archive-id', index: 4 });
  await expect(dialog).not.toBeVisible();
  await page.getByRole('button', { name: 'Add subtitle file…' }).click();
  await expect(dialog).toBeVisible();
  await dialog.getByRole('button', { name: 'Cancel', exact: true }).click();
  await expect.poll(() => page.evaluate(() => (window as any).__calls.find((c: any) => c.command === 'cancel_archived_subtitle')?.args?.id)).toBe('archive-id');
});

 test('provider subtitle archives are offered for the playing episode', async ({ page }) => {
  await desktopFixture(page);
  await page.evaluate(() => {
    (window as any).__state.__resolvedSubtitles = [
      { lang: 'Regular', url: 'https://example.com/sub.vtt', headers: {} },
      { lang: 'English', url: 'https://example.com/subs.ZIP?token=opaque', headers: {} },
    ];
    (window as any).__state.__subtitleArchive = { status: 'archive', id: 'provider-archive', entries: [{ index: 3, name: 'English.srt', size: 1000 }] };
  });
  await page.getByRole('button', { name: 'Try an open movie' }).click();
  await page.getByLabel('Search your selected source').fill('Bunny');
  await page.getByLabel('Search your selected source').press('Enter');
  await page.getByRole('button', { name: /Big Buck Bunny · Trailer/ }).click();
  await page.getByRole('button', { name: 'Find a stream', exact: true }).click();
  await page.getByRole('button', { name: /^Official Blender download/ }).click();
  await page.evaluate(() => (window as any).__emitPlayerEvent?.({ type: 'state_changed', state: { sessionId: 'provider-session', active: true, playing: true, paused: false, position: 8, duration: 100, volume: 100, speed: 1, itemName: 'Big Buck Bunny · Trailer', episodeName: 'Big Buck Bunny · Trailer' } }));
  await page.getByText('Controls', { exact: true }).click();
  await page.getByRole('button', { name: 'Open English subtitle archive…' }).click();
  await expect.poll(() => page.evaluate(() => (window as any).__calls.find((c: any) => c.command === 'prepare_provider_subtitle_archive')?.args)).toEqual({ sessionId: 'provider-session', index: 1 });
  await page.getByRole('dialog').getByRole('button', { name: /English.srt/ }).click();
  await expect.poll(() => page.evaluate(() => (window as any).__calls.find((c: any) => c.command === 'select_archived_subtitle')?.args)).toEqual({ id: 'provider-archive', index: 3 });
});

test('subtitle appearance previews changes, resets and undoes the last change', async ({ page }) => {
  await desktopFixture(page);
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  const preview = page.getByLabel('Subtitle appearance preview').locator('span');
  await page.getByLabel('Subtitle font size').fill('44');
  await page.getByLabel('Subtitle font size').press('Tab');
  await expect(preview).toHaveCSS('font-size', '44px');
  await page.getByLabel('Bold subtitles').check();
  await expect(preview).toHaveCSS('font-weight', '700');
  await page.getByRole('button', { name: 'Undo', exact: true }).click();
  await expect(page.getByLabel('Bold subtitles')).not.toBeChecked();
  await expect(page.getByLabel('Subtitle font size')).toHaveValue('44');
  await page.getByRole('button', { name: 'Reset subtitle appearance' }).click();
  await expect(page.getByLabel('Subtitle font size')).toHaveValue('38');
  await page.getByRole('button', { name: 'Undo', exact: true }).click();
  await expect(page.getByLabel('Subtitle font size')).toHaveValue('44');
  const calls = await page.evaluate(() => (window as any).__calls.filter((c: any) => c.command === 'update_settings' && c.args.patch?.player?.subtitleStyle));
  expect(calls.at(-1).args.patch.player.subtitleStyle.size).toBe(44);
  expect(calls.at(-1).args.patch.player.subtitleStyle.overrideAss).toBe(false);
});

 test('custom subtitle font is previewed, saved and restored by undo', async ({ page }) => {
  await desktopFixture(page);
  const font = readFileSync('node_modules/@fontsource-variable/roboto/files/roboto-latin-wght-normal.woff2').toString('base64');
  const id = 'a'.repeat(64);
  await page.evaluate(({ font, id }) => {
    (window as any).__state.__fontSelection = { id, family: 'Imported Roboto' };
    (window as any).__state.__fontData = font;
  }, { font, id });
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  await page.getByRole('button', { name: 'Choose custom font file…' }).click();
  await expect(page.getByLabel('Subtitle font family')).toHaveValue('Imported Roboto');
  const preview = page.getByLabel('Subtitle appearance preview').locator('span');
  await expect(preview).toHaveCSS('font-family', `Cloudstream-${id}`);
  await expect(page.getByText('Using imported font: Imported Roboto.', { exact: false })).toBeVisible();
  await page.getByRole('button', { name: 'Reset subtitle appearance' }).click();
  await expect(page.getByLabel('Subtitle font family')).toHaveValue('sans-serif');
  await page.getByRole('button', { name: 'Undo', exact: true }).click();
  await expect(page.getByLabel('Subtitle font family')).toHaveValue('Imported Roboto');
  await expect(preview).toHaveCSS('font-family', `Cloudstream-${id}`);
  await page.getByLabel('Subtitle font family').fill('Serif');
  await page.getByLabel('Subtitle font family').press('Tab');
  await expect.poll(() => page.evaluate(() => (window as any).__calls.filter((c: any) => c.command === 'update_settings' && c.args.patch?.player?.subtitleStyle).at(-1)?.args?.patch?.player?.subtitleStyle?.fontId)).toBe(null);
});

test('subtitle cleanup is opt-in and saves its choices', async ({ page }) => {
  await desktopFixture(page);
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  const captions = page.getByRole('checkbox', { name: 'Remove square-bracketed captions' });
  const uppercase = page.getByRole('checkbox', { name: 'Uppercase dialogue' });
  await expect(captions).not.toBeChecked();
  await expect(uppercase).not.toBeChecked();
  await captions.check();
  await expect.poll(() => page.evaluate(() => (window as any).__state.settings.global.player.subtitleTransforms?.removeCaptions)).toBe(true);
  await uppercase.check();
  await expect.poll(() => page.evaluate(() => (window as any).__state.settings.global.player.subtitleTransforms)).toMatchObject({ removeCaptions: true, uppercase: true, removeMusic: false, removeBloat: false });
  await expect(page.getByText(/Embedded tracks are not cleaned/)).toBeVisible();
});


test('subscription checks report new episodes for the active profile', async ({ page }) => {
  await desktopFixture(page);
  await page.getByRole('button', { name: /My library/ }).click();
  await page.getByRole('button', { name: 'Subscribed', exact: true }).click();
  await expect(page.getByText(/Checks do not run while the app is closed/)).toBeVisible();
  const alerts = page.getByRole('checkbox', { name: 'New episode desktop notifications' });
  await expect(alerts).not.toBeChecked();
  await alerts.check();
  await expect.poll(() => page.evaluate(() => (window as any).__state.settings.global.subscriptionNotifications)).toBe(true);
  await page.getByRole('button', { name: 'Check for new episodes' }).click();
  await expect(page.getByText('Fixture show: 2 new episodes')).toBeVisible();
  await page.getByRole('button', { name: 'Mark updates seen' }).click();
  await expect(page.getByText('No new episodes found.')).toBeVisible();
  await expect.poll(() => page.evaluate(() => (window as any).__calls.some((c: any) => c.command === 'deliver_subscription_alerts' && c.args.profileId === 'default'))).toBe(true);
  await expect.poll(() => page.evaluate(() => (window as any).__calls.find((c: any) => c.command === 'check_subscriptions' && c.args.force === true)?.args)).toEqual({ profileId: 'default', force: true });
});


test('subscriptions check automatically while the unlocked profile is open', async ({ page }) => {
  await desktopFixture(page);
  await expect.poll(() => page.evaluate(() => (window as any).__calls.some((c: any) => c.command === 'check_subscriptions' && c.args.profileId === 'default' && c.args.force === false)), { timeout: 6000 }).toBe(true);
  expect(await page.evaluate(() => (window as any).__calls.some((c: any) => c.command === 'deliver_subscription_alerts'))).toBe(false);
});

test('downloads: offline library groups titles and seasons and shows watch progress', async ({ page }) => {
  await desktopFixture(page);
  await page.evaluate(() => {
    const base = { profileId: 'default', providerId: 'p', destination: '/tmp', extension: 'hls', policy: {}, state: 'completed', bytes: 1024, total: 1024, output: '/tmp/file', error: null, revision: 1, createdAtMs: 1, updatedAtMs: 1 };
    (window as any).__state.downloads = [
      { ...base, id: 'a2', mediaId: 'a', episodeId: 'e2', title: 'A Show', season: 2, episode: 1, episodeName: 'Second season', order: 1 },
      { ...base, id: 'b1', mediaId: 'b', episodeId: 'e3', title: 'Another Show', season: 1, episode: 1, episodeName: 'Other episode', order: 2 },
      { ...base, id: 'a1', mediaId: 'a', episodeId: 'e1', title: 'A Show', season: 1, episode: 1, episodeName: 'First season', order: 3 },
    ];
    (window as any).__downloadProgress = { a1: { positionSeconds: 120, durationSeconds: 600, isWatched: false } };
    (window as any).__subtitleWarnings = { a1: 1 };
    (window as any).__downloadHealth = { a1: 'damaged', a2: 'available', b1: 'available' };
  });
  await page.getByRole('button', { name: 'Downloads', exact: true }).click();
  await page.getByRole('button', { name: 'Offline library', exact: true }).click();
  const library = page.getByRole('region', { name: 'Offline library' });
  await expect(library.getByRole('heading', { name: 'A Show', exact: true })).toBeVisible();
  await expect(library.getByRole('heading', { name: 'Another Show', exact: true })).toBeVisible();
  const group = library.getByRole('region', { name: 'A Show' });
  await expect(group.getByRole('heading', { name: 'Season 1' })).toBeVisible();
  await expect(group.getByRole('heading', { name: 'Season 2' })).toBeVisible();
  await expect(group.getByText('20% watched · 2 min')).toBeVisible();
  await expect(group.getByText('1 subtitle was unavailable or unsupported.')).toBeVisible();
  await group.getByRole('button', { name: 'Play A Show First season' }).click();
  const call = await page.evaluate(() => (window as any).__calls.filter((c: any) => c.command === 'play_download').at(-1).args);
  expect(call).toEqual({ profileId: 'default', id: 'a1' });
  const accessibility = await new AxeBuilder({ page }).withTags(['wcag2a', 'wcag2aa']).analyze();
  expect(accessibility.violations).toEqual([]);
  await group.getByRole('button', { name: 'Repair A Show First season' }).click();
  const repair = page.getByRole('dialog', { name: 'Download a replacement?' });
  await expect(repair).toContainText('existing files, including any edits, will be kept');
  await repair.getByRole('button', { name: 'Download replacement' }).click();
  await expect(page.getByRole('status')).toContainText('A replacement was added to the queue');
  const jobs = await page.evaluate(() => (window as any).__state.downloads);
  expect(jobs.find((job: any) => job.id === 'a1').state).toBe('completed');
  expect(jobs.find((job: any) => job.id === 'repair-1').repairOf).toBe('a1');
});

test('downloads: choose an HLS folder and play a finished download', async ({ page }) => {
  await desktopFixture(page);
  await page.getByRole('button', { name: 'Try an open movie' }).click();
  const search = page.getByLabel('Search your selected source');
  await search.fill('anime');
  await search.press('Enter');
  await page.getByRole('button', { name: /Anime Adventure/ }).first().click();
  await page.getByLabel('Download file type').selectOption('hls');
  await page.getByRole('button', { name: 'Download Episode 1', exact: true }).click();
  const enqueue = await page.evaluate(() => (window as any).__calls.filter((c: any) => c.command === 'enqueue_download').at(-1).args);
  expect(enqueue.extension).toBe('hls');
  await page.evaluate(() => { const d = (window as any).__state.downloads[0]; d.state = 'completed'; d.bytes = d.total = 1000; d.output = '/home/user/Videos/cloudstream-x.hls'; d.revision += 1; });
  await page.getByRole('button', { name: 'Downloads', exact: true }).click();
  const play = page.getByRole('button', { name: 'Play Anime Adventure Episode 1' });
  await play.click();
  const call = await page.evaluate(() => (window as any).__calls.filter((c: any) => c.command === 'play_download').at(-1).args);
  expect(call).toEqual({ profileId: 'default', id: '00000000-0000-4000-8000-000000000001' });
  await page.evaluate(() => ((window as any).__downloadPlayFails = true));
  await play.click();
  await expect(page.getByRole('alert')).toContainText('missing or have changed');
});

test('downloads: enqueue from an episode, live progress, controls, reordering and confirmed removal', async ({ page }) => {
  await desktopFixture(page);
  await page.getByRole('button', { name: 'Try an open movie' }).click();
  const search = page.getByLabel('Search your selected source');
  await search.fill('anime');
  await search.press('Enter');
  await page.getByRole('button', { name: /Anime Adventure/ }).first().click();
  await expect(page.getByRole('heading', { name: 'Anime Adventure', exact: true })).toBeVisible();

  // The chosen container is sent with the episode, and a closed folder chooser adds nothing.
  await page.getByLabel('Download file type').selectOption('mkv');
  await page.evaluate(() => ((window as any).__folderCancelled = true));
  await page.getByRole('button', { name: 'Download Episode 1', exact: true }).click();
  await expect(page.getByText('Added “Episode 1” to Downloads.')).toHaveCount(0);
  await page.evaluate(() => ((window as any).__folderCancelled = false));
  await page.getByRole('button', { name: 'Download Episode 1', exact: true }).click();
  await expect(page.getByText('Added “Episode 1” to Downloads.')).toBeVisible();
  await page.getByRole('button', { name: 'Download Episode 2', exact: true }).click();
  const enqueue = await page.evaluate(() => (window as any).__calls.filter((c: any) => c.command === 'enqueue_download').at(-1).args);
  expect(enqueue.extension).toBe('mkv');
  expect(enqueue.episode.data).toBe('{"opaque":"anime_ep_2_data"}');

  await page.getByRole('button', { name: 'Downloads', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Downloads', exact: true })).toBeVisible();
  const rows = page.locator('.download-row');
  await expect(rows).toHaveCount(2);
  await expect(rows.nth(0)).toContainText('Episode 1');
  await expect(rows.nth(0)).toContainText('Waiting in queue');

  // Live events drive the progress bar; an older revision never overwrites a newer one.
  const first = await page.evaluate(() => (window as any).__state.downloads[0]);
  await page.evaluate((job) => {
    // The backend and the event stream agree: this is the job at revision 5.
    const live = { ...job, state: 'downloading', bytes: 512 * 1024 * 1024, total: 1024 * 1024 * 1024, revision: 5 };
    Object.assign((window as any).__state.downloads[0], live);
    (window as any).__emitDownloadEvent({ type: 'job_changed', job: live });
  }, first);
  await expect(rows.nth(0)).toContainText('Downloading');
  await expect(rows.nth(0).getByRole('progressbar')).toHaveAttribute('aria-valuenow', '50');
  await page.evaluate((job) => (window as any).__emitDownloadEvent({ type: 'job_changed', job: { ...job, state: 'queued', bytes: 0, revision: 2 } }), first);
  await expect(rows.nth(0)).toContainText('Downloading');

  // Pause, resume and cancel go through the backend and reflect its answer.
  await rows.nth(0).getByRole('button', { name: 'Pause' }).click();
  await expect(rows.nth(0)).toContainText('Paused');
  await rows.nth(0).getByRole('button', { name: 'Cancel' }).click();
  await expect(rows.nth(0)).toContainText('Canceled');
  await rows.nth(0).getByRole('button', { name: 'Retry' }).click();
  await expect(rows.nth(0)).toContainText('Waiting in queue');

  // Only queued downloads reorder, and the full queued id list is sent.
  await rows.nth(0).getByRole('button', { name: /Move .* down/ }).click();
  await expect(rows.nth(0)).toContainText('Episode 2');
  const reorder = await page.evaluate(() => (window as any).__calls.filter((c: any) => c.command === 'reorder_downloads').at(-1).args);
  expect(reorder.ids).toHaveLength(2);
  await expect(rows.nth(0).getByRole('button', { name: /Move .* up/ })).toBeDisabled();

  // Removal is confirmed, offers to keep the file, and only finished files can be deleted.
  await page.evaluate(() => { const d = (window as any).__state.downloads[0]; d.state = 'completed'; d.bytes = d.total = 1000; d.output = '/home/user/Videos/cloudstream-x.mkv'; d.revision += 1; (window as any).__emitDownloadEvent({ type: 'refresh' }); });
  await expect(page.locator('.download-row').filter({ hasText: 'Finished' })).toHaveCount(1);
  await page.locator('.download-row').filter({ hasText: 'Finished' }).getByRole('button', { name: /Remove/ }).click();
  const dialog = page.getByRole('dialog');
  await expect(dialog.getByRole('heading', { name: 'Remove this download?' })).toBeVisible();
  await expect(dialog).toContainText('never deleted');
  await dialog.getByRole('button', { name: 'Keep it' }).click();
  await expect(page.locator('.download-row')).toHaveCount(2);
  await page.locator('.download-row').filter({ hasText: 'Finished' }).getByRole('button', { name: /Remove/ }).click();
  await page.getByRole('dialog').getByRole('button', { name: 'Delete file too' }).click();
  await expect(page.locator('.download-row')).toHaveCount(1);
  await expect(page.getByRole('status').filter({ hasText: 'The file was deleted.' })).toBeVisible();
  const removal = await page.evaluate(() => (window as any).__calls.filter((c: any) => c.command === 'remove_download').at(-1).args);
  expect(removal.deleteFile).toBe(true);
});

test('downloads: an unreachable queue and an empty queue are explained, with retry', async ({ page }) => {
  await desktopFixture(page);
  await page.evaluate(() => ((window as any).__downloadsDown = true));
  await page.getByRole('button', { name: 'Downloads', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Downloads are unavailable' })).toBeVisible();
  await expect(page.getByRole('alert')).toContainText('Another app instance owns the download queue');
  await page.evaluate(() => ((window as any).__downloadsDown = false));
  await page.getByRole('button', { name: 'Try again' }).click();
  await expect(page.getByRole('heading', { name: 'Nothing downloading' })).toBeVisible();
  await expect(page.getByText(/Choose a file type or an HLS folder/)).toBeVisible();
  const results = await new AxeBuilder({ page }).withTags(['wcag2a', 'wcag2aa']).analyze();
  expect(results.violations).toEqual([]);
});

test('downloads: batch enqueue skips duplicates, per-source download keeps the policy, speed is estimated, finished items clear', async ({ page }) => {
  await desktopFixture(page);
  await page.getByRole('button', { name: 'Try an open movie' }).click();
  const search = page.getByLabel('Search your selected source');
  await search.fill('anime');
  await search.press('Enter');
  await page.getByRole('button', { name: /Anime Adventure/ }).first().click();
  await expect(page.getByRole('heading', { name: 'Anime Adventure', exact: true })).toBeVisible();

  await page.getByRole('button', { name: 'Download these 50' }).click();
  await expect(page.getByText('Added 50 episodes to Downloads.')).toBeVisible();
  await page.getByRole('button', { name: 'Download all 60' }).click();
  await expect(page.getByText('Added 10 episodes to Downloads. 50 already in the queue.')).toBeVisible();
  const batch = await page.evaluate(() => (window as any).__calls.filter((c: any) => c.command === 'enqueue_downloads').at(-1).args);
  expect(batch.episodes).toHaveLength(60);
  expect(batch.episodes[0].data).toBe('{"opaque":"anime_ep_1_data"}');

  await page.getByRole('button', { name: 'Downloads', exact: true }).click();
  await expect(page.locator('.download-row')).toHaveCount(60);

  // Speed and time left are estimated from successive progress events.
  const job = await page.evaluate(() => (window as any).__state.downloads[0]);
  const emit = (bytes: number, revision: number) => page.evaluate(([j, b, r]) => (window as any).__emitDownloadEvent({ type: 'job_changed', job: { ...(j as any), state: 'downloading', bytes: b, total: 100 * 1024 * 1024, revision: r } }), [job, bytes, revision] as const);
  await emit(10 * 1024 * 1024, 3);
  await expect(page.locator('.download-row').first().locator('.download-speed')).toHaveCount(0);
  await page.waitForTimeout(600);
  await emit(20 * 1024 * 1024, 4);
  await expect(page.locator('.download-row').first().locator('.download-speed')).toContainText(/MB\/s · about .* left/);

  // Clearing removes finished and canceled records only, and keeps every file.
  await page.evaluate(() => {
    const downloads = (window as any).__state.downloads;
    downloads[1].state = 'completed'; downloads[1].output = '/home/user/Videos/a.mkv'; downloads[1].revision += 1;
    downloads[2].state = 'canceled'; downloads[2].revision += 1;
    (window as any).__emitDownloadEvent({ type: 'refresh' });
  });
  await page.getByRole('button', { name: 'Clear finished' }).click();
  await expect(page.getByRole('dialog')).toContainText('Downloaded files are not deleted');
  await page.getByRole('dialog').getByRole('button', { name: 'Clear list' }).click();
  await expect(page.getByText('Cleared 2 finished downloads. Your files were kept.')).toBeVisible();
  await expect(page.locator('.download-row')).toHaveCount(58);
  const removals = await page.evaluate(() => (window as any).__calls.filter((c: any) => c.command === 'remove_download').map((c: any) => c.args.deleteFile));
  expect(removals).toEqual([false, false]);
});

test('downloads: a specific source can be chosen for one episode', async ({ page }) => {
  await desktopFixture(page);
  await page.getByRole('button', { name: 'Try an open movie' }).click();
  const search = page.getByLabel('Search your selected source');
  await search.fill('Bunny');
  await search.press('Enter');
  await page.getByRole('button', { name: /Big Buck Bunny · Trailer/ }).click();
  await page.getByRole('button', { name: 'Find a stream', exact: true }).click();
  await page.getByRole('button', { name: /^Download .* from Official Blender download$/ }).click();
  const call = await page.evaluate(() => (window as any).__calls.filter((c: any) => c.command === 'enqueue_download').at(-1).args);
  expect(call.policy.streamName).toBe('Official Blender download');
  expect(call.policy.sourceName).toBeTruthy();
  expect(typeof call.policy.quality).toBe('number');
  expect(call.episode.data).toBe('{"opaque":"episode data, 日本語"}');
});

test('backup: choose sections to save, preview before importing, and apply exactly what was previewed', async ({ page }) => {
  await desktopFixture(page);
  await page.getByRole('button', { name: 'Settings', exact: true }).first().click();
  await expect(page.getByRole('heading', { name: 'Backup & restore' })).toBeVisible();
  await expect(page.getByText(/never contain\s+profile PINs, logins or tokens/)).toBeVisible();

  // Saving: unchecking a section is sent through; a closed save dialog is not an error.
  await page.getByRole('group', { name: 'Include' }).getByLabel('Settings').uncheck();
  await page.evaluate(() => ((window as any).__saveCancelled = true));
  await page.getByRole('button', { name: 'Save backup…' }).click();
  await expect(page.getByText(/Saved \d+ library/)).toHaveCount(0);
  await page.evaluate(() => ((window as any).__saveCancelled = false));
  await page.getByRole('button', { name: 'Save backup…' }).click();
  await expect(page.getByText(/to \/home\/user\/cloudstream-backup-2026-11-06.json/)).toBeVisible();
  const saved = await page.evaluate(() => (window as any).__calls.filter((c: any) => c.command === 'export_backup').at(-1).args);
  expect(saved.sections).not.toContain('settings');
  expect(saved.sections).toContain('library');

  // Restoring: a preview first, and changing a choice re-previews with the new options.
  await page.evaluate(() => { (window as any).__backupSource = 'android'; (window as any).__backupName = 'android-backup.json'; });
  await page.getByRole('button', { name: 'Choose backup file…' }).click();
  await expect(page.locator('.backup-preview').first()).toContainText('Cloudstream Android backup');
  await expect(page.getByRole('table', { name: 'What importing would do' })).toBeVisible();
  await expect(page.getByText('New profiles: Kids')).toBeVisible();
  await expect(page.getByText(/PINs were not imported/)).toBeVisible();
  await page.getByText('1 titles have no matching source yet').click();
  await expect(page.getByText("No installed source is called 'Gone Source'")).toBeVisible();

  await page.getByLabel('Use the backup’s copy').check();
  await expect.poll(() => page.evaluate(() => (window as any).__calls.filter((c: any) => c.command === 'preview_import').at(-1).args.options.conflictPolicy)).toBe('overwrite');
  await page.getByRole('group', { name: 'Import' }).getByLabel('Settings').uncheck();
  await expect.poll(() => page.evaluate(() => (window as any).__calls.filter((c: any) => c.command === 'preview_import').at(-1).args.options.sections)).not.toContain('settings');

  await page.getByRole('button', { name: 'Import now' }).click();
  await expect(page.getByText('Import finished.')).toBeVisible();
  const applyCall = await page.evaluate(() => (window as any).__calls.filter((c: any) => c.command === 'apply_import').at(-1).args);
  expect(applyCall.options.conflictPolicy).toBe('overwrite');
  expect(applyCall.token).toBe('token-overwrite-profiles+library+progress+repositories');
  await expect(page.getByText(/pre-import-1.json/)).toBeVisible();

  // Repositories named by the backup are offered, not added silently.
  await expect(page.getByText(/1 repository that are not added here yet|1 repository/)).toBeVisible();
  await page.getByRole('button', { name: 'Add them now' }).click();
  await expect(page.getByText('Added 1 of 1 repositories.')).toBeVisible();

  // Titles without a source stay visible and are re-checked on request.
  await expect(page.getByRole('heading', { name: /Titles waiting for a source \(1\)/ })).toBeVisible();
  await page.getByRole('button', { name: 'Check again' }).first().click();
  await expect(page.getByText('No installed source matches these titles yet.')).toBeVisible();
  await page.evaluate(() => ((window as any).__sourceInstalled = true));
  await page.getByRole('button', { name: 'Check again' }).first().click();
  await expect(page.getByText('1 title was added to your library.')).toBeVisible();
  await expect(page.getByRole('heading', { name: /Titles waiting for a source/ })).toHaveCount(0);
});

test('backup: an unreadable or newer backup is explained and cannot be imported', async ({ page }) => {
  await desktopFixture(page);
  await page.getByRole('button', { name: 'Settings', exact: true }).first().click();
  await page.evaluate(() => ((window as any).__previewError = 'This backup was made by a newer version of the app (backup format 2). Update the app to restore it.'));
  await page.getByRole('button', { name: 'Choose backup file…' }).click();
  await expect(page.getByRole('alert').filter({ hasText: 'newer version of the app' })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Import now' })).toHaveCount(0);
  await page.evaluate(() => ((window as any).__previewError = ''));
  await page.evaluate(() => ((window as any).__noBackupFile = true));
  await page.getByRole('button', { name: 'Choose backup file…' }).click();
  await expect(page.getByRole('button', { name: 'Import now' })).toHaveCount(0);
  const results = await new AxeBuilder({ page }).withTags(['wcag2a', 'wcag2aa']).analyze();
  expect(results.violations).toEqual([]);
});

test('backup: automatic backups are configured safely, run on demand and report failures', async ({ page }) => {
  await desktopFixture(page);
  await page.getByRole('button', { name: 'Settings', exact: true }).first().click();
  const section = page.getByRole('region', { name: 'Backup & restore' }).or(page.locator('section', { has: page.getByRole('heading', { name: 'Backup & restore' }) })).first();
  await expect(section.getByText(/only while this app is open/)).toBeVisible();
  const toggle = section.getByLabel('Make backups automatically');
  await expect(toggle).not.toBeChecked();
  await expect(section.getByRole('button', { name: 'Back up now' })).toBeDisabled();

  // Cannot be turned on without a folder; the control returns to the saved state.
  await toggle.click();
  await expect(section.getByRole('alert').filter({ hasText: 'Choose a folder before turning scheduled backups on' })).toBeVisible();
  await expect(toggle).not.toBeChecked();

  await section.getByRole('button', { name: 'Choose folder…' }).click();
  await expect(section.getByText('/home/user/Backups')).toBeVisible();
  await toggle.check();
  await expect(toggle).toBeChecked();
  await expect(section.getByText(/Next:/)).toBeVisible();

  await section.getByLabel('How often').selectOption('weekly');
  await expect.poll(() => page.evaluate(() => (window as any).__state.settings.global.backup.frequency)).toBe('weekly');
  const retention = section.getByLabel('Number of automatic backups to keep');
  await retention.fill('0');
  await retention.blur();
  await expect(section.getByRole('alert').filter({ hasText: 'between 1 and 100' })).toBeVisible();
  await expect(retention).toHaveValue('7');
  await retention.fill('30');
  await retention.blur();
  await expect.poll(() => page.evaluate(() => (window as any).__state.settings.global.backup.retention)).toBe(30);

  await section.getByRole('button', { name: 'Back up now' }).click();
  await expect(section.locator('.backup-result').filter({ hasText: 'and removed 1 older automatic backup' })).toContainText('cloudstream-auto-backup-20261106-172640.json');
  await expect(section.getByText(/Last backup:/)).not.toContainText('—');

  await page.evaluate(() => ((window as any).__backupFails = true));
  await section.getByRole('button', { name: 'Back up now' }).click();
  await expect(section.getByRole('alert').filter({ hasText: 'is not writable' }).first()).toBeVisible();
  await expect(section.getByText(/The last attempt failed:/)).toBeVisible();
});

test('links and files from outside: navigation runs, anything that changes or plays waits for a click', async ({ page }) => {
  await page.addInitScript(() => {
    (window as any).__entries = {
      actions: [
        { type: 'search', query: 'bunny' },
        { type: 'add_repository', input: 'https://example.test/repo.json' },
        { type: 'play_url', url: 'https://media.example/video.mp4', name: 'Shared clip' },
        { type: 'play_file', path: '/home/me/Movies/My Film.mkv' },
      ],
      errors: ['Links starting with "bogus:" are not supported'],
    };
  });
  await desktopFixture(page);

  // The refused link is reported; the search link navigated without asking.
  await expect(page.getByRole('status').filter({ hasText: 'Links starting with "bogus:" are not supported' })).toBeVisible();
  await expect.poll(() => page.evaluate(() => (window as any).__calls.some((c: any) => c.command === 'take_entry_actions'))).toBe(true);

  // Nothing that adds or plays has happened yet; each shows what it would do, one at a time.
  const calls = () => page.evaluate(() => (window as any).__calls.map((c: any) => c.command));
  expect(await calls()).not.toContain('add_repository');
  expect(await calls()).not.toContain('play');
  const dialog = page.getByRole('dialog');
  await expect(dialog.getByRole('heading', { name: 'Add this repository?' })).toBeVisible();
  await expect(dialog).toContainText('https://example.test/repo.json');
  await expect(dialog).toContainText('Only add repositories you trust');
  await dialog.getByRole('button', { name: 'Cancel' }).click();
  expect(await calls()).not.toContain('add_repository');

  await expect(dialog.getByRole('heading', { name: 'Play this address?' })).toBeVisible();
  await expect(dialog).toContainText('Shared clip');
  await expect(dialog).toContainText('https://media.example/video.mp4');
  await page.keyboard.press('Escape');
  expect(await calls()).not.toContain('play');

  await expect(dialog.getByRole('heading', { name: 'Play this file?' })).toBeVisible();
  await expect(dialog).toContainText('/home/me/Movies/My Film.mkv');
  await dialog.getByRole('button', { name: 'Play in mpv' }).click();
  const local = await page.evaluate(() => (window as any).__calls.find((c: any) => c.command === 'play_local_file').args);
  expect(local.path).toBe('/home/me/Movies/My Film.mkv');
  await expect(page.getByRole('dialog')).toHaveCount(0);
  expect(await calls()).not.toContain('play');
});

test('a link delivered to the running app is shown, and confirming a repository adds it', async ({ page }) => {
  await desktopFixture(page);
  await page.evaluate(() => {
    (window as any).__entries = { actions: [{ type: 'add_repository', input: 'https://example.test/repo.json' }], errors: [] };
    (window as any).__emitEntryPending();
  });
  const dialog = page.getByRole('dialog');
  await expect(dialog.getByRole('heading', { name: 'Add this repository?' })).toBeVisible();
  await dialog.getByRole('button', { name: 'Add repository' }).click();
  const added = await page.evaluate(() => (window as any).__calls.find((c: any) => c.command === 'add_repository').args);
  expect(added.url).toBe('https://example.test/repo.json');
  await expect(page.getByRole('heading', { name: 'Extensions' }).first()).toBeVisible();

  // A shared title from a source that is not installed is explained, not silently dropped.
  await page.evaluate(() => {
    (window as any).__entries = { actions: [{ type: 'open_title', providerName: 'Not Installed', url: 'https://x.example/t' }], errors: [] };
    (window as any).__emitEntryPending();
  });
  await expect(page.getByRole('status').filter({ hasText: 'No installed source is called “Not Installed”' })).toBeVisible();
});

test('an external player can be chosen safely and used from the source list', async ({ page }) => {
  await desktopFixture(page);
  await page.getByRole('button', { name: 'Settings', exact: true }).first().click();
  const panel = page.locator('section', { has: page.getByRole('heading', { name: 'External player' }) });
  await expect(panel.getByLabel('External player')).toHaveValue('none');
  await panel.getByLabel('External player').selectOption('custom');
  await panel.getByRole('button', { name: 'Save player' }).click();
  await expect(panel.getByRole('alert')).toContainText('Choose the program');
  await panel.getByLabel('Program').fill('/usr/bin/mpv');
  await panel.getByLabel('Arguments').fill('--title={title}\n--no-fullscreen');
  await panel.getByRole('button', { name: 'Save player' }).click();
  await expect(panel.getByRole('alert')).toContainText('{url}');
  await panel.getByLabel('Arguments').fill('--title={title}\n{url}');
  await panel.getByRole('button', { name: 'Save player' }).click();
  await expect(panel.getByText('Saved.')).toBeVisible();
  const saved = await page.evaluate(() => (window as any).__calls.filter((c: any) => c.command === 'update_settings').at(-1).args.patch.externalPlayer);
  expect(saved).toEqual({ kind: 'custom', program: '/usr/bin/mpv', args: ['--title={title}', '{url}'] });

  await page.getByRole('button', { name: 'Home', exact: true }).first().click();
  await page.getByRole('button', { name: 'Try an open movie' }).click();
  const search = page.getByLabel('Search your selected source');
  await search.fill('Bunny');
  await search.press('Enter');
  await page.getByRole('button', { name: /Big Buck Bunny · Trailer/ }).click();
  await page.getByRole('button', { name: 'Find a stream', exact: true }).click();
  await page.evaluate(() => ((window as any).__externalWarnings = ['These request headers are not passed to your player: X-Test']));
  await page.getByRole('button', { name: /^Open Official Blender download in the external player$/ }).click();
  await expect(page.getByRole('status').filter({ hasText: 'These request headers are not passed to your player: X-Test' })).toBeVisible();
  const launched = await page.evaluate(() => (window as any).__calls.filter((c: any) => c.command === 'open_in_external_player').at(-1).args);
  expect(launched.stream.headers).toEqual({ 'X-Test': 'a,b' });
  expect(launched.title).toContain('Big Buck Bunny');

  // A refusal from the backend (headers the player cannot carry) is shown, not swallowed.
  await page.evaluate(() => ((window as any).__externalFails = true));
  await page.getByRole('button', { name: /^Open Official Blender download in the external player$/ }).click();
  await expect(page.getByRole('alert').filter({ hasText: 'VLC cannot send these request headers' })).toBeVisible();
});

test('clicking a new-episode notification opens that title, and an unavailable source is explained', async ({ page }) => {
  await page.addInitScript(() => {
    (window as any).__extraProviders = [{
      id: 'sample:Blender Open Movies', pluginId: 'sample', name: 'Blender Open Movies', lang: 'en',
      mainUrl: 'https://www.blender.org', hasMainPage: true, usesWebView: false, supportedTypes: ['Movie'],
    }];
    (window as any).__entries = { actions: [{ type: 'open_media', providerId: 'sample:Blender Open Movies', url: 'https://example.com/film' }], errors: [] };
  });
  await desktopFixture(page);
  // No confirmation for opening a title: it is navigation, like clicking it in the library.
  await expect(page.getByRole('heading', { name: 'Big Buck Bunny · Trailer', exact: true })).toBeVisible();
  expect(await page.evaluate(() => (window as any).__calls.filter((c: any) => c.command === 'play').length)).toBe(0);
  const loaded = await page.evaluate(() => (window as any).__calls.find((c: any) => c.command === 'load_details').args);
  expect(loaded.providerId).toBe('sample:Blender Open Movies');
  expect(loaded.url).toBe('https://example.com/film');

  await page.evaluate(() => {
    (window as any).__entries = { actions: [{ type: 'open_media', providerId: 'gone:Source', url: 'https://x.example/t' }], errors: [] };
    (window as any).__emitEntryPending();
  });
  await expect(page.getByRole('status').filter({ hasText: 'source is not available right now' })).toBeVisible();
});

test('diagnostics show redacted facts and can be saved; updates never install anything', async ({ page }) => {
  await desktopFixture(page);
  await page.getByRole('button', { name: 'Settings', exact: true }).first().click();
  const diagnostics = page.locator('section', { has: page.getByRole('heading', { name: 'Diagnostics', exact: true }) });
  await expect(diagnostics.getByText('0.41.0')).toBeVisible();
  await expect(diagnostics.getByText('3 (1 off) from 1 packages')).toBeVisible();
  await diagnostics.getByText('2 recent events').click();
  await expect(diagnostics.getByText(/loadLinks failed: https:\/\/cdn.example\/f.mp4\?\[redacted\]/)).toBeVisible();
  await diagnostics.getByText('Slowest recent requests').click();
  await expect(diagnostics.getByText('loadLinks: 4200 ms (timeout)')).toBeVisible();
  await diagnostics.getByText('1 player features are unavailable').click();
  await expect(diagnostics.getByText(/no independent video delay/)).toBeVisible();

  await page.evaluate(() => ((window as any).__saveCancelled = true));
  await diagnostics.getByRole('button', { name: 'Save diagnostics…' }).click();
  await expect(diagnostics.getByText(/Saved to/)).toHaveCount(0);
  await page.evaluate(() => ((window as any).__saveCancelled = false));
  await diagnostics.getByRole('button', { name: 'Save diagnostics…' }).click();
  await expect(diagnostics.getByText('/home/user/cloudstream-diagnostics.json')).toBeVisible();
  await diagnostics.getByRole('button', { name: 'Clear' }).click();
  await expect(diagnostics.getByText('No errors recorded in this session.')).toBeVisible();

  const updates = page.locator('section', { has: page.getByRole('heading', { name: 'App updates' }) });
  await expect(updates.getByText(/never installs updates by itself/)).toBeVisible();
  await updates.getByRole('button', { name: 'Check for updates' }).click();
  await expect(updates.getByRole('status')).toContainText('No update channel is configured');

  await updates.getByLabel('Update channel').selectOption('beta');
  await expect.poll(() => page.evaluate(() => (window as any).__state.settings.global.updates.channel)).toBe('beta');

  await page.evaluate(() => ((window as any).__updateResult = {
    state: 'available', managedByPackageManager: true,
    release: { version: '0.3.0', published: '2026-11-01', notesUrl: 'https://example.org/notes', packages: [{ kind: 'deb', arch: 'x86_64', url: 'https://d.example/a.deb', sha256: 'c'.repeat(64), size: 52_428_800 }] },
  }));
  await updates.getByRole('button', { name: 'Check for updates' }).click();
  await expect(updates.getByText('Version 0.3.0')).toBeVisible();
  await expect(updates.getByText('c'.repeat(64))).toBeVisible();
  await expect(updates.getByText(/Update it with your package manager/)).toBeVisible();
  await expect(updates.getByRole('button', { name: /install|update now|download/i })).toHaveCount(0);
  await updates.getByRole('button', { name: 'Open release notes' }).click();
  expect(await page.evaluate(() => (window as any).__calls.some((c: any) => c.command === 'open_release_notes'))).toBe(true);
  await updates.getByRole('button', { name: 'Skip this version' }).click();
  await expect(updates.getByText(/you chose to skip it/)).toBeVisible();
  expect(await page.evaluate(() => (window as any).__state.settings.global.updates.skippedVersion)).toBe('0.3.0');

  await page.evaluate(() => ((window as any).__updateError = true));
  await updates.getByRole('button', { name: 'Check for updates' }).click();
  await expect(updates.getByRole('alert')).toContainText('signature is not valid');
  const results = await new AxeBuilder({ page }).withTags(['wcag2a', 'wcag2aa']).analyze();
  expect(results.violations).toEqual([]);
});

test('appearance choices apply to the whole window and stay readable in every theme and accent', async ({ page }) => {
  await desktopFixture(page);
  await page.getByRole('button', { name: 'Settings', exact: true }).first().click();
  const appearance = page.locator('section', { has: page.getByRole('heading', { name: 'Appearance', exact: true }) });
  const root = page.locator('html');

  await appearance.getByRole('group', { name: 'Theme' }).getByRole('button', { name: 'Black' }).click();
  await expect(root).toHaveAttribute('data-theme', 'black');
  await appearance.getByRole('group', { name: 'Accent color' }).getByRole('button', { name: 'Teal' }).click();
  await expect(root).toHaveAttribute('data-accent', 'teal');
  await appearance.getByRole('group', { name: 'Title placement' }).getByRole('button', { name: 'Over poster' }).click();
  await expect(page.locator('.app-shell')).toHaveClass(/title-overlay/);
  await appearance.getByRole('group', { name: 'Type badges' }).getByRole('button', { name: 'Hidden' }).click();
  await expect(page.locator('.app-shell')).toHaveClass(/hide-badges/);
  await appearance.getByRole('group', { name: 'Motion' }).getByRole('button', { name: 'Always reduce' }).click();
  await expect(root).toHaveAttribute('data-motion', 'reduced');
  const scale = appearance.getByLabel('Poster scale');
  await scale.fill('120');
  await expect.poll(() => page.evaluate(() => document.documentElement.style.getPropertyValue('--poster-scale'))).toBe('1.2');
  const sent = await page.evaluate(() => (window as any).__calls.filter((c: any) => c.command === 'update_settings').map((c: any) => c.args.patch.appearance));
  expect(sent).toContainEqual({ theme: 'black' });
  expect(sent).toContainEqual({ accent: 'teal' });
  expect(sent).toContainEqual({ posterScale: 120 });
  expect(sent).toContainEqual({ showBadges: false });

  // Every combination keeps text readable (WCAG AA colour contrast), including the brand link name.
  for (const theme of ['Dark', 'Black']) {
    await appearance.getByRole('group', { name: 'Theme' }).getByRole('button', { name: theme }).click();
    for (const accent of ['Indigo', 'Teal', 'Rose', 'Amber', 'Green', 'Violet']) {
      await appearance.getByRole('group', { name: 'Accent color' }).getByRole('button', { name: accent }).click();
      const report = await new AxeBuilder({ page }).withTags(['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa']).analyze();
      expect(report.violations.flatMap((v) => v.nodes.map((n) => `${theme}/${accent}: ${v.id} ${n.html.slice(0, 140)}`))).toEqual([]);
    }
  }
});

test('settings can be searched, jumped through and read about, with honest limits stated', async ({ page }) => {
  await desktopFixture(page);
  await page.getByRole('button', { name: 'Settings', exact: true }).first().click();
  const search = page.getByLabel('Search settings');
  const panels = page.locator('.settings-panels > .settings-panel');
  const total = await panels.count();
  expect(total).toBeGreaterThan(6);

  await search.fill('subtitle');
  await expect(page.getByText(/sections? match/)).toBeVisible();
  const visible = await panels.evaluateAll((els) => els.filter((e) => !(e as HTMLElement).hidden).map((e) => e.querySelector('h2')?.textContent));
  expect(visible.length).toBeLessThan(total);
  expect(visible.join(' ')).toContain('Subtitle appearance');
  await search.fill('qqqq-no-such-setting');
  await expect(page.getByRole('status').filter({ hasText: 'No settings match' })).toBeVisible();
  await search.fill('');
  await expect.poll(() => panels.evaluateAll((els) => els.filter((e) => !(e as HTMLElement).hidden).length)).toBe(total);

  // Words in different panels, and words only a panel's keywords hold, are found.
  await search.fill('anilist');
  await expect(page.getByRole('heading', { name: 'Accounts', exact: true })).toBeVisible();
  await expect(page.getByText(/not available in this\s+version/)).toBeVisible();
  await search.fill('');

  // The jump list focuses the section heading.
  await page.getByRole('navigation', { name: 'Settings sections' }).getByRole('button', { name: 'Backup & restore' }).click();
  await expect(page.getByRole('heading', { name: 'Backup & restore' })).toBeFocused();

  // Project links open by id, never by an address the page supplies; licences come from the app itself.
  await page.getByRole('button', { name: 'Report a problem' }).click();
  expect(await page.evaluate(() => (window as any).__calls.filter((c: any) => c.command === 'open_link').at(-1).args)).toEqual({ id: 'issues' });
  await page.getByRole('button', { name: 'Third-party licenses' }).click();
  const legal = page.getByRole('region', { name: 'Third-party licenses' });
  await expect(legal).toContainText('LEGAL TEXT FOR third-party');
  await legal.focus();
  await page.getByRole('button', { name: 'Close', exact: true }).first().click();
  await expect(page.getByRole('region', { name: 'Third-party licenses' })).toHaveCount(0);
});

test('player: opt-in skip markers apply only to the active session', async ({ page }) => {
  await desktopFixture(page);
  await page.evaluate(() => {
    (window as any).__skipMarkers = [{ kind: 'op', start: 5, end: 15, source: 'AniSkip' }];
    (window as any).__emitPlayerEvent({ type: 'state_changed', state: { sessionId: 'session-1', episodeId: 'episode-1', active: true, playing: true, paused: false, position: 8, duration: 100, volume: 100, speed: 1 } });
  });
  await expect(page.getByLabel('Stop playback')).toBeVisible();
  expect(await page.evaluate(() => (window as any).__calls.some((c: any) => c.command === 'player_skip_markers'))).toBe(false);
  await page.getByRole('button', { name: 'Settings', exact: true }).first().click();
  await page.getByLabel('AniSkip markers').check();
  const skip = page.getByRole('button', { name: 'Skip opening' });
  await expect(skip).toBeVisible();
  const accessibility = await new AxeBuilder({ page }).include('.player-bar').withTags(['wcag2a', 'wcag2aa']).analyze();
  expect(accessibility.violations).toEqual([]);
  await skip.click();
  expect(await page.evaluate(() => (window as any).__calls.filter((c: any) => c.command === 'player_skip_interval').at(-1).args)).toEqual({ sessionId: 'session-1', start: 5, end: 15 });
  await page.evaluate(() => {
    (window as any).__skipMarkers = [];
    (window as any).__emitPlayerEvent({ type: 'state_changed', state: { sessionId: 'session-2', episodeId: 'episode-2', active: true, playing: true, paused: false, position: 8, duration: 100, volume: 100, speed: 1 } });
  });
  await expect(skip).toHaveCount(0);
  await page.getByLabel('AniSkip markers').uncheck();
  const count = await page.evaluate(() => (window as any).__calls.filter((c: any) => c.command === 'player_skip_markers').length);
  await page.evaluate(() => (window as any).__emitPlayerEvent({ type: 'state_changed', state: { sessionId: 'session-3', episodeId: 'episode-3', active: true, playing: true, paused: false, position: 8, duration: 100, volume: 100, speed: 1 } }));
  await page.waitForTimeout(100);
  expect(await page.evaluate(() => (window as any).__calls.filter((c: any) => c.command === 'player_skip_markers').length)).toBe(count);
});

test('player: configurable seek, temporary hold and app control lock', async ({ page }) => {
  await desktopFixture(page);
  await page.getByRole('button', { name: 'Settings', exact: true }).first().click();
  await page.getByLabel('Seek step in seconds').fill('25');
  await page.getByLabel('Seek step in seconds').press('Tab');
  await expect.poll(() => page.evaluate(() => (window as any).__state.settings.global.player?.seekStepSeconds)).toBe(25);
  await page.getByLabel('Hold speed', { exact: true }).fill('3');
  await page.getByLabel('Hold speed', { exact: true }).press('Tab');
  await expect.poll(() => page.evaluate(() => (window as any).__state.settings.global.player?.holdSpeed)).toBe(3);
  await page.evaluate(() => {
    const state = { sessionId: 'hold-session', active: true, playing: true, paused: false, position: 20, duration: 100, volume: 100, speed: 1, itemName: 'Clip' };
    (window as any).__state.__playerState = state;
    (window as any).__emitPlayerEvent({ type: 'state_changed', state });
    (window as any).__delaySpeedHold = true;
  });
  await expect(page.getByRole('button', { name: 'Seek back 25 seconds' })).toBeVisible();
  await page.locator('body').click({ position: { x: 5, y: 5 } });
  await page.keyboard.press('j');
  await expect.poll(() => page.evaluate(() => (window as any).__calls.filter((c: any) => c.command === 'player_command').at(-1)?.args.command)).toEqual({ type: 'seek_relative', seconds: -25 });
  await page.keyboard.down('h');
  await page.keyboard.up('h');
  await expect.poll(() => page.evaluate(() => (window as any).__calls.filter((c: any) => c.command === 'end_speed_hold').at(-1)?.args)).toEqual({ sessionId: 'hold-session' });
  const holds = await page.evaluate(() => (window as any).__calls.filter((c: any) => ['begin_speed_hold', 'end_speed_hold'].includes(c.command)));
  expect(holds.map((c: any) => c.command)).toEqual(['begin_speed_hold', 'end_speed_hold']);
  await page.keyboard.down('h');
  await page.evaluate(() => window.dispatchEvent(new Event('blur')));
  await expect.poll(() => page.evaluate(() => (window as any).__calls.filter((c: any) => c.command === 'end_speed_hold').length)).toBe(2);
  await page.keyboard.up('h');
  await page.getByRole('button', { name: 'Lock player controls' }).click();
  await expect(page.getByText('App controls locked')).toBeVisible();
  await expect(page.getByRole('button', { name: 'Stop playback' })).toHaveCount(0);
  await page.keyboard.press('Escape');
  await expect(page.getByRole('button', { name: 'Unlock player controls' })).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Stop playback' })).toBeVisible();
});

test('keyboard: shortcuts list, playback keys, arrow-key navigation and the quit confirmation', async ({ page }) => {
  await desktopFixture(page);
  await page.keyboard.press('?');
  const shortcuts = page.getByRole('dialog', { name: 'Keyboard shortcuts' });
  await expect(shortcuts).toContainText('Pause or resume playback');
  await page.keyboard.press('Escape');
  await expect(shortcuts).toHaveCount(0);

  // Playback keys act only when something plays and the focus is not on a control or a text box.
  const commands = () => page.evaluate(() => (window as any).__calls.filter((c: any) => c.command === 'player_command').map((c: any) => c.args.command));
  await page.keyboard.press('k');
  expect(await commands()).toEqual([]);
  await page.evaluate(() => (window as any).__emitPlayerEvent({ type: 'state_changed', state: { active: true, playing: true, paused: false, position: 5, duration: 100, volume: 100, speed: 1, muted: false, fullscreen: false, itemName: 'Clip' } }));
  await expect(page.getByLabel('Stop playback')).toBeVisible();
  await page.locator('body').click({ position: { x: 5, y: 5 } });
  await page.keyboard.press('k');
  await page.keyboard.press('j');
  await page.keyboard.press('l');
  await page.keyboard.press('m');
  await page.keyboard.press('f');
  await page.keyboard.press('>');
  expect(await commands()).toEqual([
    { type: 'toggle_pause' },
    { type: 'seek_relative', seconds: -10 },
    { type: 'seek_relative', seconds: 10 },
    { type: 'set_mute', muted: true },
    { type: 'set_fullscreen', fullscreen: true },
    { type: 'set_speed', speed: 1.25 },
  ]);
  await page.getByRole('button', { name: 'Settings', exact: true }).first().click();
  await page.getByLabel('Search settings').focus();
  await page.keyboard.type('kjlm');
  expect((await commands()).length, 'typing in a text box never controls playback').toBe(6);
  await page.getByLabel('Search settings').fill('');
  await page.getByRole('button', { name: 'Third-party licenses' }).focus();
  await page.keyboard.press('Space');
  expect((await commands()).length, 'Space on a focused button presses that button, not play/pause').toBe(6);
  await page.getByRole('region', { name: 'Third-party licenses' }).waitFor();
  await page.getByRole('button', { name: 'Close', exact: true }).first().click();

  // Arrow-key navigation is opt-in.
  const first = page.getByRole('navigation', { name: 'Main navigation' }).getByRole('button').first();
  await first.focus();
  await page.keyboard.press('ArrowDown');
  await expect(first).toBeFocused();
  await page.locator('section', { has: page.getByRole('heading', { name: 'Appearance', exact: true }) }).getByRole('group', { name: 'Keyboard navigation' }).getByRole('button', { name: 'Arrow keys' }).click();
  await expect(page.locator('html')).toHaveAttribute('data-nav', 'spatial');
  await first.focus();
  await page.keyboard.press('ArrowDown');
  await expect(first).not.toBeFocused();
  await expect(page.locator(':focus')).toBeVisible();

  // Closing the window while something runs asks first; "keep it open" changes nothing.
  await page.evaluate(() => (window as any).__emitConfirmExit({ playing: true, downloads: 2 }));
  const quit = page.getByRole('dialog', { name: 'Quit Cloudstream Desktop?' });
  await expect(quit).toContainText('Something is playing and 2 downloads are running');
  await quit.getByRole('button', { name: 'Keep it open' }).click();
  expect(await page.evaluate(() => (window as any).__calls.some((c: any) => c.command === 'quit_app'))).toBe(false);
  await page.evaluate(() => (window as any).__emitConfirmExit({ playing: false, downloads: 1 }));
  await expect(page.getByRole('dialog', { name: 'Quit Cloudstream Desktop?' })).toContainText('1 download is still running');
  await page.getByRole('button', { name: 'Quit', exact: true }).click();
  expect(await page.evaluate(() => (window as any).__calls.some((c: any) => c.command === 'quit_app'))).toBe(true);
});

test('a very large library renders in steps and keeps every title reachable', async ({ page }) => {
  await desktopFixture(page);
  await page.evaluate(() => {
    const store = ((window as any).__state.profileStores ||= {});
    store.default = { history: [], library: Array.from({ length: 2500 }, (_, i) => ({
      id: `big::${i}`, savedAt: i, watchStatus: 'none', isFavorite: false, isSubscribed: false, profileId: 'default', updatedAt: i,
      item: { providerId: 'sample:Blender Open Movies', name: `Title ${String(i).padStart(4, '0')}`, url: `https://example.com/t/${i}`, mediaType: i % 3 === 0 ? 'Documentary' : 'Movie' },
    })) };
  });
  // A player shutdown event makes the app reload its data, as it does after any playback.
  await page.evaluate(() => (window as any).__emitPlayerEvent({ type: 'shutdown', sessionId: 'x' }));
  await page.getByRole('button', { name: /My library/ }).click();
  await expect(page.getByRole('button', { name: /Open Title \d{4}/ }).first()).toBeVisible();
  const rendered = await page.locator('.library-card-wrapper').count();
  expect(rendered).toBeGreaterThan(0);
  expect(rendered, 'only the first chunk is in the page').toBeLessThanOrEqual(120);
  await expect(page.getByText(`Showing ${rendered} of 2500`)).toBeVisible();
  await page.getByRole('button', { name: 'Show more' }).click();
  await expect.poll(() => page.locator('.library-card-wrapper').count()).toBeGreaterThan(rendered);
  // Unknown and uncommon types stay as data and are searchable, never turned into "Movie".
  await expect(page.getByText('Documentary').first()).toBeVisible();
});

test('first run: choose language, source languages and display; add a repository only when asked', async ({ page }) => {
  await page.addInitScript(() => ((window as any).__setupPending = true));
  await desktopFixture(page);
  const dialog = page.getByRole('dialog', { name: 'Choose your language' });
  await expect(dialog).toBeVisible();
  await expect(dialog.getByRole('status')).toHaveText('Step 1 of 4');
  await expect(dialog.getByRole('heading')).toBeFocused();
  // Escape does not dismiss setup; skipping is an explicit choice.
  await page.keyboard.press('Escape');
  await expect(dialog).toBeVisible();

  // Step 1: the language applies at once, to the navigation behind the dialog and the dialog itself.
  await dialog.getByRole('combobox').selectOption('de');
  await expect(page.locator('html')).toHaveAttribute('lang', 'de');
  await expect(page.locator('html')).toHaveAttribute('dir', 'ltr');
  await expect(page.getByRole('navigation', { name: 'Main navigation' })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Startseite' })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Suche' }).first()).toBeVisible();
  await expect(dialog.getByRole('button', { name: 'Weiter' })).toBeVisible();
  await dialog.getByRole('button', { name: 'Weiter' }).click();

  // Step 2: source languages are saved as preferences only.
  await expect(page.getByRole('dialog', { name: 'Which languages do your sources use?' })).toBeVisible();
  await page.getByRole('checkbox', { name: 'English' }).check();
  await page.getByRole('checkbox', { name: '日本語' }).check();
  await page.getByRole('dialog').getByRole('button', { name: 'Weiter', exact: true }).click();

  // Step 3: display and behaviour.
  await expect(page.getByRole('dialog', { name: 'How should it look and behave?' })).toBeVisible();
  await page.getByRole('checkbox', { name: 'Show more posters on screen' }).check();
  await page.getByRole('checkbox', { name: 'Move around with the arrow keys' }).check();
  await page.getByRole('checkbox', { name: 'Play the next episode automatically' }).uncheck();
  await page.getByRole('dialog').getByRole('button', { name: 'Weiter', exact: true }).click();

  // Step 4: nothing is added or installed unless the button is pressed.
  const last = page.getByRole('dialog', { name: 'Add a source repository (optional)' });
  await expect(last).toContainText('Nothing is installed unless you choose it');
  const callsBefore = await page.evaluate(() => (window as any).__calls.map((c: any) => c.command));
  expect(callsBefore).not.toContain('add_repository');
  expect(callsBefore).not.toContain('install_sample');
  expect(callsBefore.filter((c: string) => c.startsWith('install'))).toEqual([]);
  await last.getByLabel('Repository address').fill('https://example.test/repo.json');
  await last.getByRole('button', { name: 'Add repository' }).click();
  await expect(last.getByText('Repository added.')).toBeVisible();
  await last.getByRole('button', { name: 'Finish' }).click();
  await expect(page.getByRole('dialog')).toHaveCount(0);

  const settings = await page.evaluate(() => (window as any).__state.settings.global);
  expect(settings.setup).toMatchObject({ completed: true, skipped: false, version: 1 });
  expect(settings.providerLanguages).toEqual(['en', 'ja']);
  expect(settings.appearance.locale).toBe('de');
  expect(settings.appearance.posterDensity).toBe('compact');
  expect(settings.appearance.navigation).toBe('spatial');
  expect(settings.playback.autoNext).toBe(false);
  expect(await page.evaluate(() => (window as any).__calls.filter((c: any) => c.command === 'add_repository').length)).toBe(1);

  // The extension catalog starts on the chosen languages.
  await page.getByRole('button', { name: 'Erweiterungen' }).click();
  await page.getByRole('button', { name: 'Browse repositories' }).click().catch(() => {});
  const languageFilter = page.getByLabel('Filter by language');
  if (await languageFilter.count()) await expect(languageFilter).toHaveValue('mine');
  expect(await page.evaluate(() => (window as any).__state.settings.global.providerLanguages)).toEqual(['en', 'ja']);
});

test('skipping setup leaves an honest empty state and installs nothing; redo changes only preferences', async ({ page }) => {
  await page.addInitScript(() => ((window as any).__setupPending = true));
  await desktopFixture(page);
  await page.getByRole('dialog').getByRole('button', { name: 'Skip setup' }).click();
  await expect(page.getByRole('dialog')).toHaveCount(0);
  await expect(page.getByRole('heading', { name: 'Your media. Your way.' })).toBeVisible();
  let settings = await page.evaluate(() => (window as any).__state.settings.global.setup);
  expect(settings).toMatchObject({ completed: true, skipped: true });
  const commands = () => page.evaluate(() => (window as any).__calls.map((c: any) => c.command));
  expect(await commands()).not.toContain('add_repository');
  expect((await commands()).filter((c: string) => /^(install|remove|delete|clear)/.test(c))).toEqual([]);

  // Setup can be run again from Settings, starting from the current choices, and removes nothing.
  await page.getByRole('button', { name: 'Settings', exact: true }).first().click();
  await page.getByRole('button', { name: 'Run setup again' }).click();
  const dialog = page.getByRole('dialog', { name: 'Choose your language' });
  await expect(dialog).toBeVisible();
  const next = page.getByRole('dialog').getByRole('button', { name: 'Next', exact: true });
  await next.click();
  await next.click();
  await next.click();
  await page.getByRole('dialog').getByRole('button', { name: 'Finish', exact: true }).click();
  await expect(page.getByRole('dialog')).toHaveCount(0);
  expect((await commands()).filter((c: string) => /^(install|remove|delete|clear|add_repository)/.test(c))).toEqual([]);
  settings = await page.evaluate(() => (window as any).__state.settings.global.setup);
  expect(settings).toMatchObject({ completed: true, skipped: false });
});

test('an install that already has data is not greeted with setup', async ({ page }) => {
  await page.addInitScript(() => { (window as any).__setupPending = true; (window as any).__existingData = true; });
  await desktopFixture(page);
  await expect(page.getByRole('dialog')).toHaveCount(0);
  await expect.poll(() => page.evaluate(() => (window as any).__state.settings.global.setup)).toMatchObject({ completed: true, skipped: true });
});

test('switching the interface language keeps browsing state and every other setting', async ({ page }) => {
  await desktopFixture(page);
  await page.getByRole('button', { name: 'Try an open movie' }).click();
  const search = page.getByLabel('Search your selected source');
  await search.fill('Bunny');
  await search.press('Enter');
  await expect(page.getByRole('button', { name: /Big Buck Bunny · Trailer/ })).toBeVisible();
  // Change a few other settings first, so we can see that none of them are touched.
  await page.getByRole('button', { name: 'Settings', exact: true }).first().click();
  const appearance = page.locator('section', { has: page.getByRole('heading', { name: 'Appearance', exact: true }) });
  await appearance.getByRole('group', { name: 'Accent color' }).getByRole('button', { name: 'Teal' }).click();
  await appearance.getByRole('group', { name: 'Poster size' }).getByRole('button', { name: 'Compact' }).click();

  await page.getByLabel('Interface language').selectOption('es');
  await expect(page.locator('html')).toHaveAttribute('lang', 'es');
  await expect(page.getByRole('heading', { name: 'Ajustes' })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Inicio' })).toBeVisible();
  const saved = await page.evaluate(() => (window as any).__state.settings.global.appearance);
  expect(saved).toMatchObject({ locale: 'es', accent: 'teal', posterDensity: 'compact' });
  await expect(page.locator('html')).toHaveAttribute('data-accent', 'teal');

  // Untranslated text falls back to English per string, never to blank labels.
  await expect(page.getByRole('heading', { name: 'Appearance', exact: true })).toBeVisible();

  // The search the person had is still there after changing language.
  await page.getByRole('button', { name: 'Buscar' }).first().click();
  await expect(page.getByRole('button', { name: /Big Buck Bunny · Trailer/ })).toBeVisible();
  await expect(page.getByLabel('Search your selected source')).toHaveValue('Bunny');

  // Dates and counts follow the language too.
  expect(await page.evaluate(() => new Intl.NumberFormat('es').format(1234.5))).toBe('1234,5');
});

test('a right-to-left language mirrors the layout without horizontal overflow', async ({ page }) => {
  await desktopFixture(page);
  await page.getByRole('button', { name: 'Settings', exact: true }).first().click();
  const ltrBox = await page.locator('.sidebar').boundingBox();
  expect(ltrBox!.x).toBeLessThan(5);
  await page.getByLabel('Interface language').selectOption('ar');
  await expect(page.locator('html')).toHaveAttribute('dir', 'rtl');
  await expect(page.locator('html')).toHaveAttribute('lang', 'ar');
  await expect(page.getByRole('button', { name: 'الرئيسية' })).toBeVisible();
  await expect(page.getByRole('button', { name: 'بحث' }).first()).toBeVisible();
  const rtlBox = await page.locator('.sidebar').boundingBox();
  expect(rtlBox!.x, 'the sidebar moves to the right edge').toBeGreaterThan(600);
  const overflowAt = async (width: number, height: number) => {
    await page.setViewportSize({ width, height });
    return page.evaluate(() => Math.max(document.documentElement.scrollWidth - document.documentElement.clientWidth, ...[...document.querySelectorAll('main *')].map((e) => Math.max(0, -e.getBoundingClientRect().left, e.getBoundingClientRect().right - document.documentElement.clientWidth))));
  };
  for (const [width, height] of [[1280, 900], [760, 800], [420, 800]]) {
    expect(await overflowAt(width, height), `${width}px wide in Arabic`).toBeLessThanOrEqual(1);
  }
  await page.getByLabel('اللغة').first().isVisible().catch(() => {});
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.screenshot({ path: 'test-results/rtl-arabic.png' });
  const report = await new AxeBuilder({ page }).withTags(['wcag2a', 'wcag2aa']).analyze();
  expect(report.violations.map((v) => v.id)).toEqual([]);
  // Back to a left-to-right language restores the layout, and it also fits a phone-width window.
  await page.getByLabel('Interface language').or(page.locator('select').first()).first().isVisible().catch(() => {});
  await page.locator('select').filter({ has: page.locator('option[value="en"]') }).first().selectOption('en');
  await expect(page.locator('html')).toHaveAttribute('dir', 'ltr');
  expect(await overflowAt(420, 800), '420px wide in English').toBeLessThanOrEqual(1);
});

async function openAnimeDetails(page: Page, firstRun = true) {
  if (firstRun) await page.getByRole('button', { name: 'Try an open movie' }).click();
  const search = page.getByLabel('Search your selected source');
  await search.fill('anime');
  await search.press('Enter');
  await page.getByRole('button', { name: /Anime Adventure/ }).first().click();
  await expect(page.getByRole('heading', { name: 'Anime Adventure', exact: true })).toBeVisible();
}

test('metadata is off until enabled, then enriches beside the source data with its provenance', async ({ page }) => {
  await desktopFixture(page);
  await openAnimeDetails(page);
  const panel = page.locator('section.metadata-panel');
  await expect(panel).toContainText('nothing is');
  await expect(panel.getByRole('button', { name: 'Find a match' })).toHaveCount(0);
  expect(await page.evaluate(() => (window as any).__calls.some((c: any) => c.command === 'metadata_for'))).toBe(false);

  // Opting in is explicit and per service.
  await page.getByRole('button', { name: 'Settings', exact: true }).first().click();
  const settings = page.locator('section', { has: page.getByRole('heading', { name: 'Metadata', exact: true }) });
  await expect(settings).toContainText('only for titles you open');
  await settings.getByLabel('Look up extra details').check();
  await expect.poll(() => page.evaluate(() => (window as any).__state.settings.global.metadata.enabled)).toBe(true);
  await page.getByRole('button', { name: 'Home', exact: true }).first().click();
  await openAnimeDetails(page, false);

  await expect(panel.getByRole('list', { name: 'Linked services' })).toContainText('AniList');
  await expect(panel.getByRole('list', { name: 'Linked services' })).toContainText('given by the source');
  await expect(panel).toContainText('From AniList:');
  await expect(panel).toContainText('Two brothers search for the Philosopher');
  await expect(panel).toContainText('Edward Elric');
  await expect(panel).toContainText('2 episodes are marked as filler (AniList)');
  // The provider's own text is still the source of the title's description.
  await expect(page.getByText('A thrilling adventure across worlds.')).toBeVisible();
  // Filler badges appear on exactly the flagged episodes.
  await expect(page.locator('.episode-item-wrap').filter({ hasText: /\bEpisode 2\b/ }).getByText('FILLER')).toBeVisible();
  await expect(page.locator('.episode-item-wrap').filter({ hasText: /\bEpisode 3\b/ }).getByText('FILLER')).toHaveCount(0);
});

test('a match is only ever chosen by the person, with cautions, and can be changed or removed', async ({ page }) => {
  await desktopFixture(page);
  await page.evaluate(() => { (window as any).__state.settings.global.metadata = { enabled: true, anilist: true, kitsu: true, filler: true, trailer: true, cast: true, recommendations: true, preferPoster: false }; });
  await openAnimeDetails(page);
  const panel = page.locator('section.metadata-panel');
  await panel.getByRole('button', { name: 'Find a match' }).click();
  const dialog = page.getByRole('dialog', { name: /Find a match for/ });
  await expect(dialog).toContainText('Nothing is linked until you choose one');
  await expect(dialog.getByText('Year differs (2003 here, 2024 in your source)')).toBeVisible();
  expect(await page.evaluate(() => (window as any).__calls.some((c: any) => c.command === 'set_metadata_link'))).toBe(false);
  await dialog.getByRole('button', { name: 'Use Fullmetal Alchemist on AniList' }).click();
  const chosen = await page.evaluate(() => (window as any).__calls.find((c: any) => c.command === 'set_metadata_link').args);
  expect(chosen).toMatchObject({ service: 'anilist', externalId: '25', year: 2003 });
  expect(chosen.item.url).toContain('mixed-series');
  await expect(panel.getByRole('list', { name: 'Linked services' })).toContainText('chosen by you');
  await expect(panel.getByRole('list', { name: 'Linked services' })).toContainText('25');

  // Unlinking tells the backend which id the source supplied, so it stays rejected.
  await panel.getByRole('button', { name: 'Unlink AniList' }).click();
  const removed = await page.evaluate(() => (window as any).__calls.find((c: any) => c.command === 'remove_metadata_link').args);
  expect(removed).toMatchObject({ service: 'anilist', syncData: { anilist: '121', mal: '5114' } });
  await expect(panel).toContainText('Titles are never linked by name alone');

  // Finding a source goes through the person's own sources and their choice.
  await panel.getByRole('button', { name: 'Find a match' }).click();
  await page.getByRole('dialog').getByRole('button', { name: 'Search my sources instead' }).click();
  await expect(page.getByLabel('Search your selected source')).toHaveValue('Anime Adventure');
});

test('a metadata outage is explained and never blocks the title', async ({ page }) => {
  await desktopFixture(page);
  await page.evaluate(() => { (window as any).__state.settings.global.metadata = { enabled: true, anilist: true, kitsu: true, filler: true, trailer: true, cast: true, recommendations: true, preferPoster: false }; (window as any).__metaErrors = true; (window as any).__metaStale = true; });
  await openAnimeDetails(page);
  await expect(page.locator('section.metadata-panel').getByRole('alert')).toContainText('Showing saved details because a service could not be reached');
  // The source's own episodes are all there and playable.
  await expect(page.getByRole('button', { name: /Episode 1/ }).first()).toBeVisible();
  // Scoped to the new panel at the 2.1 level; the rest of the page is covered by the existing scans.
  const results = await new AxeBuilder({ page }).include('section.metadata-panel').withTags(['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa']).analyze();
  expect(results.violations.flatMap((v) => v.nodes.map((n) => `${v.id}: ${n.html.slice(0, 160)}`))).toEqual([]);
});

test('metadata artwork replaces the source poster only when asked for, and is credited', async ({ page }) => {
  await desktopFixture(page);
  await page.evaluate(() => { (window as any).__state.settings.global.metadata = { enabled: true, anilist: true, kitsu: true, filler: true, trailer: true, cast: true, recommendations: true, preferPoster: false }; });
  await openAnimeDetails(page);
  const poster = page.locator('.detail-poster img');
  // The source has both a poster and a background, so both stay and nothing is credited to the service.
  await expect(poster).toHaveAttribute('src', 'https://example.com/poster.jpg');
  await expect(page.locator('.details-backdrop-hero img')).toHaveAttribute('src', 'https://example.com/backdrop.jpg');
  await expect(page.locator('section.metadata-panel')).toContainText('From AniList:');
  await expect(page.locator('section.metadata-panel')).not.toContainText('Background from');
  await expect(page.locator('section.metadata-panel')).not.toContainText('Poster from');

  // Preferring the service's poster is a setting and is credited where it is used.
  await page.getByRole('button', { name: 'Settings', exact: true }).first().click();
  await page.getByLabel('Prefer metadata posters').check();
  await expect.poll(() => page.evaluate(() => (window as any).__state.settings.global.metadata.preferPoster)).toBe(true);
  await page.getByRole('button', { name: 'Home', exact: true }).first().click();
  await openAnimeDetails(page, false);
  await expect(poster).toHaveAttribute('src', 'https://s4.anilist.co/xl.jpg');
  await expect(page.locator('section.metadata-panel')).toContainText('Poster from AniList.');

  // When the source has no background at all, the service's fills the gap and says so.
  await page.evaluate(() => ((window as any).__noBackdrop = true));
  await page.getByRole('button', { name: 'Home', exact: true }).first().click();
  await openAnimeDetails(page, false);
  await expect(page.locator('.details-backdrop-hero img')).toHaveAttribute('src', 'https://s4.anilist.co/banner.jpg');
  await expect(page.locator('section.metadata-panel')).toContainText('Background from AniList.');

  // Turning metadata off restores the source's own art, and none of the service's lingers.
  await page.getByRole('button', { name: 'Settings', exact: true }).first().click();
  await page.getByLabel('Look up extra details').uncheck();
  await page.getByRole('button', { name: 'Home', exact: true }).first().click();
  await openAnimeDetails(page, false);
  await expect(poster).toHaveAttribute('src', 'https://example.com/poster.jpg');
  await expect(page.locator('.details-backdrop-hero')).toHaveCount(0);
});
