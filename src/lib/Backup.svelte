<script lang="ts">
  import { onMount } from 'svelte';
  import { Download, FolderOpen, RefreshCw, Upload } from 'lucide-svelte';
  import { call, getSettings, toErrorMessage, updateSettings } from './api';
  import { formatDateTime } from './i18n.svelte';
  import type {
    BackupExportResult,
    BackupFrequency,
    BackupRun,
    BackupSettings,
    BackupStatus,
    ConflictPolicy,
    ImportOptions,
    ImportPreview,
    ImportReport,
    UnresolvedTitle,
  } from './types';

  let { profileId, onapplied }: { profileId: string; onapplied?: () => void } = $props();

  const SECTION_LABELS: Record<string, string> = {
    profiles: 'Profiles',
    library: 'Library',
    unresolved: 'Titles waiting for a source',
    history: 'Watch history',
    progress: 'Watch progress',
    settings: 'Settings',
    repositories: 'Repository list',
    provider_preferences: 'Source preferences',
    provider_clones: 'Source copies',
  };
  const EXPORT_SECTIONS = ['profiles', 'library', 'history', 'progress', 'settings', 'repositories', 'provider_preferences'];
  const label = (section: string) => SECTION_LABELS[section] ?? section;

  let exportChoice = $state<Record<string, boolean>>(Object.fromEntries(EXPORT_SECTIONS.map((s) => [s, true])));
  let exporting = $state(false);
  let exported = $state<BackupExportResult | null>(null);

  let importId = $state<string | null>(null);
  let fileName = $state('');
  let policy = $state<ConflictPolicy>('skip_existing');
  let chosenSections = $state<Record<string, boolean>>({});
  let preview = $state<ImportPreview | null>(null);
  let working = $state(false);
  let applied = $state<ImportReport | null>(null);
  let error = $state('');
  let addingRepositories = $state(false);
  let repositoryMessage = $state('');
  let waiting = $state<UnresolvedTitle[]>([]);
  let recheckMessage = $state('');

  const selectedExport = $derived(EXPORT_SECTIONS.filter((s) => exportChoice[s]));
  const selectedImport = $derived(Object.keys(chosenSections).filter((s) => chosenSections[s]));

  function options(): ImportOptions {
    return {
      sections: preview ? selectedImport : null,
      conflictPolicy: policy,
    };
  }

  async function loadWaiting() {
    try {
      waiting = await call<UnresolvedTitle[]>('list_unresolved_imports', { profileId });
    } catch {
      waiting = [];
    }
  }
  // ---- Automatic backups ----
  let schedule = $state<BackupSettings>({ enabled: false, frequency: 'daily', retention: 7, directory: null });
  let scheduleStatus = $state<BackupStatus | null>(null);
  let scheduleError = $state('');
  let scheduleBusy = $state(false);
  let lastRun = $state<BackupRun | null>(null);

  function when(ms?: number | null) {
    return ms ? formatDateTime(ms) : '—';
  }

  async function loadSchedule() {
    try {
      const settings = await getSettings();
      const saved = (settings.global as { backup?: BackupSettings }).backup;
      if (saved) schedule = { ...saved };
      scheduleStatus = await call<BackupStatus>('backup_status');
    } catch (e) {
      scheduleError = toErrorMessage(e, 'Backup settings could not be loaded');
    }
  }

  async function saveSchedule(patch: Partial<BackupSettings>): Promise<boolean> {
    scheduleError = '';
    scheduleBusy = true;
    try {
      const settings = await updateSettings({ backup: patch }, 'global');
      const saved = (settings.global as { backup?: BackupSettings }).backup;
      if (saved) schedule = { ...schedule, ...saved };
      scheduleStatus = await call<BackupStatus>('backup_status');
      return true;
    } catch (e) {
      // The saved settings are unchanged; show the real values again.
      scheduleError = toErrorMessage(e, 'That backup setting could not be saved');
      await loadSchedule();
      return false;
    } finally {
      scheduleBusy = false;
    }
  }

  async function chooseBackupFolder() {
    const folder = await call<string | null>('choose_folder', { title: 'Choose a folder for automatic backups' }).catch(() => null);
    if (folder) await saveSchedule({ directory: folder });
  }

  async function backUpNow() {
    scheduleError = '';
    scheduleBusy = true;
    lastRun = null;
    try {
      lastRun = await call<BackupRun>('run_backup_now');
    } catch (e) {
      scheduleError = toErrorMessage(e, 'The backup failed');
    } finally {
      scheduleStatus = await call<BackupStatus>('backup_status').catch(() => scheduleStatus);
      scheduleBusy = false;
    }
  }

  onMount(() => {
    void loadWaiting();
    void loadSchedule();
  });
  $effect(() => {
    profileId;
    void loadWaiting();
  });

  async function exportNow() {
    exporting = true;
    error = '';
    exported = null;
    try {
      exported = await call<BackupExportResult | null>('export_backup', { sections: selectedExport, profileIds: null });
    } catch (e) {
      error = toErrorMessage(e, 'The backup could not be saved');
    } finally {
      exporting = false;
    }
  }

  async function runPreview(keepChoices = true) {
    if (!importId) return;
    working = true;
    error = '';
    try {
      const result = await call<ImportPreview>('preview_import', { importId, options: options() });
      if (!keepChoices || !Object.keys(chosenSections).length) {
        chosenSections = Object.fromEntries(result.availableSections.map((s) => [s, true]));
      }
      preview = result;
    } catch (e) {
      preview = null;
      error = toErrorMessage(e, 'This backup could not be read');
    } finally {
      working = false;
    }
  }

  async function chooseFile() {
    error = '';
    applied = null;
    repositoryMessage = '';
    try {
      const picked = await call<{ importId: string; name: string } | null>('choose_backup_file');
      if (!picked) return;
      importId = picked.importId;
      fileName = picked.name;
      preview = null;
      chosenSections = {};
      await runPreview(false);
    } catch (e) {
      error = toErrorMessage(e, 'That file could not be opened');
    }
  }

  async function apply() {
    if (!importId || !preview) return;
    working = true;
    error = '';
    try {
      applied = await call<ImportReport>('apply_import', { importId, options: options(), token: preview.token });
      preview = null;
      importId = null;
      await loadWaiting();
      onapplied?.();
    } catch (e) {
      error = toErrorMessage(e, 'The import failed. Nothing was changed.');
    } finally {
      working = false;
    }
  }

  async function cancelImport() {
    try {
      await call('discard_import');
    } catch {
      // Nothing is held if this fails.
    }
    importId = null;
    preview = null;
    chosenSections = {};
  }

  async function addRepositories() {
    if (!applied) return;
    addingRepositories = true;
    repositoryMessage = '';
    let added = 0;
    const failed: string[] = [];
    for (const repository of applied.repositoriesToAdd) {
      try {
        await call('add_repository', { url: repository.url });
        added += 1;
      } catch (e) {
        failed.push(`${repository.name || repository.url}: ${toErrorMessage(e)}`);
      }
    }
    repositoryMessage = `Added ${added} of ${applied.repositoriesToAdd.length} repositories.${failed.length ? ` ${failed.join(' · ')}` : ''}`;
    if (!failed.length) applied = { ...applied, repositoriesToAdd: [] };
    addingRepositories = false;
    onapplied?.();
  }

  async function recheck() {
    recheckMessage = '';
    try {
      const count = await call<number>('resolve_unresolved_imports', { profileId });
      recheckMessage = count > 0 ? `${count} ${count === 1 ? 'title was' : 'titles were'} added to your library.` : 'No installed source matches these titles yet.';
      await loadWaiting();
      onapplied?.();
    } catch (e) {
      recheckMessage = toErrorMessage(e, 'Could not check the installed sources');
    }
  }
