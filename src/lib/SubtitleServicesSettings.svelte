<script lang="ts">
  import { onMount } from 'svelte';
  import { call, toErrorMessage, updateSettings } from './api';
  import type { HearingImpairedChoice, SubtitleService, SubtitleServicesView } from './types';

  let { profileId }: { profileId: string } = $props();

  let view = $state<SubtitleServicesView | null>(null);
  let error = $state('');
  let notice = $state('');
  let busy = $state('');
  let open = $state<string | null>(null);
  let values = $state<Record<string, string>>({});
  let languages = $state('');

  async function load() {
    try {
      view = await call<SubtitleServicesView>('subtitle_services', { profileId });
      languages = view.languages.join(', ');
    } catch (e) {
      error = toErrorMessage(e, 'Subtitle services could not be loaded');
    }
  }

  onMount(() => void load());

  async function save(patch: Record<string, unknown>) {
    error = '';
    try {
      await updateSettings({ subtitleServices: patch }, 'global');
      await load();
    } catch (e) {
      error = toErrorMessage(e, 'That setting could not be saved');
      await load();
    }
  }

  function saveLanguages() {
    const list = languages.split(',').map((l) => l.trim().toLowerCase()).filter(Boolean);
    void save({ languages: list });
  }

  function startConnect(service: SubtitleService) {
    open = service.id;
    values = Object.fromEntries(service.fields.map((f) => [f.name, f.secret ? '' : (service.account ?? '')]));
    notice = '';
    error = '';
  }

  async function connect(service: SubtitleService, event: SubmitEvent) {
    event.preventDefault();
    busy = service.id;
    error = '';
    notice = '';
    try {
      await call('connect_subtitle_service', { profileId, provider: service.id, fields: { ...values } });
      notice = `${service.name} is connected for this profile.`;
      open = null;
      values = {};
      await load();
    } catch (e) {
      error = `${service.name}: ${toErrorMessage(e, 'could not connect')}`;
    } finally {
      busy = '';
    }
  }

  async function disconnect(service: SubtitleService) {
    busy = service.id;
    error = '';
    try {
      await call('disconnect_subtitle_service', { profileId, provider: service.id });
      notice = `${service.name} was disconnected. Its saved details were removed from the keyring.`;
      await load();
    } catch (e) {
      error = `${service.name}: ${toErrorMessage(e, 'could not disconnect')}`;
    } finally {
      busy = '';
    }
  }

  function stateText(service: SubtitleService): string {
    switch (service.state.status) {
      case 'connected':
        return service.account ? `Connected as ${service.account}` : 'Connected';
      case 'disconnected':
        return 'Not connected';
      case 'open':
        return 'No account needed';
      default:
        return 'Keyring unavailable';
    }
  }
</script>

