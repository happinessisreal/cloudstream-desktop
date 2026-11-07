<script lang="ts">
  import { onMount } from 'svelte';
  import { Radio, RefreshCw, Check, AlertCircle, Sparkles, Trash2 } from 'lucide-svelte';
  import { call, toErrorMessage } from './api';

  const RECOMMENDED_TRACKERS = [
    'udp://tracker.opentrackr.org:1337/announce',
    'udp://open.demonii.com:1337/announce',
    'udp://open.stealth.si:80/announce',
    'udp://tracker.torrent.eu.org:451/announce',
    'udp://explodie.org:6969/announce',
    'udp://tracker.coppersurfer.tk:6969/announce',
    'udp://p4p.arenabg.com:1337/announce',
    'udp://movies.zsw.ca:6969/announce',
    'http://tracker.dler.org:6969/announce',
    'udp://tracker.internetwarriors.net:1337/announce',
    'udp://tracker.cyberia.is:6969/announce',
    'udp://tracker.tiny-vps.com:6969/announce',
  ];

  let trackersText = $state('');
  let saved = $state(false);
  let busy = $state(false);
  let error = $state('');

  function parseTrackers(text: string): string[] {
    return text
      .split('\n')
      .map((t) => t.trim())
      .filter((t) => t.length > 0 && !t.startsWith('#'));
  }

  onMount(async () => {
    try {
      const list = await call<string[]>('get_custom_trackers');
      trackersText = (list || []).join('\n');
    } catch (e) {
      error = toErrorMessage(e);
    }
  });

  async function save() {
    busy = true;
    error = '';
    saved = false;
    const trackers = parseTrackers(trackersText);
    try {
      await call('save_custom_trackers', { trackers });
      trackersText = trackers.join('\n');
      saved = true;
    } catch (e) {
      error = toErrorMessage(e, 'Could not save custom trackers');
    } finally {
      busy = false;
    }
  }

  function loadRecommended() {
    const existing = new Set(parseTrackers(trackersText));
    for (const t of RECOMMENDED_TRACKERS) {
      existing.add(t);
    }
    trackersText = Array.from(existing).join('\n');
    saved = false;
  }

  function clearAll() {
    trackersText = '';
    saved = false;
  }
</script>

<section class="settings-panel" data-keywords="torrent trackers bittorrent torrentio announce p2p magnet seeds peers">
  <div class="section-title">
    <h2>BitTorrent Trackers (Torrentio)</h2>
    <span class="muted">{parseTrackers(trackersText).length} custom trackers configured</span>
  </div>
  <p class="muted">
    Torrentio embeds these announce trackers into every generated magnet link. Adding reliable public or private trackers
    helps discover more seeders and peers for faster P2P streaming.
  </p>
  <div class="field-block">
    <label for="trackers-textarea" class="field-label">Custom announce tracker list (one per line):</label>
    <textarea
      id="trackers-textarea"
      class="trackers-textarea"
      rows="7"
      placeholder="udp://tracker.opentrackr.org:1337/announce&#10;udp://open.demonii.com:1337/announce"
      bind:value={trackersText}
    ></textarea>
  </div>
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
    <button class="button secondary" disabled={busy} onclick={loadRecommended}>
      <Sparkles size={16} />Load recommended trackers
    </button>
    <button class="button secondary" disabled={busy || !trackersText} onclick={clearAll}>
      <Trash2 size={16} />Clear list
    </button>
  </div>
  {#if error}
    <div class="alert error" style="margin-top: 10px;">
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
