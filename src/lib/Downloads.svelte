<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { ArrowDown, ArrowUp, Download, FolderOpen, Pause, Play, RotateCcw, Trash2, X } from 'lucide-svelte';
  import { call, onDownloadEvent, toErrorMessage } from './api';
  import type { DownloadAction, DownloadJob, DownloadState, RemovalReport, WatchProgress } from './types';

  let {
    profileId,
    onchange,
  }: { profileId: string; onchange?: (jobs: DownloadJob[]) => void } = $props();

  let jobs = $state<DownloadJob[]>([]);
  let loading = $state(true);
  let loadError = $state('');
  let actionError = $state('');
  let notice = $state('');
  let working = $state<string | null>(null);
  let removing = $state<DownloadJob | null>(null);
  let removeDialog = $state<HTMLDialogElement | null>(null);
  let unlisten: (() => void) | undefined;
  let disposed = false;
  let clearDialog = $state<HTMLDialogElement | null>(null);
  let view = $state<'queue' | 'offline'>('queue');
  let watchProgress = $state<Record<string, WatchProgress>>({});
  let subtitleWarnings = $state<Record<string, number>>({});
  let health = $state<Record<string, 'available' | 'damaged' | 'unavailable'>>({});
  let healthError = $state('');
  let repairing = $state<DownloadJob | null>(null);
  let repairDialog = $state<HTMLDialogElement | null>(null);
  let progressError = $state('');
  // Transfer speed is estimated from the progress events themselves (smoothed), never invented.
  const samples = new Map<string, { at: number; bytes: number; speed: number }>();
  let speeds = $state<Record<string, number>>({});

  const STATE_LABEL: Record<DownloadState, string> = {
    queued: 'Waiting in queue',
    resolving: 'Finding a source',
    downloading: 'Downloading',
    paused: 'Paused',
    completed: 'Finished',
    failed: 'Failed',
    canceled: 'Canceled',
  };

  function sortJobs(list: DownloadJob[]) {
    return [...list].sort((a, b) => a.order - b.order || a.id.localeCompare(b.id));
  }

  function setJobs(next: DownloadJob[]) {
    jobs = sortJobs(next);
    onchange?.(jobs);
  }

  async function load() {
    const requestedProfile = profileId;
    try {
      const list = await call<DownloadJob[]>('download_jobs', { profileId: requestedProfile });
      if (disposed || requestedProfile !== profileId) return;
      setJobs(list);
      loadError = '';
    } catch (e) {
      if (!disposed && requestedProfile === profileId) loadError = toErrorMessage(e, 'Could not load downloads');
    } finally {
      if (!disposed && requestedProfile === profileId) loading = false;
    }
  }

  function sampleSpeed(job: DownloadJob) {
    if (job.state !== 'downloading') {
      samples.delete(job.id);
      if (job.id in speeds) {
        const { [job.id]: _gone, ...rest } = speeds;
        speeds = rest;
      }
      return;
    }
    const now = performance.now();
    const previous = samples.get(job.id);
    if (!previous || job.bytes < previous.bytes) {
      samples.set(job.id, { at: now, bytes: job.bytes, speed: 0 });
      return;
    }
    const seconds = (now - previous.at) / 1000;
    if (seconds < 0.4) return;
    const instant = (job.bytes - previous.bytes) / seconds;
    const speed = previous.speed > 0 ? previous.speed * 0.7 + instant * 0.3 : instant;
    samples.set(job.id, { at: now, bytes: job.bytes, speed });
    speeds = { ...speeds, [job.id]: speed };
  }

  function applyJob(job: DownloadJob) {
    if (job.profileId !== profileId) return;
    const existing = jobs.find((j) => j.id === job.id);
    // Events can arrive out of order; never replace a newer record with an older one.
    if (existing && existing.revision > job.revision) return;
    sampleSpeed(job);
    setJobs(existing ? jobs.map((j) => (j.id === job.id ? job : j)) : [...jobs, job]);
  }

  onMount(() => {
    void load();
    void onDownloadEvent((event) => {
      if (event.type === 'job_changed') applyJob(event.job);
      else if (event.type === 'queue_error') actionError = event.message;
      else void load();
    }).then((fn) => {
      if (disposed) fn();
      else unlisten = fn;
    });
  });
  onDestroy(() => {
    disposed = true;
    unlisten?.();
  });

  $effect(() => {
    // Switching profile shows that profile's queue only.
    profileId;
    loading = true;
    void load();
  });

  $effect(() => {
    profileId;
    watchProgress = {};
    subtitleWarnings = {};
    health = {};
    if (view === 'offline') void showOffline();
  });

  const queuedIds = $derived(jobs.filter((j) => j.state === 'queued').map((j) => j.id));
  const offlineGroups = $derived.by(() => {
    const titles = new Map<string, { id: string; title: string; seasons: Map<number | null, DownloadJob[]> }>();
    for (const job of jobs.filter((entry) => entry.state === 'completed')) {
      const title = titles.get(job.mediaId) ?? { id: job.mediaId, title: job.title, seasons: new Map<number | null, DownloadJob[]>() };
      titles.set(job.mediaId, title);
      const season = job.season ?? null;
      title.seasons.set(season, [...(title.seasons.get(season) ?? []), job]);
    }
    return [...titles.values()].sort((a, b) => a.title.localeCompare(b.title)).map((title) => ({
      ...title,
      seasons: [...title.seasons.entries()].sort(([a], [b]) => (a ?? 0) - (b ?? 0)).map(([season, episodes]) => ({
        season,
        episodes: episodes.sort((a, b) => (a.episode ?? 0) - (b.episode ?? 0) || a.episodeName.localeCompare(b.episodeName)),
      })),
    }));
  });

  async function showOffline() {
    view = 'offline';
    const requestedProfile = profileId;
    progressError = '';
    healthError = '';
    health = {};
    const healthCheck = call<Record<string, 'available' | 'damaged' | 'unavailable'>>('download_health', { profileId: requestedProfile }).then((result) => {
      if (!disposed && requestedProfile === profileId) health = result;
    }).catch((e) => {
      if (!disposed && requestedProfile === profileId) healthError = toErrorMessage(e, 'Could not check downloaded files');
    });
    try {
      const result = await call<Record<string, WatchProgress>>('download_watch_progress', { profileId: requestedProfile });
      if (!disposed && requestedProfile === profileId) watchProgress = result;
    } catch (e) {
      if (!disposed && requestedProfile === profileId) progressError = toErrorMessage(e, 'Watch progress is unavailable');
    }
    // Both checks take the same per-download lock; run them in sequence rather than causing
    // a healthy download to look unavailable while the other check is still reading it.
    await healthCheck;
    if (disposed || requestedProfile !== profileId) return;
    try {
      const warnings = await call<Record<string, number>>('download_subtitle_warnings', { profileId: requestedProfile });
      if (!disposed && requestedProfile === profileId) subtitleWarnings = warnings;
    } catch (e) {
      if (!disposed && requestedProfile === profileId) progressError = toErrorMessage(e, 'Subtitle information is unavailable');
    }
  }

  function watchLabel(job: DownloadJob) {
    const saved = watchProgress[job.id];
    if (saved?.isWatched) return 'Watched';
    if (!saved || saved.positionSeconds <= 0) return 'Not started';
    const minutes = Math.floor(saved.positionSeconds / 60);
    if (saved.durationSeconds && saved.durationSeconds > 0) return `${Math.min(100, Math.floor(saved.positionSeconds / saved.durationSeconds * 100))}% watched · ${minutes} min`;
    return `${minutes} min watched`;
  }

  async function control(job: DownloadJob, action: DownloadAction) {
    working = job.id;
    actionError = '';
    try {
      applyJob(await call<DownloadJob>('control_download', { profileId, id: job.id, action }));
    } catch (e) {
      actionError = toErrorMessage(e, 'That action could not be completed');
    } finally {
      working = null;
    }
  }

  async function play(job: DownloadJob) {
    working = job.id;
    actionError = '';
    try {
      await call('play_download', { profileId, id: job.id });
    } catch (e) {
      actionError = toErrorMessage(e, 'This download could not be played');
    } finally {
      working = null;
    }
  }

  async function confirmRepair() {
    const job = repairing;
    if (!job) return;
    working = job.id;
    actionError = '';
    try {
      const replacement = await call<DownloadJob>('repair_download', { profileId, id: job.id });
      applyJob(replacement);
      repairDialog?.close();
      repairing = null;
      notice = 'A replacement was added to the queue. The existing files were kept.';
      view = 'queue';
    } catch (e) {
      actionError = toErrorMessage(e, 'The replacement could not be queued');
      repairDialog?.close();
      repairing = null;
    } finally {
      working = null;
    }
  }

  async function move(job: DownloadJob, direction: -1 | 1) {
    const ids = [...queuedIds];
    const from = ids.indexOf(job.id);
    const to = from + direction;
    if (from < 0 || to < 0 || to >= ids.length) return;
    [ids[from], ids[to]] = [ids[to], ids[from]];
    working = job.id;
    actionError = '';
    try {
      await call('reorder_downloads', { profileId, ids });
      await load();
    } catch (e) {
      actionError = toErrorMessage(e, 'The queue could not be reordered');
      await load();
    } finally {
      working = null;
    }
  }

  function askRemove(job: DownloadJob) {
    removing = job;
    actionError = '';
    removeDialog?.showModal();
  }

  async function confirmRemove(deleteFile: boolean) {
    const job = removing;
    if (!job) return;
    working = job.id;
    try {
      const report = await call<RemovalReport>('remove_download', { profileId, id: job.id, deleteFile });
      removeDialog?.close();
      removing = null;
      setJobs(jobs.filter((j) => j.id !== job.id));
      const parts = [report.fileDeleted ? 'The file was deleted.' : deleteFile ? '' : 'The file was kept.', ...report.notes];
      notice = `Removed “${job.title}”. ${parts.filter(Boolean).join(' ')}`.trim();
    } catch (e) {
      actionError = toErrorMessage(e, 'The download could not be removed');
      removeDialog?.close();
      removing = null;
    } finally {
      working = null;
    }
  }

  function formatBytes(bytes: number) {
    if (!Number.isFinite(bytes) || bytes <= 0) return '0 MB';
    const units = ['B', 'KB', 'MB', 'GB', 'TB'];
    let value = bytes;
    let unit = 0;
    while (value >= 1024 && unit < units.length - 1) {
      value /= 1024;
      unit += 1;
    }
    return `${value >= 100 || unit === 0 ? Math.round(value) : value.toFixed(1)} ${units[unit]}`;
  }

  function progress(job: DownloadJob): number | null {
    if (job.state === 'completed') return 100;
    if (!job.total || job.total <= 0) return null;
    return Math.min(100, Math.floor((job.bytes / job.total) * 100));
  }

  function label(job: DownloadJob) {
    const parts = [job.title];
    if (job.season != null || job.episode != null) {
      parts.push(`S${job.season ?? 1}E${job.episode ?? '?'}`);
    }
    return parts.join(' · ');
  }

  function formatSpeed(job: DownloadJob): string | null {
    const speed = speeds[job.id];
    if (!speed || speed < 1) return null;
    const rate = `${formatBytes(speed)}/s`;
    if (!job.total || job.total <= job.bytes) return rate;
    const seconds = Math.ceil((job.total - job.bytes) / speed);
    const left =
      seconds < 90 ? `${seconds} s` : seconds < 5400 ? `${Math.round(seconds / 60)} min` : `${(seconds / 3600).toFixed(1)} h`;
    return `${rate} · about ${left} left`;
  }

  const clearable = $derived(jobs.filter((j) => j.state === 'completed' || j.state === 'canceled'));

  async function confirmClear() {
    const targets = [...clearable];
    let removed = 0;
    const problems: string[] = [];
    for (const job of targets) {
      try {
        await call<RemovalReport>('remove_download', { profileId, id: job.id, deleteFile: false });
        removed += 1;
        setJobs(jobs.filter((j) => j.id !== job.id));
      } catch (e) {
        problems.push(`${job.title}: ${toErrorMessage(e)}`);
      }
    }
    clearDialog?.close();
    notice = `Cleared ${removed} finished ${removed === 1 ? 'download' : 'downloads'}. Your files were kept.`;
    if (problems.length) actionError = problems.join(' · ');
  }

  const activeCount = $derived(jobs.filter((j) => j.state === 'resolving' || j.state === 'downloading').length);
