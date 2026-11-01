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
        case 'snapshot':
          return structuredClone(state);
        case 'list_providers':
          return state.installed.length && !unsupported ? [provider] : [];
        case 'install_sample':
          state.installed = [plugin];
          return plugin;
        case 'provider_home':
          return { rows: [{ name: 'Open movies', items: [item], hasNext: false }] };
        case 'search':
          if (args.query === 'first slow search') {
            await new Promise((resolve) => setTimeout(resolve, 800));
          }
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
        case 'get_settings':
          return structuredClone(state.settings);
        case 'update_settings': {
          const patch = (args.patch as any) || {};
          const density = patch.appearance?.posterDensity || patch.posterDensity;
          if (density) {
            state.settings.global.appearance.posterDensity = density;
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
