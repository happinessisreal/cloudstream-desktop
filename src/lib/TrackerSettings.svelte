<script lang="ts">
  import { onMount } from 'svelte';
  import { Radio, RefreshCw, Check, AlertCircle, Upload, Trash2, ShieldCheck } from 'lucide-svelte';
  import { call, toErrorMessage } from './api';

  let trackersText = $state('');
  let saved = $state(false);
  let busy = $state(false);
  let error = $state('');
  let infoMessage = $state('');
  let fileInput: HTMLInputElement;

  function normalizeTracker(url: string): string {
    let trimmed = url.trim();
    if (!trimmed || trimmed.startsWith('#')) return '';
    if (!trimmed.includes('://')) {
      trimmed = `udp://${trimmed}`;
    }
    // Remove trailing slashes and lowercase for canonical comparison
    return trimmed.toLowerCase().replace(/\/+$/, '');
  }

  function parseUniqueTrackers(text: string): { unique: string[]; duplicatesCount: number } {
    const seen = new Set<string>();
    const result: string[] = [];
    let duplicatesCount = 0;

    // Split on newlines, commas, semicolons or double-spaces
    const tokens = text.split(/[\r\n,;]+|\s{2,}/);
    for (const raw of tokens) {
      const clean = normalizeTracker(raw);
      if (!clean) continue;
      const canonical = clean.toLowerCase();
      if (seen.has(canonical)) {
        duplicatesCount++;
      } else {
        seen.add(canonical);
        result.push(clean);
      }
    }
    return { unique: result, duplicatesCount };
  }

  onMount(async () => {
    try {
      const list = await call<string[]>('get_custom_trackers');
      const { unique } = parseUniqueTrackers((list || []).join('\n'));
      trackersText = unique.join('\n');
    } catch (e) {
      error = toErrorMessage(e);
    }
  });

  async function save() {
    busy = true;
    error = '';
    infoMessage = '';
    saved = false;
    const { unique, duplicatesCount } = parseUniqueTrackers(trackersText);
    try {
      await call('save_custom_trackers', { trackers: unique });
      trackersText = unique.join('\n');
      saved = true;
      if (duplicatesCount > 0) {
        infoMessage = `Saved ${unique.length} unique trackers (${duplicatesCount} duplicate${duplicatesCount === 1 ? '' : 's'} removed).`;
      }
    } catch (e) {
      error = toErrorMessage(e, 'Could not save custom trackers');
    } finally {
      busy = false;
    }
  }

  async function handleFileUpload(event: Event) {
    const input = event.target as HTMLInputElement;
    const file = input.files?.[0];
    if (!file) return;

    try {
      const content = await file.text();
      const existing = parseUniqueTrackers(trackersText).unique;
      const parsedFromFile = parseUniqueTrackers(content);

      const seen = new Set(existing.map((t) => t.toLowerCase()));
      const added: string[] = [];
      let duplicateMatches = parsedFromFile.duplicatesCount;

      for (const t of parsedFromFile.unique) {
        const canonical = t.toLowerCase();
        if (seen.has(canonical)) {
          duplicateMatches++;
        } else {
          seen.add(canonical);
          added.push(t);
        }
      }

      const merged = [...existing, ...added];
      trackersText = merged.join('\n');
      saved = false;
      infoMessage = `Imported ${added.length} new tracker${added.length === 1 ? '' : 's'} from ${file.name}${duplicateMatches > 0 ? ` (${duplicateMatches} duplicate${duplicateMatches === 1 ? '' : 's'} ignored)` : ''}. Click "Save trackers" to apply.`;
    } catch (e) {
      error = toErrorMessage(e, 'Failed to read the uploaded .txt file');
    } finally {
      input.value = '';
    }
  }

  function clearAll() {
    trackersText = '';
    saved = false;
    infoMessage = '';
  }
</script>

<section class="settings-panel" data-keywords="torrent trackers bittorrent torrentio announce p2p magnet seeds peers">
  <div class="section-title">
    <h2>BitTorrent Trackers (Torrentio)</h2>
    <span class="muted">{parseUniqueTrackers(trackersText).unique.length} trackers configured</span>
  </div>
  <p class="muted">
    Upload a <code>.txt</code> file or paste line-separated announce URLs below. All duplicates are automatically
    detected and removed. Every saved tracker will be embedded into generated magnet links.
  </p>

  <div class="field-block">
    <label for="trackers-textarea" class="field-label">Custom announce tracker list (line-separated):</label>
    <textarea
      id="trackers-textarea"
      class="trackers-textarea"
      rows="7"
      placeholder="Paste your tracker list here (one per line, e.g. udp://tracker.example.com:1337/announce)"
      bind:value={trackersText}
      oninput={() => {
        saved = false;
        infoMessage = '';
      }}
    ></textarea>
  </div>

  <input
    type="file"
    accept=".txt,text/plain"
    bind:this={fileInput}
    style="display: none;"
    onchange={handleFileUpload}
  />

  <div class="button-row" style="margin-top: 12px; display: flex; gap: 10px; flex-wrap: wrap;">
    <button class="button primary" disabled={busy} onclick={save}>
      {#if busy}
        <RefreshCw class="spin" size={16} />
      {:else if saved}
        <Check size={16} />
      {:else}
        <Radio size={16} />
      {/if}
      {saved ? 'Trackers saved' : 'Save trackers'}
    </button>
    <button class="button secondary" disabled={busy} onclick={() => fileInput?.click()}>
      <Upload size={16} />Upload .txt file
    </button>
    <button class="button secondary" disabled={busy || !trackersText} onclick={clearAll}>
      <Trash2 size={16} />Clear list
    </button>
  </div>

  {#if infoMessage}
    <div class="alert info" style="margin-top: 12px; display: flex; align-items: center; gap: 8px;">
      <ShieldCheck size={16} />
      <span>{infoMessage}</span>
    </div>
  {/if}

  {#if error}
    <div class="alert error" style="margin-top: 12px; display: flex; align-items: center; gap: 8px;">
      <AlertCircle size={16} />
      <span>{error}</span>
    </div>
  {/if}
</section>

<style>
  .field-label {
    display: block;
    font-size: 13px;
    font-weight: 500;
    color: var(--text, #f0f0f5);
    margin-bottom: 8px;
  }
  .trackers-textarea {
    width: 100%;
    background: #14151c;
    border: 1px solid var(--border, #2a2b38);
    border-radius: 8px;
    color: #e2e8f0;
    padding: 10px 12px;
    font-family: monospace;
    font-size: 12px;
    line-height: 1.5;
    resize: vertical;
    box-sizing: border-box;
  }
  .trackers-textarea:focus {
    outline: none;
    border-color: #6366f1;
    box-shadow: 0 0 0 2px rgba(99, 102, 241, 0.2);
  }
</style>