<section class="settings-panel" data-keywords="subtitles online opensubtitles subdl subsource addic7ed accounts api key sign in" aria-labelledby="subtitle-services-heading">
  <div class="section-title"><h2 id="subtitle-services-heading">Online subtitles</h2></div>
  <p class="muted">
    Search subtitle services for the video that is playing. When on, its title, year, season, episode and IDs are sent to
    the services you switch on below, only when you search. You choose which result to load; nothing is picked for you.
  </p>
  {#if view}
    <label class="sub-set-row"><input type="checkbox" checked={view.enabled} onchange={(e) => save({ enabled: e.currentTarget.checked })} />Search subtitle services</label>
    <div class="sub-set-grid" class:dim={!view.enabled}>
      <label>Languages to look for
        <input value={languages} disabled={!view.enabled} placeholder="en, fr" oninput={(e) => (languages = e.currentTarget.value)} onchange={saveLanguages} aria-describedby="sub-lang-help" />
      </label>
      <span id="sub-lang-help" class="muted small">Language tags, best first. Empty uses the player’s subtitle language.</span>
      <label>Hearing-impaired subtitles
        <select disabled={!view.enabled} value={view.hearingImpaired} onchange={(e) => save({ hearingImpaired: e.currentTarget.value as HearingImpairedChoice })}>
          <option value="include">Show both kinds</option>
          <option value="exclude">Hide them</option>
          <option value="only">Only show them</option>
        </select>
      </label>
    </div>
    {#if view.keyring.status === 'unavailable'}
      <p class="sub-set-warning" role="status">{view.keyring.message} Services that need a key or account cannot be connected until then.</p>
    {/if}
    <ul class="sub-services" aria-label="Subtitle services">
      {#each view.services as service (service.id)}
        <li class="sub-service" class:dim={!view.enabled}>
          <div class="sub-service-head">
            <label class="sub-set-row">
              <input type="checkbox" disabled={!view.enabled} checked={service.enabled} onchange={(e) => save({ providers: { [service.id]: e.currentTarget.checked } })} aria-label={`Use ${service.name}`} />
              <strong>{service.name}</strong>
            </label>
            <span class="sub-state" data-state={service.state.status}>{stateText(service)}</span>
          </div>
          <p class="muted small">{service.description} <span class="sub-site">{service.site}</span></p>
          {#if service.fields.length}
            <div class="sub-actions">
              {#if service.state.status === 'connected'}
                <button class="button secondary small" type="button" disabled={!!busy} onclick={() => disconnect(service)}>Disconnect {service.name}</button>
              {:else if open !== service.id}
                <button class="button secondary small" type="button" disabled={!view.enabled || !!busy || view.keyring.status !== 'ready'} onclick={() => startConnect(service)}>Connect {service.name}…</button>
              {/if}
            </div>
            {#if open === service.id}
              <form class="sub-connect" onsubmit={(e) => connect(service, e)} aria-label={`Connect ${service.name}`}>
                {#each service.fields as field (field.name)}
                  <label>{field.label}
                    <input type={field.secret ? 'password' : 'text'} autocomplete={field.secret ? 'off' : 'username'} required={field.required} value={values[field.name] ?? ''} oninput={(e) => (values[field.name] = e.currentTarget.value)} />
                    <span class="muted small">{field.help}</span>
                  </label>
                {/each}
                <p class="muted small">The details are checked with {service.name} and kept in your system keyring for this profile only.</p>
                <div class="sub-actions">
                  <button class="button secondary small" type="button" onclick={() => { open = null; values = {}; }}>Cancel</button>
                  <button class="button primary small" type="submit" disabled={busy === service.id}>{busy === service.id ? 'Checking…' : 'Connect'}</button>
                </div>
              </form>
            {/if}
          {/if}
        </li>
      {/each}
    </ul>
  {/if}
  {#if notice}<p class="muted" role="status">{notice}</p>{/if}
  {#if error}<p class="sub-set-error" role="alert">{error}</p>{/if}
</section>

<style>
  .sub-set-row { display: flex; gap: 0.6rem; align-items: center; }
  .sub-set-grid { display: grid; gap: 0.4rem; margin-block: 0.5rem; max-inline-size: 32rem; }
  .sub-set-grid label { display: grid; gap: 0.25rem; }
  .dim { opacity: 0.6; }
  .small { font-size: 0.85em; }
  .sub-services { list-style: none; padding: 0; margin: 0.5rem 0 0; display: grid; gap: 0.6rem; }
  .sub-service { border: 1px solid var(--border, #333); border-radius: 0.6rem; padding: 0.6rem 0.8rem; }
  .sub-service-head { display: flex; justify-content: space-between; gap: 0.5rem; flex-wrap: wrap; align-items: center; }
  .sub-state { font-size: 0.85em; }
  .sub-state[data-state='connected'] { color: var(--success, #6fcf97); }
  .sub-site { overflow-wrap: anywhere; }
  .sub-actions { display: flex; gap: 0.5rem; flex-wrap: wrap; margin-block-start: 0.4rem; }
  .sub-connect { display: grid; gap: 0.5rem; margin-block-start: 0.5rem; max-inline-size: 30rem; }
  .sub-connect label { display: grid; gap: 0.25rem; }
  .sub-set-warning { color: var(--warning, #f2c94c); }
  .sub-set-error { color: var(--danger); margin: 0; }
</style>
