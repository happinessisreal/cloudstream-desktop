<script lang="ts">
  import { onMount } from 'svelte';
  import { ChevronLeft, ChevronRight, ArrowRight } from 'lucide-svelte';
  import MediaCard from './MediaCard.svelte';
  import type { MediaItem } from './types';
  let {
    title,
    items,
    orientation = 'vertical',
    onopen,
    onexpand,
  }: {
    title: string;
    items: MediaItem[];
    orientation?: 'horizontal' | 'vertical' | string;
    onopen: (item: MediaItem) => void;
    onexpand: () => void;
  } = $props();
  let rail: HTMLDivElement;
  let canPrevious = $state(false);
  let canNext = $state(false);
  function measure() {
    if (rail) {
      canPrevious = rail.scrollLeft > 2;
      canNext = rail.scrollLeft + rail.clientWidth < rail.scrollWidth - 2;
    }
  }
  function scroll(direction: number) {
    rail.scrollBy({
      left: direction * rail.clientWidth * 0.8,
      behavior: matchMedia('(prefers-reduced-motion: reduce)').matches ? 'instant' : 'smooth',
    });
  }
  onMount(() => {
    const observer = new ResizeObserver(measure);
    observer.observe(rail);
    measure();
    return () => observer.disconnect();
  });
  $effect(() => {
    items.length;
    if (rail) requestAnimationFrame(measure);
  });
</script>

<section class="catalog-section" aria-label={title}>
  <div class="section-title">
    <h2>{title}</h2>
    <div class="rail-actions">
      <button class="text-button" onclick={onexpand}>See all <ArrowRight size={15} /></button
      ><button
        class="rail-arrow"
        aria-label={`Scroll ${title} left`}
        disabled={!canPrevious}
        onclick={() => scroll(-1)}><ChevronLeft size={19} /></button
      ><button
        class="rail-arrow"
        aria-label={`Scroll ${title} right`}
        disabled={!canNext}
        onclick={() => scroll(1)}><ChevronRight size={19} /></button
      >
    </div>
  </div>
  <div class="media-rail" class:horizontal={orientation === 'horizontal'} bind:this={rail} onscroll={measure}>
    {#each items as item, i}<MediaCard {item} {orientation} index={i} onclick={() => onopen(item)} />{/each}
  </div>
</section>
