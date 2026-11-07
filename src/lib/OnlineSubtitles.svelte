<script lang="ts">
  import { onMount } from 'svelte';
  import { call, toErrorMessage } from './api';
  import type { OnlineSubtitleResult, OnlineSubtitleSearch } from './types';

  type Selection = { status: 'added' } | { status: 'archive'; id: string; entries: { index: number; name: string; size: number }[] };

  let {
    sessionId,
    onclose,
    onadded,
    onarchive,
  }: {
    sessionId: string;
    onclose: () => void;
    onadded: () => void;
    onarchive: (archive: { id: string; entries: { index: number; name: string; size: number }[] }) => void;
  } = $props();

  let dialog = $state<HTMLDialogElement | null>(null);
  let search = $state<OnlineSubtitleSearch | null>(null);
  let searching = $state(false);
  let using = $state<number | null>(null);
  let error = $state('');

  const LANGUAGES: Record<string, string> = { en: 'English', fr: 'French', es: 'Spanish', de: 'German', it: 'Italian', pt: 'Portuguese', 'pt-br': 'Portuguese (Brazil)', ja: 'Japanese', ko: 'Korean', zh: 'Chinese', ar: 'Arabic', ru: 'Russian', nl: 'Dutch', pl: 'Polish', tr: 'Turkish' };
  const languageName = (tag: string) => LANGUAGES[tag] ?? tag;

  async function run() {
    searching = true;
    error = '';
    try {
      search = await call<OnlineSubtitleSearch>('search_online_subtitles', { sessionId, options: {} });
    } catch (e) {
      error = toErrorMessage(e, 'The search could not run');
      search = null;
    } finally {
      searching = false;
    }
  }

  onMount(() => {
    dialog?.showModal();
    void run();
  });

  function close() {
    dialog?.close();
    onclose();
  }

  async function use(result: OnlineSubtitleResult) {
    if (!search || using !== null) return;
    using = result.index;
    error = '';
    try {
      const selection = await call<Selection>('use_online_subtitle', { searchId: search.searchId, index: result.index });
      dialog?.close();
      onclose();
      if (selection.status === 'added') onadded();
      else onarchive({ id: selection.id, entries: selection.entries });
    } catch (e) {
      error = toErrorMessage(e, 'That subtitle could not be loaded');
    } finally {
      using = null;
    }
  }

  const what = $derived(
    search
      ? `${search.title}${search.year ? ` (${search.year})` : ''}${search.episode != null ? ` · ${search.season != null ? `S${search.season} ` : ''}E${search.episode}` : ''}`
      : '',
  );

  function label(result: OnlineSubtitleResult): string {
    const parts = [languageName(result.language)];
    if (result.hearingImpaired) parts.push('hearing impaired');
    if (result.forced) parts.push('forced');
    if (result.pack) parts.push(result.season != null ? `season ${result.season} pack` : 'pack');
    if (result.aiTranslated) parts.push('machine translated');
    return parts.join(' · ');
  }
</script>

<dialog bind:this={dialog} class="app-dialog online-subtitles" aria-labelledby="online-subs-title" oncancel={onclose}>
  <h2 id="online-subs-title">Subtitles online</h2>
  {#if search}<p class="muted">For {what} · {search.languages.map(languageName).join(', ')}</p>{/if}
  {#if searching}<p role="status">Searching…</p>{/if}
  {#if error}<p class="online-subs-error" role="alert">{error}</p>{/if}
  {#if search}
    <ul class="online-subs-outcomes" aria-label="Services searched">
      {#each search.outcomes as outcome (outcome.provider)}
        <li data-status={outcome.status}>
          <strong>{outcome.name}:</strong>
          {#if outcome.status === 'ok'}{outcome.count} {outcome.count === 1 ? 'result' : 'results'}
          {:else if outcome.status === 'no_results'}no results
          {:else}{outcome.message}{/if}
        </li>
      {/each}
    </ul>
    {#if search.results.length}
      <ul class="online-subs-results" aria-label="Subtitles found">
        {#each search.results as result (result.index)}
          <li>
            <div>
              <strong>{label(result)}</strong>
              <span class="muted">{result.release || 'No release name'}</span>
              <span class="muted small">
                {result.provider}{result.matchedTitle ? ` · matched “${result.matchedTitle}${result.matchedYear ? ` (${result.matchedYear})` : ''}”` : ''}{result.episode != null ? ` · S${result.season ?? '?'}E${result.episode}` : ''}{result.uploader ? ` · by ${result.uploader}` : ''}{result.downloads ? ` · ${result.downloads} downloads` : ''}
              </span>
            </div>
            <button class="button secondary small" type="button" disabled={using !== null} onclick={() => use(result)} aria-label={`Use ${label(result)} subtitle from ${result.provider}: ${result.release}`}
              >{using === result.index ? 'Loading…' : 'Use'}</button>
          </li>
        {/each}
      </ul>
    {:else if !searching}
      <p>No subtitles were found. Check the results above for services that could not search.</p>
    {/if}
  {/if}
  <div class="dialog-actions">
    <button class="button secondary" type="button" disabled={searching} onclick={run}>Search again</button>
    <button class="button primary" type="button" onclick={close}>Close</button>
  </div>
</dialog>

<style>
  .online-subtitles { max-inline-size: min(44rem, 94vw); }
  .online-subs-outcomes { list-style: none; padding: 0; margin: 0.4rem 0; display: grid; gap: 0.2rem; font-size: 0.9em; }
  .online-subs-results { list-style: none; padding: 0; margin: 0.6rem 0; display: grid; gap: 0.5rem; max-block-size: 50vh; overflow-y: auto; }
  .online-subs-results li { display: flex; justify-content: space-between; gap: 0.6rem; align-items: center; border: 1px solid var(--border, #333); border-radius: 0.5rem; padding: 0.5rem 0.7rem; }
  .online-subs-results li div { display: grid; gap: 0.15rem; min-inline-size: 0; overflow-wrap: anywhere; }
  .small { font-size: 0.85em; }
  .online-subs-error { color: var(--danger); }
</style>
