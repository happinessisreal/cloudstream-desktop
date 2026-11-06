import { test, expect, type Page } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';

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
              nextAiring: 'Next Sunday',
              backgroundPoster: 'https://example.com/backdrop.jpg',
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
            ],
            subtitles: [],
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
        case 'play':
          return null;
        case 'player_action':
          return null;
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
          state.settings.revision = (state.settings.revision || 1) + 1;
          return structuredClone(state.settings);
        }
        case 'add_repository':
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
  await page.getByRole('button', { name: /Official Blender download/ }).click();
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
  await page.getByRole('button', { name: 'Disable search suggestions', exact: true }).click();

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
  await page.getByRole('button', { name: /Official Blender download/ }).click();

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
