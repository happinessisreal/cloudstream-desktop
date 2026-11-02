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
        case 'provider_home':
          return { rows: [{ name: 'Open movies', items: [item], hasNext: false }] };
        case 'search':
          if (args.providerId === 'failing:Error Source') {
            throw new Error('Connection refused by provider host');
          }
          if (args.query === 'first slow search') {
            await new Promise((resolve) => setTimeout(resolve, 800));
          }
          return {
            items: String(args.query).toLowerCase().includes('bunny') ? [item] : [],
            hasNext: args.page === 1 && String(args.query).toLowerCase().includes('bunny'),
          };
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
        case 'quick_search':
          return {
            items: String(args.query).toLowerCase().includes('bunny') ? [item] : [],
            hasNext: false,
          };
        case 'load_details':
          return {
            item,
            plot: 'An open movie from the Blender Foundation.',
            tags: ['Animation'],
            episodes: [episode],
            duration: 1,
          };
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
        case 'bookmark':
          state.library.push({ id: 'saved', item: args.item, savedAt: 1 });
          return state.library[0];
        case 'remove_bookmark':
          state.library = [];
          return null;
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
  const updateCall = calls.find((c: any) => c.command === 'update_settings');
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
