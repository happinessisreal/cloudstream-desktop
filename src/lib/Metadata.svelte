<script lang="ts">
  import { untrack } from 'svelte';
  import { call, toErrorMessage } from './api';
  import { formatDateTime, i18n } from './i18n.svelte';
  import type { Details, MediaItem, MetadataCandidate, MetadataResult } from './types';

  let {
    details,
    enabled,
    onfiller,
    onart,
    onfindsource,
  }: {
    details: Details;
    enabled: boolean;
    onfiller: (ids: string[]) => void;
    /** Artwork that came from a metadata service, only when it is not the source's own. */
    onart: (art: { posterUrl?: string; backdropUrl?: string }) => void;
    onfindsource: (title: string) => void;
  } = $props();

  const SERVICE_NAMES: Record<string, string> = { anilist: 'AniList', mal: 'MyAnimeList', kitsu: 'Kitsu', imdb: 'IMDb', tmdb: 'TMDB', tvdb: 'TVDB' };
  const label = (service: string) => SERVICE_NAMES[service] ?? service;

  let result = $state<MetadataResult | null>(null);
  let loading = $state(false);
  let error = $state('');
  let searching = $state(false);
  let query = $state('');
  let candidates = $state<MetadataCandidate[] | null>(null);
  let problems = $state<string[]>([]);
  let searchError = $state('');
  let dialog = $state<HTMLDialogElement | null>(null);
  let message = $state('');

  const item = $derived(details.item as MediaItem);
  const identity = $derived(`${item.providerId}\u0000${item.url}`);

  async function load() {
    // A different title never shows the previous title's artwork or flags while this one loads.
    onfiller([]);
    onart({});
    if (!enabled) {
      result = null;
      return;
    }
    loading = true;
    error = '';
    try {
      result = await call<MetadataResult>('metadata_for', { details, locale: i18n.locale });
      onfiller(result.view.fillerEpisodeIds);
      const view = result.view;
      onart({
        posterUrl: view.posterUrl && view.provenance.poster && view.provenance.poster !== 'provider' ? view.posterUrl : undefined,
        backdropUrl: view.backdropUrl && view.provenance.backdrop && view.provenance.backdrop !== 'provider' ? view.backdropUrl : undefined,
      });
    } catch (e) {
      error = toErrorMessage(e, 'Metadata could not be loaded');
      result = null;
    } finally {
      loading = false;
    }
  }

  // Reload when a different title opens or the setting changes; the title's own data never waits on it.
  $effect(() => {
    identity;
    enabled;
    untrack(() => void load());
  });

  async function openSearch() {
    query = item.name;
    candidates = null;
    searchError = '';
    problems = [];
    dialog?.showModal();
    await runSearch();
  }

  async function runSearch() {
    searching = true;
    searchError = '';
    try {
      const found = await call<{ candidates: MetadataCandidate[]; problems: string[] }>('metadata_search', { title: query, year: item.year ?? null });
      candidates = found.candidates;
      problems = found.problems;
    } catch (e) {
      searchError = toErrorMessage(e, 'The search failed');
      candidates = null;
    } finally {
      searching = false;
    }
  }

  async function choose(candidate: MetadataCandidate) {
    try {
      await call('set_metadata_link', { item, service: candidate.service, externalId: candidate.externalId, label: candidate.title, year: candidate.year ?? null });
      dialog?.close();
      message = `Linked to ${label(candidate.service)}: ${candidate.title}.`;
      await load();
    } catch (e) {
      searchError = toErrorMessage(e);
    }
  }

  async function unlink(service: string) {
    try {
      await call('remove_metadata_link', { item, service, syncData: details.syncData ?? {} });
      message = `Unlinked ${label(service)}.`;
      await load();
    } catch (e) {
      error = toErrorMessage(e);
    }
  }

  const sourceNote = (source: 'provider' | 'user') => (source === 'provider' ? 'given by the source' : 'chosen by you');
  const from = (field: string) => result?.view.provenance[field];
</script>

