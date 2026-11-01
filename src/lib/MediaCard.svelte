<script lang="ts">
  import { Film, Play } from 'lucide-svelte';
  import type { MediaItem } from './types';
  let {
    item,
    index = 0,
    onclick,
  }: { item: MediaItem; index?: number; onclick: () => void } = $props();
</script>

<button
  class="media-card"
  {onclick}
  aria-label={`Open ${item.name}`}
  style={`--card-hue:${220 + ((index * 19) % 45)}`}
>
  <div class="poster">
    {#if item.posterUrl && Object.keys(item.posterHeaders ?? {}).length === 0}
      <img
        src={item.posterUrl}
        alt=""
        loading="lazy"
        referrerpolicy="no-referrer"
        onerror={(event) => ((event.currentTarget as HTMLImageElement).style.display = 'none')}
      />
    {/if}
    <div class="poster-fallback"><Film size={38} strokeWidth={1} /><span>{item.name}</span></div>
    <span class="card-type">{item.mediaType?.replaceAll('TvSeries', 'Series') ?? 'Video'}</span>
    <span class="card-play"><Play size={22} fill="currentColor" /></span>
    <span class="poster-vignette"></span>
  </div>
  <div class="card-title"><strong>{item.name}</strong></div>
  <span class="card-meta"
    >{[item.year, item.mediaType?.replace('TvSeries', 'TV series')].filter(Boolean).join(' · ') ||
      'Video'}</span
  >
</button>
