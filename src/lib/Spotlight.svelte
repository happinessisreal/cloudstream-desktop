<script lang="ts">
  import { Info, Plus, Check, ChevronLeft, ChevronRight, Cloud } from 'lucide-svelte';
  import type { MediaItem } from './types';
  let {
    item,
    source,
    category,
    index,
    count,
    saved,
    busy,
    ondetails,
    onsave,
    onprevious,
    onnext,
  }: {
    item: MediaItem;
    source: string;
    category: string;
    index: number;
    count: number;
    saved: boolean;
    busy: boolean;
    ondetails: () => void;
    onsave: () => void;
    onprevious: () => void;
    onnext: () => void;
  } = $props();
  let failed = $state(false);
  $effect(() => {
    item.posterUrl;
    failed = false;
  });
</script>

<section class="spotlight" aria-label="Featured title">
  <div class="spotlight-art" aria-hidden="true">
    {#if item.posterUrl && !failed && !Object.keys(item.posterHeaders ?? {}).length}
      <img
        src={item.posterUrl}
        alt=""
        onerror={() => (failed = true)}
        referrerpolicy="no-referrer"
      />
    {:else}<Cloud size={190} strokeWidth={0.7} />{/if}
  </div>
  <div class="spotlight-shade"></div>
  <div class="spotlight-copy">
    <span class="spotlight-kicker"><span></span>{category}</span>
    <h2>{item.name}</h2>
    <div class="spotlight-meta">
      <span>{item.mediaType?.replace('TvSeries', 'TV series') ?? 'Video'}</span>{#if item.year}<span
          >{item.year}</span
        >{/if}<span>{source}</span>
    </div>
    <div class="button-row">
      <button class="button primary" disabled={busy} onclick={ondetails}
        ><Info size={19} />More info</button
      >
      <button class="button hero-secondary" disabled={busy} onclick={onsave}
        >{#if saved}<Check size={19} />{:else}<Plus size={19} />{/if}{saved
          ? 'In my library'
          : 'My list'}</button
      >
    </div>
  </div>
  <div class="spotlight-pagination">
    <span>{String(index + 1).padStart(2, '0')} <i>/</i> {String(count).padStart(2, '0')}</span
    ><button aria-label="Previous featured title" onclick={onprevious} disabled={count < 2}
      ><ChevronLeft size={20} /></button
    ><button aria-label="Next featured title" onclick={onnext} disabled={count < 2}
      ><ChevronRight size={20} /></button
    >
  </div>
</section>
