<script lang="ts">
  import { onMount } from 'svelte';
  import { call, getSettings, toErrorMessage, updateSettings } from './api';
  import type { DetectedPlayer, ExternalPlayerKind, ExternalPlayerSettings } from './types';

  let { onchange }: { onchange?: (settings: ExternalPlayerSettings) => void } = $props();

  let detected = $state<DetectedPlayer[]>([]);
  let kind = $state<ExternalPlayerKind>('none');
  let program = $state('');
  let argsText = $state('{url}');
  let error = $state('');
  let saved = $state(false);
  let busy = $state(false);

  function apply(value?: ExternalPlayerSettings) {
    kind = value?.kind ?? 'none';
    program = value?.program ?? '';
    argsText = (value?.args?.length ? value.args : ['{url}']).join('\n');
  }

  onMount(async () => {
    try {
      detected = await call<DetectedPlayer[]>('list_external_players');
      const settings = await getSettings();
      apply((settings.global as { externalPlayer?: ExternalPlayerSettings }).externalPlayer);
    } catch (e) {
      error = toErrorMessage(e);
    }
  });

  async function save() {
    busy = true;
    error = '';
    saved = false;
    // One argument per line: each line is exactly one argument, never split or shell-expanded.
    const args = argsText.split('\n').map((line) => line.replace(/\r$/, '')).filter((line) => line.length > 0);
    const patch: ExternalPlayerSettings =
      kind === 'custom'
        ? { kind, program: program.trim(), args }
        : kind === 'vlc'
          ? { kind, program: detected.find((d) => d.kind === 'vlc')?.program ?? 'vlc', args: [] }
          : { kind: 'none', program: null, args: [] };
    try {
      const settings = await updateSettings({ externalPlayer: patch }, 'global');
      const next = (settings.global as { externalPlayer?: ExternalPlayerSettings }).externalPlayer;
      apply(next);
      saved = true;
      if (next) onchange?.(next);
    } catch (e) {
      error = toErrorMessage(e, 'That player setting could not be saved');
    } finally {
      busy = false;
    }
  }
</script>

<section class="settings-panel" aria-labelledby="external-player-heading">
  <div class="section-title"><h2 id="external-player-heading">External player</h2></div>
  <p class="muted">
    Mpv plays everything by default. You can also send a stream to another player from the source list. Players
    start without a shell, so titles and addresses are never run as commands.
  </p>
  <label class="ext-row">
    Open streams in
    <select class="filter-select" bind:value={kind} aria-label="External player">
      <option value="none">Only the built-in player</option>
      <option value="vlc" disabled={!detected.some((d) => d.kind === 'vlc')}>
        VLC{detected.some((d) => d.kind === 'vlc') ? '' : ' (not found)'}
      </option>
      <option value="custom">A command I choose</option>
    </select>
  </label>
  {#if kind === 'vlc'}
    <p class="muted">VLC can receive the referrer, user agent, start position and public subtitles. Sources that need other headers are refused instead of failing in VLC.</p>
  {/if}
  {#if kind === 'custom'}
    <label class="ext-col">
      Program (name on your PATH, or a full path)
      <input type="text" bind:value={program} placeholder="/usr/bin/mpv" aria-label="Program" />
    </label>
    <label class="ext-col">
      Arguments, one per line
      <textarea rows="4" bind:value={argsText} aria-label="Arguments"></textarea>
    </label>
    <p class="muted">
      Use {'{url}'}, {'{title}'}, {'{referer}'}, {'{user_agent}'}, {'{start}'} and {'{subtitle}'}. A line holding only a
      placeholder with no value is skipped.
    </p>
  {/if}
  <div class="ext-row">
    <button class="button secondary" disabled={busy} onclick={save}>Save player</button>
    {#if saved}<span role="status">Saved.</span>{/if}
  </div>
  {#if error}<p class="ext-error" role="alert">{error}</p>{/if}
</section>

<style>
  .ext-row { display: flex; gap: 0.8rem; align-items: center; flex-wrap: wrap; }
  .ext-col { display: grid; gap: 0.3rem; }
  .ext-col input, .ext-col textarea { font: inherit; padding: 0.4rem 0.6rem; border-radius: 8px; border: 1px solid var(--border); background: var(--surface); color: inherit; }
  .ext-error { color: var(--danger); margin: 0; }
</style>
