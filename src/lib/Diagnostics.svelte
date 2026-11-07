<script lang="ts">
  import { onMount } from 'svelte';
  import { call, getSettings, toErrorMessage, updateSettings } from './api';
  import { formatDateTime } from './i18n.svelte';
  import type { DiagnosticsReport, UpdateCheck } from './types';

  let report = $state<DiagnosticsReport | null>(null);
  let loadError = $state('');
  let savedTo = $state('');
  let busy = $state(false);

  let channel = $state<'stable' | 'beta'>('stable');
  let update = $state<UpdateCheck | null>(null);
  let updateError = $state('');
  let checking = $state(false);
  let notesMessage = $state('');

  async function load() {
    loadError = '';
    try {
      report = await call<DiagnosticsReport>('diagnostics_summary');
    } catch (e) {
      loadError = toErrorMessage(e, 'Diagnostics could not be gathered');
    }
  }

  onMount(async () => {
    void load();
    try {
      const settings = await getSettings();
      channel = ((settings.global as { updates?: { channel?: 'stable' | 'beta' } }).updates?.channel) ?? 'stable';
    } catch {
      // Defaults are fine.
    }
  });

  async function save() {
    busy = true;
    savedTo = '';
    try {
      const path = await call<string | null>('export_diagnostics');
      if (path) savedTo = path;
    } catch (e) {
      loadError = toErrorMessage(e, 'The diagnostics could not be saved');
    } finally {
      busy = false;
    }
  }

  async function clear() {
    await call('clear_diagnostics').catch(() => {});
    await load();
  }

  async function check() {
    checking = true;
    updateError = '';
    notesMessage = '';
    try {
      update = await call<UpdateCheck>('check_app_update');
    } catch (e) {
      update = null;
      updateError = toErrorMessage(e, 'The update check failed');
    } finally {
      checking = false;
    }
  }

  async function changeChannel(next: 'stable' | 'beta') {
    try {
      await updateSettings({ updates: { channel: next } }, 'global');
      channel = next;
      update = null;
    } catch (e) {
      updateError = toErrorMessage(e);
    }
  }

  async function skip(version: string) {
    try {
      await updateSettings({ updates: { skippedVersion: version } }, 'global');
      update = { state: 'skipped', version };
    } catch (e) {
      updateError = toErrorMessage(e);
    }
  }

  async function openNotes() {
    notesMessage = '';
    try {
      await call('open_release_notes');
    } catch (e) {
      notesMessage = toErrorMessage(e);
    }
  }

  const size = (bytes: number) => `${(bytes / (1024 * 1024)).toFixed(0)} MB`;
  const recent = $derived((report?.events ?? []).slice(-20).reverse());
  const slow = $derived([...(report?.timings ?? [])].sort((a, b) => b.durationMs - a.durationMs).slice(0, 5));
</script>