<section class="section metadata-panel" aria-labelledby="metadata-heading">
  <div class="section-title">
    <h2 id="metadata-heading">Metadata</h2>
    {#if enabled}<button class="button secondary small" onclick={openSearch}>Find a match</button>{/if}
  </div>

  {#if !enabled}
    <p class="muted">
      Extra details (cast, recommendations, filler episodes) from AniList and Kitsu are off. Turn them on in Settings; nothing is
      sent to those services until you do.
    </p>
  {:else}
    {#if loading}<p class="muted" role="status">Loading metadata…</p>{/if}
    {#if error}<p class="meta-error" role="alert">{error}</p>{/if}
    {#if message}<p role="status">{message}</p>{/if}
    {#if result}
      {#if result.links.length}
        <ul class="meta-links" aria-label="Linked services">
          {#each result.links as link}
            <li>
              <strong>{label(link.service)}</strong> {link.externalId}
              <span class="muted">({sourceNote(link.source)}{link.label ? `: ${link.label}` : ''}{link.year ? `, ${link.year}` : ''})</span>
              <button class="button secondary small" aria-label={`Unlink ${label(link.service)}`} onclick={() => unlink(link.service)}>Unlink</button>
            </li>
          {/each}
        </ul>
      {:else}
        <p class="muted">No service is linked to this title. Use Find a match to choose one. Titles are never linked by name alone.</p>
      {/if}
      {#if result.errors.length}
        <p class="meta-error" role="alert">
          {result.stale ? 'Showing saved details because a service could not be reached.' : 'Some details could not be loaded.'}
          {result.errors.join(' ')}
        </p>
      {/if}
      {#if result.view.synopsis && from('synopsis') !== 'provider'}
        <p><span class="muted">From {label(from('synopsis') ?? '')}:</span> {result.view.synopsis}</p>
      {/if}
      {#if result.view.genres.length}<p><span class="muted">Genres ({label(from('genres') ?? '')}):</span> {result.view.genres.join(', ')}</p>{/if}
      {#if result.view.score != null && from('score')}<p><span class="muted">Score ({label(from('score') ?? '')}):</span> {Math.round(result.view.score)} / 100</p>{/if}
      {#if result.view.nextAiringEpisode}
        <p><span class="muted">Next episode ({label(from('airing') ?? '')}):</span> {result.view.nextAiringEpisode}{result.view.nextAiringAtMs ? ` · ${formatDateTime(result.view.nextAiringAtMs)}` : ''}</p>
      {/if}
      {#if result.view.cast.length}
        <h3>Cast <span class="muted">({label(from('cast') ?? '')})</span></h3>
        <ul class="meta-cast">{#each result.view.cast as person}<li>{person.name}{#if person.role} <span class="muted">· {person.role}</span>{/if}</li>{/each}</ul>
      {/if}
      {#if result.view.recommendations.length}
        <h3>Similar titles <span class="muted">({label(from('recommendations') ?? '')})</span></h3>
        <ul class="meta-recs">
          {#each result.view.recommendations as rec}
            <li><span>{rec.title}</span> <button class="button secondary small" onclick={() => onfindsource(rec.title)}>Search my sources</button></li>
          {/each}
        </ul>
      {/if}
      {#if result.view.posterUrl && from('poster') && from('poster') !== 'provider'}
        <p class="muted">Poster from {label(from('poster') ?? '')}.</p>
      {/if}
      {#if result.view.backdropUrl && from('backdrop') && from('backdrop') !== 'provider'}
        <p class="muted">Background from {label(from('backdrop') ?? '')}.</p>
      {/if}
      {#if result.view.fillerEpisodeIds.length}
        <p class="muted">{result.view.fillerEpisodeIds.length} episodes are marked as filler ({label(from('filler') ?? '')}).</p>
      {/if}
      {#each result.view.notes as note}<p class="muted">{note}</p>{/each}
    {/if}
  {/if}

  <dialog bind:this={dialog} class="app-dialog" aria-labelledby="meta-search-title">
    <h2 id="meta-search-title">Find a match for “{item.name}”</h2>
    <p class="muted">These are suggestions. Nothing is linked until you choose one.</p>
    <form
      class="meta-search"
      onsubmit={(event) => {
        event.preventDefault();
        void runSearch();
      }}
    >
      <label class="visually-hidden" for="meta-query">Title to search for</label>
      <input id="meta-query" type="search" bind:value={query} />
      <button class="button secondary" type="submit" disabled={searching}>Search</button>
    </form>
    {#if searching}<p role="status">Searching…</p>{/if}
    {#if searchError}<p class="meta-error" role="alert">{searchError}</p>{/if}
    {#each problems as problem}<p class="muted">{problem}</p>{/each}
    {#if candidates}
      {#if candidates.length === 0}<p>No results.</p>{/if}
      <ul class="meta-candidates">
        {#each candidates as candidate}
          <li>
            <div>
              <strong>{candidate.title}</strong>
              <span class="muted">{label(candidate.service)} · {candidate.year ?? 'year unknown'}{candidate.format ? ` · ${candidate.format}` : ''}</span>
              {#each candidate.warnings as warning}<p class="meta-warning">{warning}</p>{/each}
            </div>
            <button class="button secondary small" aria-label={`Use ${candidate.title} on ${label(candidate.service)}`} onclick={() => choose(candidate)}>Use this</button>
          </li>
        {/each}
      </ul>
    {/if}
    <div class="meta-actions">
      <button class="button secondary" onclick={() => { dialog?.close(); onfindsource(item.name); }}>Search my sources instead</button>
      <button class="button primary" onclick={() => dialog?.close()}>Close</button>
    </div>
  </dialog>
</section>

<style>
  .metadata-panel { display: grid; gap: 0.5rem; }
  .meta-links, .meta-cast, .meta-recs, .meta-candidates { list-style: none; margin: 0; padding: 0; display: grid; gap: 0.4rem; }
  .meta-links li, .meta-recs li, .meta-candidates li { display: flex; gap: 0.6rem; align-items: center; justify-content: space-between; flex-wrap: wrap; }
  .meta-cast { grid-template-columns: repeat(auto-fill, minmax(12rem, 1fr)); }
  .meta-error { color: var(--danger); margin: 0; }
  .meta-warning { margin: 0.2rem 0 0; color: #f0caa3; }
  .meta-search { display: flex; gap: 0.5rem; margin: 0.6rem 0; }
  .meta-search input { flex: 1; padding: 0.5rem 0.7rem; border-radius: 8px; border: 1px solid var(--border); background: var(--surface); color: inherit; font: inherit; }
  .meta-actions { display: flex; gap: 0.5rem; justify-content: flex-end; margin-top: 1rem; flex-wrap: wrap; }
  .meta-candidates { max-height: 45vh; overflow: auto; }
</style>