</script>

<section class="settings-panel" aria-labelledby="backup-heading">
  <div class="section-title"><h2 id="backup-heading">Backup &amp; restore</h2></div>
  <p class="muted">
    Backups hold your profiles, library, history, progress, settings and repository list. They never contain
    profile PINs, logins or tokens, installed extensions or downloaded files.
  </p>

  <div class="backup-block" aria-labelledby="auto-backup-heading">
    <h3 id="auto-backup-heading">Automatic backups</h3>
    <p class="muted">
      Backups are made only while this app is open. If one came due while the app was closed, it is made soon after you
      start the app.
    </p>
    <label class="backup-row">
      <input
        type="checkbox"
        checked={schedule.enabled}
        disabled={scheduleBusy}
        onchange={async (e) => {
          const box = e.currentTarget;
          // A rejected change must not leave the box showing something that was not saved.
          if (!(await saveSchedule({ enabled: box.checked }))) box.checked = schedule.enabled;
        }}
      />Make backups automatically
    </label>
    <div class="backup-row">
      <button class="button secondary" disabled={scheduleBusy} onclick={chooseBackupFolder}>
        <FolderOpen size={15} />{schedule.directory ? 'Change folder…' : 'Choose folder…'}
      </button>
      <span class="muted" title={schedule.directory ?? ''}>{schedule.directory ?? 'No folder chosen'}</span>
    </div>
    <div class="backup-row">
      <label>
        How often
        <select
          class="filter-select"
          value={schedule.frequency}
          disabled={scheduleBusy}
          onchange={async (e) => {
            const select = e.currentTarget;
            if (!(await saveSchedule({ frequency: select.value as BackupFrequency }))) select.value = schedule.frequency;
          }}
        >
          <option value="daily">Every day</option>
          <option value="weekly">Every week</option>
        </select>
      </label>
      <label>
        Keep the latest
        <input
          type="number"
          min="1"
          max="100"
          value={schedule.retention}
          disabled={scheduleBusy}
          aria-label="Number of automatic backups to keep"
          onchange={async (e) => {
            const input = e.currentTarget;
            if (!(await saveSchedule({ retention: Number(input.value) }))) input.value = String(schedule.retention);
          }}
        />
      </label>
    </div>
    <p class="muted">Older automatic backups are deleted only after a new one is saved and checked. Other files in the folder are never touched.</p>
    {#if scheduleStatus}
      <p class="backup-result" role="status">
        Last backup: {when(scheduleStatus.lastSuccessMs)}
        {#if schedule.enabled && scheduleStatus.nextDueMs} · Next: {when(scheduleStatus.nextDueMs)}{/if}
      </p>
      {#if scheduleStatus.lastError}
        <p class="backup-error" role="alert">The last attempt failed: {scheduleStatus.lastError}</p>
      {/if}
    {/if}
    {#if scheduleError}<p class="backup-error" role="alert">{scheduleError}</p>{/if}
    <button class="button secondary" disabled={scheduleBusy || !schedule.directory} onclick={backUpNow}>
      <Download size={15} />Back up now
    </button>
    {#if lastRun}
      <p class="backup-result" role="status">
        Saved <code>{lastRun.file}</code>{#if lastRun.pruned.length} and removed {lastRun.pruned.length} older automatic {lastRun.pruned.length === 1 ? 'backup' : 'backups'}{/if}.
      </p>
    {/if}
  </div>

  <div class="backup-block">
    <h3>Save a backup</h3>
    <fieldset class="backup-sections">
      <legend>Include</legend>
      {#each EXPORT_SECTIONS as section}
        <label><input type="checkbox" bind:checked={exportChoice[section]} />{label(section)}</label>
      {/each}
    </fieldset>
    <button class="button secondary" disabled={exporting || selectedExport.length === 0} onclick={exportNow}>
      <Download size={15} />{exporting ? 'Saving…' : 'Save backup…'}
    </button>
    {#if exported}
      <p class="backup-result" role="status">
        Saved {exported.library} library {exported.library === 1 ? 'entry' : 'entries'}, {exported.history} history,
        {exported.progress} progress to <code>{exported.path}</code>.
      </p>
    {/if}
  </div>

  <div class="backup-block">
    <h3>Restore from a backup</h3>
    <p class="muted">Works with backups saved here and with Cloudstream Android backups. Nothing changes until you confirm.</p>
    <button class="button secondary" disabled={working} onclick={chooseFile}>
      <FolderOpen size={15} />Choose backup file…
    </button>

    {#if error}<p class="backup-error" role="alert">{error}</p>{/if}

    {#if preview}
      <div class="backup-preview" aria-live="polite">
        <p>
          <strong>{fileName}</strong> ·
          {preview.source === 'android' ? 'Cloudstream Android backup' : 'Cloudstream Desktop backup'}
          {#if preview.appVersion} · made with version {preview.appVersion}{/if}
        </p>
        <fieldset class="backup-sections">
          <legend>Import</legend>
          {#each preview.availableSections as section}
            <label>
              <input type="checkbox" bind:checked={chosenSections[section]} onchange={() => runPreview()} />{label(section)}
            </label>
          {/each}
        </fieldset>
        <fieldset class="backup-sections">
          <legend>If something already exists here</legend>
          {#each [['skip_existing', 'Keep what I have'], ['keep_newest', 'Keep whichever is newer'], ['overwrite', 'Use the backup’s copy']] as [value, text]}
            <label>
              <input type="radio" name="conflict-policy" {value} bind:group={policy} onchange={() => runPreview()} />{text}
            </label>
          {/each}
        </fieldset>
        <table class="backup-table">
          <caption>What importing would do</caption>
          <thead><tr><th scope="col">Section</th><th scope="col">New</th><th scope="col">Replaced</th><th scope="col">Same</th><th scope="col">Skipped</th></tr></thead>
          <tbody>
            {#each preview.report.sections as row}
              <tr><th scope="row">{label(row.section)}</th><td>{row.new}</td><td>{row.replaced}</td><td>{row.unchanged}</td><td>{row.skipped}</td></tr>
            {/each}
          </tbody>
        </table>
        {#if preview.report.createdProfiles.length}
          <p>New profiles: {preview.report.createdProfiles.map((p) => p.name).join(', ')}</p>
        {/if}
        {#each preview.report.notes as note}<p class="muted">{note}</p>{/each}
        {#if preview.report.unresolved.length}
          <details>
            <summary>{preview.report.unresolved.length} titles have no matching source yet</summary>
            <ul>
              {#each preview.report.unresolved as title}
                <li><strong>{title.name}</strong> — {title.reason}</li>
              {/each}
            </ul>
          </details>
        {/if}
        <div class="backup-actions">
          <button class="button primary" disabled={working || selectedImport.length === 0} onclick={apply}>
            <Upload size={15} />{working ? 'Working…' : 'Import now'}
          </button>
          <button class="button secondary" disabled={working} onclick={cancelImport}>Cancel</button>
        </div>
        <p class="muted">A copy of your current data is saved before anything is changed.</p>
      </div>
    {/if}

    {#if applied}
      <div class="backup-preview" role="status">
        <p><strong>Import finished.</strong></p>
        <ul>
          {#each applied.sections as row}
            <li>{label(row.section)}: {row.new} added, {row.replaced} replaced, {row.unchanged} unchanged, {row.skipped} skipped</li>
          {/each}
        </ul>
        {#if applied.preImportBackup}<p class="muted">Your earlier data was saved to <code>{applied.preImportBackup}</code>.</p>{/if}
        {#each applied.notes as note}<p class="muted">{note}</p>{/each}
        {#if applied.repositoriesToAdd.length}
          <p>The backup lists {applied.repositoriesToAdd.length} {applied.repositoriesToAdd.length === 1 ? 'repository' : 'repositories'} that are not added here yet.</p>
          <button class="button secondary" disabled={addingRepositories} onclick={addRepositories}>
            {addingRepositories ? 'Adding…' : 'Add them now'}
          </button>
        {/if}
        {#if repositoryMessage}<p>{repositoryMessage}</p>{/if}
      </div>
    {/if}
  </div>

  {#if waiting.length}
    <div class="backup-block">
      <h3>Titles waiting for a source ({waiting.length})</h3>
      <p class="muted">
        These came from an Android backup but no installed source matches them. Install the source, then check again.
      </p>
      <ul class="backup-waiting">
        {#each waiting as title}<li><strong>{title.name}</strong> <span class="muted">({title.apiName})</span></li>{/each}
      </ul>
      <button class="button secondary" onclick={recheck}><RefreshCw size={15} />Check again</button>
    </div>
  {/if}
  <!-- Outside the list's block so the result stays visible after the list empties. -->
  {#if recheckMessage}<p class="backup-result" role="status">{recheckMessage}</p>{/if}
</section>

<style>
  .backup-block { display: grid; gap: 0.6rem; padding: 0.9rem 0; border-top: 1px solid var(--border); justify-items: start; }
  .backup-block h3 { margin: 0; font-size: 1rem; }
  .backup-sections { display: flex; flex-wrap: wrap; gap: 0.4rem 1.2rem; border: 1px solid var(--border); border-radius: 10px; padding: 0.5rem 0.9rem; margin: 0; }
  .backup-sections label { display: inline-flex; gap: 0.4rem; align-items: center; }
  .backup-preview { display: grid; gap: 0.7rem; width: 100%; background: var(--surface); border: 1px solid var(--border); border-radius: 12px; padding: 0.9rem 1rem; }
  .backup-table { border-collapse: collapse; width: 100%; max-width: 38rem; }
  .backup-table th, .backup-table td { text-align: left; padding: 0.25rem 0.6rem; border-bottom: 1px solid var(--border); }
  .backup-table caption { text-align: left; color: var(--muted); padding-bottom: 0.3rem; }
  .backup-row { display: flex; gap: 0.9rem; flex-wrap: wrap; align-items: center; }
  .backup-row label { display: inline-flex; gap: 0.4rem; align-items: center; }
  .backup-actions { display: flex; gap: 0.6rem; flex-wrap: wrap; }
  .backup-error { color: var(--danger); margin: 0; }
  .backup-result, .backup-waiting { margin: 0; }
  code { word-break: break-all; }
</style>
