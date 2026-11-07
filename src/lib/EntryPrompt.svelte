<script lang="ts">
  import { onMount } from 'svelte';
  import type { EntryAction, WatchEntry } from './types';

  let {
    action,
    latest = null,
    busy = false,
    onconfirm,
    oncancel,
  }: {
    action: EntryAction;
    latest?: WatchEntry | null;
    busy?: boolean;
    onconfirm: () => void;
    oncancel: () => void;
  } = $props();

  let dialog = $state<HTMLDialogElement | null>(null);
  onMount(() => dialog?.showModal());

  const heading = $derived(
    action.type === 'add_repository'
      ? 'Add this repository?'
      : action.type === 'play_file'
        ? 'Play this file?'
        : action.type === 'resume_latest'
          ? 'Resume where you left off?'
          : 'Play this address?',
  );
  const confirmLabel = $derived(
    action.type === 'add_repository' ? 'Add repository' : action.type === 'resume_latest' ? 'Resume' : 'Play in mpv',
  );
</script>

<dialog
  bind:this={dialog}
  class="app-dialog"
  aria-labelledby="entry-title"
  oncancel={(event) => {
    event.preventDefault();
    oncancel();
  }}
>
  <h2 id="entry-title">{heading}</h2>
  <p class="muted">Another app or a link asked Cloudstream Desktop to do this. Nothing has happened yet.</p>
  {#if action.type === 'add_repository'}
    <p><code class="entry-value">{action.input}</code></p>
    <p>Extensions from a repository run code on this computer. Only add repositories you trust.</p>
  {:else if action.type === 'play_url' || action.type === 'open_url'}
    {#if action.type === 'play_url' && action.name}<p><strong>{action.name}</strong></p>{/if}
    <p><code class="entry-value">{action.url}</code></p>
  {:else if action.type === 'play_file'}
    <p><code class="entry-value">{action.path}</code></p>
  {:else if action.type === 'resume_latest'}
    {#if latest}
      <p><strong>{latest.item.name}</strong>{#if latest.episode?.name} — {latest.episode.name}{/if}</p>
    {:else}
      <p>There is nothing to resume yet.</p>
    {/if}
  {/if}
  <div class="dialog-actions" style="display:flex;gap:0.5rem;justify-content:flex-end;margin-top:1rem;flex-wrap:wrap">
    <button class="button secondary" onclick={oncancel}>Cancel</button>
    <button class="button primary" disabled={busy || (action.type === 'resume_latest' && !latest)} onclick={onconfirm}>{confirmLabel}</button>
  </div>
</dialog>

<style>
  .entry-value { word-break: break-all; }
</style>