<section class="settings-panel" aria-labelledby="updates-heading">
  <div class="section-title"><h2 id="updates-heading">App updates</h2></div>
  <p class="muted">
    This app never installs updates by itself. It can tell you when a newer, signed release exists; install it with your
    package manager or by downloading it yourself and checking the checksum shown here.
  </p>
  <label class="diag-row">
    Channel
    <select class="filter-select" value={channel} aria-label="Update channel" onchange={(e) => changeChannel(e.currentTarget.value as 'stable' | 'beta')}>
      <option value="stable">Stable</option>
      <option value="beta">Beta (also shows newer stable releases)</option>
    </select>
  </label>
  <button class="button secondary" disabled={checking} onclick={check}>{checking ? 'Checking…' : 'Check for updates'}</button>
  {#if updateError}<p class="diag-error" role="alert">{updateError}</p>{/if}
  {#if update}
    <div class="diag-result" role="status">
      {#if update.state === 'not_configured'}
        <p>{update.reason}</p>
      {:else if update.state === 'up_to_date'}
        <p>You are up to date (version {update.current}).</p>
      {:else if update.state === 'skipped'}
        <p>Version {update.version} is available, but you chose to skip it.</p>
      {:else if update.state === 'no_release'}
        <p>No release has been published on the {update.channel} channel.</p>
      {:else}
        {@const release = update.release}
        <p><strong>Version {release.version}</strong>{#if release.published} · {release.published}{/if} is available.</p>
        {#if update.managedByPackageManager}
          <p class="muted">This copy was installed by your system. Update it with your package manager.</p>
        {/if}
        {#if release.packages.length}
          <ul class="diag-packages">
            {#each release.packages as pkg}
              <li><strong>{pkg.kind}</strong> ({pkg.arch}, {size(pkg.size)}) — SHA-256 <code>{pkg.sha256}</code></li>
            {/each}
          </ul>
        {:else}
          <p class="muted">No package for this computer's architecture is listed.</p>
        {/if}
        <div class="diag-row">
          <button class="button secondary" onclick={openNotes}>Open release notes</button>
          <button class="button secondary" onclick={() => skip(release.version)}>Skip this version</button>
        </div>
        {#if notesMessage}<p class="diag-error" role="alert">{notesMessage}</p>{/if}
      {/if}
    </div>
  {/if}
</section>

<section class="settings-panel" aria-labelledby="diagnostics-heading">
  <div class="section-title"><h2 id="diagnostics-heading">Diagnostics</h2></div>
  <p class="muted">
    Recent errors and how long requests took, kept in memory only. Addresses, tokens, cookies, PINs and e-mail addresses
    are removed before anything is stored or saved.
  </p>
  {#if loadError}<p class="diag-error" role="alert">{loadError}</p>{/if}
  {#if report}
    <dl class="diag-facts">
      <div><dt>App</dt><dd>{report.appVersion} · {report.os} {report.arch}</dd></div>
      <div><dt>Player (mpv)</dt><dd>{report.mpvVersion ?? 'not found'}</dd></div>
      <div><dt>Extension worker</dt><dd>{report.workerAvailable ? 'present' : 'missing'} · Java {report.javaAvailable ? 'found' : 'not found'}</dd></div>
      <div><dt>Sources</dt><dd>{report.providers} ({report.disabledProviders} off) from {report.packages.length} packages</dd></div>
    </dl>
    {#if report.playerUnsupported.length}
      <details>
        <summary>{report.playerUnsupported.length} player features are unavailable</summary>
        <ul>{#each report.playerUnsupported as item}<li>{item}</li>{/each}</ul>
      </details>
    {/if}
    {#if recent.length}
      <details>
        <summary>{recent.length} recent events</summary>
        <ul class="diag-events">
          {#each recent as event}<li><time>{formatDateTime(event.atMs)}</time> <strong>{event.category}</strong> {event.message}</li>{/each}
        </ul>
      </details>
    {:else}
      <p class="muted">No errors recorded in this session.</p>
    {/if}
    {#if slow.length}
      <details>
        <summary>Slowest recent requests</summary>
        <ul>{#each slow as t}<li>{t.method}: {t.durationMs} ms ({t.outcome})</li>{/each}</ul>
      </details>
    {/if}
  {/if}
  <div class="diag-row">
    <button class="button secondary" disabled={busy} onclick={save}>Save diagnostics…</button>
    <button class="button secondary" onclick={clear}>Clear</button>
    <button class="button secondary" onclick={load}>Refresh</button>
  </div>
  {#if savedTo}<p role="status">Saved to <code>{savedTo}</code>. Look it over before sharing it.</p>{/if}
</section>

<style>
  .diag-row { display: flex; gap: 0.7rem; align-items: center; flex-wrap: wrap; }
  .diag-error { color: var(--danger); margin: 0; }
  .diag-result { background: var(--surface); border: 1px solid var(--border); border-radius: 12px; padding: 0.7rem 1rem; }
  .diag-packages { margin: 0.3rem 0; padding-left: 1.1rem; }
  .diag-facts { display: grid; gap: 0.3rem; margin: 0; }
  .diag-facts div { display: flex; gap: 0.8rem; flex-wrap: wrap; }
  .diag-facts dt { color: var(--muted); min-width: 9rem; }
  .diag-facts dd { margin: 0; }
  .diag-events { list-style: none; padding: 0; margin: 0.3rem 0; display: grid; gap: 0.25rem; word-break: break-word; }
  code { word-break: break-all; }
</style>
