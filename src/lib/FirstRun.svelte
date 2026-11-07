<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import { call, toErrorMessage, updateSettings } from './api';
  import { i18n, languageName, setLocalePreference, supportedLocales, t } from './i18n.svelte';
  import type { Settings } from './types';

  let { settings, onchange, onfinish }: { settings: Settings; onchange: (settings: Settings) => void; onfinish: (settings: Settings) => void } = $props();

  const SOURCE_LANGUAGES = ['en', 'es', 'fr', 'de', 'it', 'pt', 'ru', 'ar', 'hi', 'ja', 'ko', 'zh', 'tr', 'nl', 'pl'];
  const TOTAL = 4;

  // The setup starts from the settings as they are when it opens, then edits its own copy.
  const initial = untrack(() => settings);
  let dialog = $state<HTMLDialogElement | null>(null);
  let step = $state(0);
  let busy = $state(false);
  let error = $state('');
  let locale = $state(initial.global.appearance?.locale ?? 'system');
  let sourceLanguages = $state<string[]>([...((initial.global as { providerLanguages?: string[] }).providerLanguages ?? [])]);
  let compact = $state(initial.global.appearance?.posterDensity === 'compact');
  let autoNext = $state(initial.global.playback?.autoNext ?? true);
  let arrows = $state(initial.global.appearance?.navigation === 'spatial');
  let repositoryUrl = $state('');
  let repositoryMessage = $state('');
  let heading = $state<HTMLElement | null>(null);

  onMount(() => dialog?.showModal());

  // Move focus to the new step's heading so keyboard and screen-reader users follow along.
  $effect(() => {
    step;
    void Promise.resolve().then(() => heading?.focus());
  });

  const systemLanguage = $derived(languageName(typeof navigator !== 'undefined' ? navigator.language.split('-')[0] : 'en'));

  async function save(patch: Record<string, unknown>) {
    error = '';
    try {
      const updated = await updateSettings(patch, 'global');
      // The app's own settings stay the single source of truth, so a later refresh cannot undo a choice.
      onchange(updated);
      return updated;
    } catch (e) {
      error = toErrorMessage(e);
      throw e;
    }
  }

  async function chooseLocale(value: string) {
    locale = value;
    // Applied at once so the next screens already show the chosen language.
    setLocalePreference(value);
    await save({ appearance: { locale: value } }).catch(() => {});
  }

  async function next() {
    busy = true;
    try {
      if (step === 1) await save({ providerLanguages: sourceLanguages });
      if (step === 2) {
        await save({ appearance: { posterDensity: compact ? 'compact' : 'comfortable', navigation: arrows ? 'spatial' : 'pointer' }, playback: { autoNext } });
      }
      step += 1;
    } catch {
      // The message is already shown.
    } finally {
      busy = false;
    }
  }

  async function addRepository() {
    const url = repositoryUrl.trim();
    if (!url) return;
    busy = true;
    repositoryMessage = '';
    error = '';
    try {
      await call('add_repository', { url });
      repositoryMessage = t('firstRun.repo.added');
      repositoryUrl = '';
    } catch (e) {
      error = toErrorMessage(e);
    } finally {
      busy = false;
    }
  }

  async function finish(skipped: boolean) {
    busy = true;
    try {
      const updated = await save({ setup: { completed: true, skipped, completedAtMs: Date.now(), version: 1 } });
      dialog?.close();
      onfinish(updated);
    } catch {
      // Stay open so the person can retry; the message is shown.
    } finally {
      busy = false;
    }
  }

  function toggleLanguage(code: string) {
    sourceLanguages = sourceLanguages.includes(code) ? sourceLanguages.filter((c) => c !== code) : [...sourceLanguages, code];
  }
</script>

<dialog
  bind:this={dialog}
  class="app-dialog first-run"
  aria-labelledby="first-run-title"
  oncancel={(event) => event.preventDefault()}
