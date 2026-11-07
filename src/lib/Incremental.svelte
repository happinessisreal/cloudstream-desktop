<script lang="ts" generics="T">
  import type { Snippet } from 'svelte';
  import { onDestroy } from 'svelte';

  let {
    items,
    chunk = 120,
    children,
  }: { items: T[]; chunk?: number; children: Snippet<[T, number]> } = $props();

  // Large catalogs render in steps, so opening a library of thousands of titles stays fast.
  let visible = $state(120);
  let sentinel = $state<HTMLElement | null>(null);
  let observer: IntersectionObserver | undefined;

  // A new list (filter, sort, new search) starts again from the first chunk.
  $effect(() => {
    items;
    visible = chunk;
  });

  $effect(() => {
    observer?.disconnect();
    if (!sentinel || typeof IntersectionObserver === 'undefined') return;
    observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((e) => e.isIntersecting)) visible = Math.min(items.length, visible + chunk);
      },
      { rootMargin: '600px' },
    );
    observer.observe(sentinel);
  });
  onDestroy(() => observer?.disconnect());
</script>

{#each items.slice(0, visible) as item, i (i)}
  {@render children(item, i)}
{/each}
{#if visible < items.length}
  <div class="grid-more" bind:this={sentinel}>
    <span role="status">Showing {visible} of {items.length}</span>
    <button class="button secondary small" onclick={() => (visible = Math.min(items.length, visible + chunk))}>Show more</button>
  </div>
{/if}

<style>
  .grid-more { grid-column: 1 / -1; display: flex; gap: 0.8rem; align-items: center; justify-content: center; padding: 1rem 0; color: var(--muted); }
</style>