</script>

<section class="downloads-page" aria-labelledby="downloads-heading">
  <div class="page-heading">
    <div>
      <h1 id="downloads-heading">Downloads</h1>
      <p>
        {#if jobs.length === 0}
          Files you download are saved in a folder you choose.
        {:else}
          {activeCount} active · {jobs.length} total. Downloads only run while the app is open.
        {/if}
      </p>
    </div>
    {#if clearable.length > 0}
      <button class="button secondary small" onclick={() => clearDialog?.showModal()}
        ><Trash2 size={14} />Clear finished</button
      >
    {/if}
  </div>

  {#if notice}
    <div class="inline-notice" role="status">
      {notice}
      <button class="icon-button" aria-label="Dismiss message" onclick={() => (notice = '')}><X size={16} /></button>
    </div>
  {/if}
  {#if actionError}
    <div class="inline-error" role="alert">
      {actionError}
      <button class="icon-button" aria-label="Dismiss error" onclick={() => (actionError = '')}><X size={16} /></button>
    </div>
  {/if}

  <nav class="download-views" aria-label="Download views">
    <button class="button small" class:primary={view === 'queue'} class:secondary={view !== 'queue'} aria-current={view === 'queue' ? 'page' : undefined} onclick={() => (view = 'queue')}>Queue</button>
    <button class="button small" class:primary={view === 'offline'} class:secondary={view !== 'offline'} aria-current={view === 'offline' ? 'page' : undefined} onclick={() => (view = 'offline')}>Offline library</button>
  </nav>

  {#if loading}
    <p class="muted" role="status">Loading downloads…</p>
  {:else if loadError}
    <div class="empty-state" role="alert">
      <h2>Downloads are unavailable</h2>
      <p>{loadError}</p>
      <button class="button secondary" onclick={() => void load()}><RotateCcw size={15} />Try again</button>
    </div>
  {:else if view === 'offline'}
    <section class="offline-library" aria-labelledby="offline-heading">
      <h2 id="offline-heading">Offline library</h2>
      {#if progressError}<p class="download-error" role="alert">{progressError}</p>{/if}
      {#if healthError}<p class="download-error" role="alert">{healthError}</p>{/if}
      {#if offlineGroups.length === 0}
        <p class="muted">Finished downloads will appear here, grouped by title and season.</p>
      {:else}
        {#each offlineGroups as group (group.id)}
          <section class="offline-title" aria-label={group.title}>
            <h3>{group.title}</h3>
            {#each group.seasons as season (season.season)}
              <h4>{season.season === null ? 'Episodes' : `Season ${season.season}`}</h4>
              <ul class="download-list">
                {#each season.episodes as job (job.id)}
                  <li class="download-row">
                    <div class="download-main">
                      <strong>{job.episodeName}</strong>
                      <span class="download-status">{watchLabel(job)} · {formatBytes(job.bytes)}</span>
                      {#if health[job.id] === 'damaged'}
                        <span class="download-error">Files are missing or have changed.</span>
                      {:else if health[job.id] === 'unavailable'}
                        <span class="download-error">The download folder is unavailable.</span>
                      {:else if !health[job.id] && !healthError}
                        <span class="download-status">Checking files…</span>
                      {/if}
                      {#if subtitleWarnings[job.id]}
                        <span class="download-error">{subtitleWarnings[job.id]} {subtitleWarnings[job.id] === 1 ? 'subtitle was' : 'subtitles were'} unavailable or unsupported.</span>
                      {/if}
                    </div>
                    <div class="download-actions">
                      <button class="button primary small" aria-label={`Play ${job.title} ${job.episodeName}`} disabled={working === job.id} onclick={() => play(job)}><Play size={14} />Play</button>
                      {#if health[job.id] === 'damaged'}
                        <button class="button secondary small" aria-label={`Repair ${job.title} ${job.episodeName}`} disabled={working === job.id} onclick={() => { repairing = job; repairDialog?.showModal(); }}><RotateCcw size={14} />Repair</button>
                      {/if}
                      <button class="button secondary small" aria-label={`Remove ${job.title} ${job.episodeName}`} disabled={working === job.id} onclick={() => askRemove(job)}><Trash2 size={14} />Remove</button>
                    </div>
                  </li>
                {/each}
              </ul>
            {/each}
          </section>
        {/each}
      {/if}
    </section>
  {:else if jobs.length === 0}
    <div class="empty-state">
      <Download size={34} />
      <h2>Nothing downloading</h2>
      <p>Open a title and use the download button on an episode. Choose a file type or an HLS folder to match the source.</p>
    </div>
  {:else}
    <ul class="download-list" aria-label="Download queue">
      {#each jobs as job (job.id)}
        {@const pct = progress(job)}
        <li class="download-row" data-state={job.state} data-download-id={job.id}>
          <div class="download-main">
            <strong>{label(job)}</strong>
            <span class="download-episode">{job.episodeName}</span>
            <div class="download-status">
              <span class={`state-pill state-${job.state}`}>{STATE_LABEL[job.state]}</span>
              {#if job.state === 'downloading' || job.state === 'paused' || job.state === 'queued'}
                {#if job.bytes > 0}
                  <span>{formatBytes(job.bytes)}{job.total ? ` of ${formatBytes(job.total)}` : ''}</span>
                {/if}
                {#if job.state === 'downloading' && formatSpeed(job)}
                  <span class="download-speed" title="Estimated from recent progress">{formatSpeed(job)}</span>
                {/if}
              {:else if job.state === 'completed'}
                <span>{formatBytes(job.total ?? job.bytes)}</span>
              {/if}
            </div>
            {#if job.state === 'downloading' || job.state === 'paused' || job.state === 'resolving'}
              <div
                class="download-bar"
                role="progressbar"
                aria-label={`Progress for ${job.title} ${job.episodeName}`}
                aria-valuemin="0"
                aria-valuemax="100"
                aria-valuenow={pct ?? undefined}
                aria-valuetext={pct == null ? 'Size unknown' : `${pct}%`}
              >
                <span class:indeterminate={pct == null} style={pct == null ? '' : `width:${pct}%`}></span>
              </div>
            {/if}
            {#if job.error}<p class="download-error">{job.error}</p>{/if}
            {#if job.state === 'completed' && job.output}
              <p class="download-path" title={job.output}><FolderOpen size={13} />{job.output}</p>
            {:else}
              <p class="download-path" title={job.destination}><FolderOpen size={13} />{job.destination}</p>
            {/if}
          </div>
          <div class="download-actions">
            {#if job.state === 'completed'}
              <button class="button primary small" aria-label={`Play ${job.title} ${job.episodeName}`} disabled={working === job.id} onclick={() => play(job)}
                ><Play size={14} />Play</button
              >
            {/if}
            {#if job.state === 'queued'}
              <button
                class="icon-button"
                aria-label={`Move ${job.title} ${job.episodeName} up`}
                disabled={working === job.id || queuedIds.indexOf(job.id) <= 0}
                onclick={() => move(job, -1)}><ArrowUp size={16} /></button
              >
              <button
                class="icon-button"
                aria-label={`Move ${job.title} ${job.episodeName} down`}
                disabled={working === job.id || queuedIds.indexOf(job.id) === queuedIds.length - 1}
                onclick={() => move(job, 1)}><ArrowDown size={16} /></button
              >
            {/if}
            {#if job.state === 'queued' || job.state === 'resolving' || job.state === 'downloading'}
              <button class="button secondary small" disabled={working === job.id} onclick={() => control(job, 'pause')}
                ><Pause size={14} />Pause</button
              >
            {/if}
            {#if job.state === 'paused'}
              <button class="button secondary small" disabled={working === job.id} onclick={() => control(job, 'resume')}
                ><Play size={14} />Resume</button
              >
            {/if}
            {#if job.state === 'failed' || job.state === 'canceled'}
              <button class="button secondary small" disabled={working === job.id} onclick={() => control(job, 'retry')}
                ><RotateCcw size={14} />Retry</button
              >
            {/if}
            {#if job.state === 'queued' || job.state === 'resolving' || job.state === 'downloading' || job.state === 'paused' || job.state === 'failed'}
              <button class="button secondary small" disabled={working === job.id} onclick={() => control(job, 'cancel')}
                ><X size={14} />Cancel</button
              >
            {/if}
            {#if job.state === 'completed' || job.state === 'failed' || job.state === 'canceled' || job.state === 'paused' || job.state === 'queued'}
              <button
                class="button secondary small"
                aria-label={`Remove ${job.title} ${job.episodeName}`}
                disabled={working === job.id}
                onclick={() => askRemove(job)}><Trash2 size={14} />Remove</button
              >
            {/if}
          </div>
        </li>
      {/each}
    </ul>
  {/if}

  <dialog bind:this={clearDialog} class="app-dialog" aria-labelledby="clear-downloads-title">
    <h2 id="clear-downloads-title">Clear finished downloads?</h2>
    <p>
      This removes {clearable.length} finished or canceled {clearable.length === 1 ? 'item' : 'items'} from the list.
      Downloaded files are not deleted.
    </p>
    <div class="dialog-actions">
      <button class="button secondary" onclick={() => clearDialog?.close()}>Keep them</button>
      <button class="button primary" onclick={confirmClear}>Clear list</button>
    </div>
  </dialog>

  <dialog bind:this={repairDialog} class="app-dialog" aria-labelledby="repair-download-title" onclose={() => (repairing = null)}>
    {#if repairing}
      <h2 id="repair-download-title">Download a replacement?</h2>
      <p>A new copy of {label(repairing)} — {repairing.episodeName} will be downloaded to the same folder. The existing files, including any edits, will be kept.</p>
      <div class="dialog-actions">
        <button class="button secondary" onclick={() => repairDialog?.close()}>Cancel</button>
        <button class="button primary" disabled={working === repairing.id} onclick={confirmRepair}>Download replacement</button>
      </div>
    {/if}
  </dialog>

  <dialog
    bind:this={removeDialog}
    class="app-dialog"
    aria-labelledby="remove-download-title"
    onclose={() => (removing = null)}
  >
    {#if removing}
      <h2 id="remove-download-title">Remove this download?</h2>
      <p>
        <strong>{label(removing)}</strong> — {removing.episodeName}
      </p>
      {#if removing.state === 'completed'}
        <p>
          The finished video stays on your computer unless you choose to delete it. A file you moved,
          replaced or edited is never deleted.
        </p>
      {:else}
        <p>The unfinished data for this download will be discarded.</p>
      {/if}
      <div class="dialog-actions">
        <button class="button secondary" onclick={() => removeDialog?.close()}>Keep it</button>
        <button class="button secondary" disabled={working === removing.id} onclick={() => confirmRemove(false)}>
          {removing.state === 'completed' ? 'Remove from list, keep file' : 'Remove from list'}
        </button>
        {#if removing.state === 'completed'}
          <button class="button primary danger" disabled={working === removing.id} onclick={() => confirmRemove(true)}
            ><Trash2 size={15} />Delete file too</button
          >
        {/if}
      </div>
    {/if}
  </dialog>
</section>

<style>
  .downloads-page { display: grid; gap: 1rem; }
  .download-views { display: flex; gap: 0.5rem; flex-wrap: wrap; }
  .offline-library, .offline-title { display: grid; gap: 0.75rem; }
  .offline-title h3, .offline-title h4, .offline-library h2 { margin: 0; }
  .page-heading { display: flex; justify-content: space-between; align-items: flex-start; gap: 1rem; flex-wrap: wrap; }
  .download-list { list-style: none; margin: 0; padding: 0; display: grid; gap: 0.75rem; }
  .download-row {
    display: flex; gap: 1rem; justify-content: space-between; align-items: flex-start;
    flex-wrap: wrap; background: var(--surface); border: 1px solid var(--border);
    border-radius: 12px; padding: 0.9rem 1rem;
  }
  .download-main { display: grid; gap: 0.3rem; min-width: 0; flex: 1 1 18rem; }
  .download-episode { color: var(--muted); }
  .download-status { display: flex; gap: 0.6rem; align-items: center; color: var(--muted); flex-wrap: wrap; }
  .state-pill { padding: 0.1rem 0.55rem; border-radius: 999px; background: var(--raised); color: var(--accent); font-size: 0.85rem; }
  .state-failed { color: var(--danger); }
  .state-completed { color: #a8e6b0; }
  .download-bar { height: 6px; border-radius: 999px; background: var(--raised); overflow: hidden; }
  .download-bar span { display: block; height: 100%; background: var(--primary); transition: width 0.3s; }
  .download-bar span.indeterminate { width: 35%; animation: download-slide 1.4s ease-in-out infinite; }
  @keyframes download-slide { 0% { margin-left: -35%; } 100% { margin-left: 100%; } }
  @media (prefers-reduced-motion: reduce) { .download-bar span.indeterminate { animation: none; width: 100%; opacity: 0.5; } }
  .download-error { margin: 0; color: var(--danger); }
  .download-path { margin: 0; color: var(--subtle); display: flex; gap: 0.35rem; align-items: center; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 0.85rem; }
  .download-actions { display: flex; gap: 0.4rem; flex-wrap: wrap; align-items: center; }
  .dialog-actions { display: flex; gap: 0.5rem; flex-wrap: wrap; justify-content: flex-end; margin-top: 1rem; }
  .danger { background: #8c2f2f; }
  .inline-notice, .inline-error { display: flex; justify-content: space-between; align-items: center; gap: 0.5rem; padding: 0.6rem 0.9rem; border-radius: 10px; background: var(--raised); }
  .inline-error { color: var(--danger); }
  .empty-state { display: grid; justify-items: center; gap: 0.5rem; padding: 3rem 1rem; text-align: center; color: var(--muted); }
</style>
