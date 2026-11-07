<script lang="ts">
  import { onMount } from 'svelte';
  import { getSettings, toErrorMessage, updateSettings } from './api';
  import type { MetadataSettings } from './types';

  let { onchange }: { onchange?: (settings: MetadataSettings) => void } = $props();

  let settings = $state<MetadataSettings>({ enabled: false, anilist: true, kitsu: true, filler: true, trailer: true, cast: true, recommendations: true, preferPoster: false });
  let error = $state('');

  onMount(async () => {
    try {
      const loaded = (await getSettings()).global as { metadata?: MetadataSettings };
      if (loaded.metadata) settings = { ...settings, ...loaded.metadata };
    } catch (e) {
      error = toErrorMessage(e);
    }
  });

  async function save(patch: Partial<MetadataSettings>) {
    error = '';
    try {
      const updated = (await updateSettings({ metadata: patch }, 'global')).global as { metadata?: MetadataSettings };
      if (updated.metadata) settings = { ...settings, ...updated.metadata };
      onchange?.(settings);
    } catch (e) {
      error = toErrorMessage(e, 'That setting could not be saved');
    }
  }

  const rows: { key: keyof MetadataSettings; label: string; hint: string }[] = [
    { key: 'anilist', label: 'AniList', hint: 'Search and details from anilist.co.' },
    { key: 'kitsu', label: 'Kitsu', hint: 'Search and details from kitsu.io.' },
    { key: 'filler', label: 'Filler episodes', hint: 'Marks filler episodes, using Jikan (needs a MyAnimeList id from the source).' },
    { key: 'cast', label: 'Cast', hint: 'Show characters when the source has none.' },
    { key: 'recommendations', label: 'Similar titles', hint: 'Show recommendations when the source has none.' },
    { key: 'trailer', label: 'Trailers', hint: 'Show a trailer link when the source has none.' },
    { key: 'preferPoster', label: 'Prefer metadata posters', hint: 'Use the service’s poster instead of the source’s. Off keeps the source’s art.' },
  ];
</script>

<section class="settings-panel" data-keywords="metadata anilist kitsu jikan tmdb filler cast recommendations posters trailers privacy" aria-labelledby="metadata-settings-heading">
  <div class="section-title"><h2 id="metadata-settings-heading">Metadata</h2></div>
  <p class="muted">
    Optional extra details from public services. When on, a title’s name or id is sent to the services you enable below,
    only for titles you open. The source’s own details and episodes are never replaced.
  </p>
  <label class="meta-set-row"><input type="checkbox" checked={settings.enabled} onchange={(e) => save({ enabled: e.currentTarget.checked })} />Look up extra details</label>
  {#each rows as row}
    <label class="meta-set-row" class:dim={!settings.enabled}>
      <input type="checkbox" disabled={!settings.enabled} checked={Boolean(settings[row.key])} onchange={(e) => save({ [row.key]: e.currentTarget.checked })} />
      <span><strong>{row.label}</strong> <span class="muted">{row.hint}</span></span>
    </label>
  {/each}
  {#if error}<p class="meta-set-error" role="alert">{error}</p>{/if}
</section>

<style>
  .meta-set-row { display: flex; gap: 0.6rem; align-items: flex-start; }
  .meta-set-row.dim { opacity: 0.6; }
  .meta-set-error { color: var(--danger); margin: 0; }
</style>