>
  <p class="muted" role="status">{t('firstRun.step', { current: step + 1, total: TOTAL })}</p>
  <h2 id="first-run-title" bind:this={heading} tabindex="-1">
    {#if step === 0}{t('firstRun.language.heading')}
    {:else if step === 1}{t('firstRun.sources.heading')}
    {:else if step === 2}{t('firstRun.display.heading')}
    {:else}{t('firstRun.repo.heading')}{/if}
  </h2>

  {#if step === 0}
    <p class="muted">{t('firstRun.language.hint')}</p>
    <label class="first-run-field">
      {t('settings.interfaceLanguage')}
      <select class="filter-select" value={locale} onchange={(e) => chooseLocale(e.currentTarget.value)}>
        <option value="system">{t('settings.interfaceLanguage.system', { language: systemLanguage })}</option>
        {#each supportedLocales as code}<option value={code}>{languageName(code)}</option>{/each}
      </select>
    </label>
  {:else if step === 1}
    <p class="muted">{t('firstRun.sources.hint')}</p>
    <fieldset class="first-run-languages">
      <legend class="visually-hidden">{t('settings.sourceLanguages')}</legend>
      {#each SOURCE_LANGUAGES as code}
        <label><input type="checkbox" checked={sourceLanguages.includes(code)} onchange={() => toggleLanguage(code)} />{languageName(code)}</label>
      {/each}
    </fieldset>
  {:else if step === 2}
    <div class="first-run-options">
      <label><input type="checkbox" bind:checked={compact} />{t('firstRun.display.compact')}</label>
      <label><input type="checkbox" bind:checked={autoNext} />{t('firstRun.display.autoplay')}</label>
      <label><input type="checkbox" bind:checked={arrows} />{t('firstRun.display.arrows')}</label>
    </div>
  {:else}
    <p class="muted">{t('firstRun.repo.hint')}</p>
    <div class="first-run-repo">
      <label class="first-run-field">
        {t('firstRun.repo.label')}
        <input type="text" bind:value={repositoryUrl} placeholder="https://" />
      </label>
      <button class="button secondary" disabled={busy || !repositoryUrl.trim()} onclick={addRepository}>{t('firstRun.repo.add')}</button>
    </div>
    {#if repositoryMessage}<p role="status">{repositoryMessage}</p>{/if}
  {/if}

  {#if error}<p class="first-run-error" role="alert">{error}</p>{/if}

  <div class="first-run-actions">
    <button class="button secondary" disabled={busy} onclick={() => finish(true)}>{t('firstRun.skip')}</button>
    <span class="spacer"></span>
    {#if step > 0}<button class="button secondary" disabled={busy} onclick={() => (step -= 1)}>{t('common.previous')}</button>{/if}
    {#if step < TOTAL - 1}
      <button class="button primary" disabled={busy} onclick={next}>{t('common.next')}</button>
    {:else}
      <button class="button primary" disabled={busy} onclick={() => finish(false)}>{t('firstRun.finish')}</button>
    {/if}
  </div>
  <!-- Current language is read here so the dialog re-renders when it changes. -->
  <span class="visually-hidden" lang={i18n.locale}></span>
</dialog>

<style>
  .first-run { max-width: min(36rem, 92vw); }
  .first-run-field { display: grid; gap: 0.3rem; }
  .first-run-field input { padding: 0.5rem 0.7rem; border-radius: 8px; border: 1px solid var(--border); background: var(--surface); color: inherit; font: inherit; }
  .first-run-languages { display: grid; grid-template-columns: repeat(auto-fill, minmax(10rem, 1fr)); gap: 0.4rem 1rem; border: 0; padding: 0; margin: 0.5rem 0; }
  .first-run-languages label, .first-run-options label { display: flex; gap: 0.5rem; align-items: center; }
  .first-run-options { display: grid; gap: 0.6rem; margin: 0.5rem 0; }
  .first-run-repo { display: flex; gap: 0.7rem; align-items: end; flex-wrap: wrap; }
  .first-run-repo .first-run-field { flex: 1 1 14rem; }
  .first-run-actions { display: flex; gap: 0.5rem; align-items: center; margin-top: 1.2rem; flex-wrap: wrap; }
  .first-run-actions .spacer { flex: 1; }
  .first-run-error { color: var(--danger); margin: 0.5rem 0 0; }
</style>
