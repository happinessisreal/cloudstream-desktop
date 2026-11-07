// Reactive wrapper around i18n-core: the chosen locale lives in one $state object, so every `t()`
// call in a template re-renders when it changes.
import { buildCatalogs, directionOf, resolveLocale, translate, translatePlural, type Catalog } from './i18n-core';

const files = import.meta.glob('./locales/*.json', { eager: true, import: 'default' }) as Record<string, Catalog>;
export const catalogs = buildCatalogs(files);
export const supportedLocales = Object.keys(catalogs).sort((a, b) => (a === 'en' ? -1 : b === 'en' ? 1 : a.localeCompare(b)));

export const i18n = $state({ preference: 'system', locale: 'en' });

export function t(key: string, params?: Record<string, string | number>): string {
  return translate(catalogs, i18n.locale, key, params);
}

export function tn(key: string, count: number, params?: Record<string, string | number>): string {
  return translatePlural(catalogs, i18n.locale, key, count, params);
}

export function formatNumber(value: number): string {
  return new Intl.NumberFormat(i18n.locale).format(value);
}

export function formatDateTime(ms: number): string {
  return new Intl.DateTimeFormat(i18n.locale, { dateStyle: 'medium', timeStyle: 'short' }).format(ms);
}

/** The language's own name for itself ("Deutsch", "العربية"), so a user can find theirs. */
export function languageName(code: string): string {
  try {
    return new Intl.DisplayNames([code], { type: 'language' }).of(code) ?? code;
  } catch {
    return code;
  }
}

export function setLocalePreference(preference: string) {
  const browser = typeof navigator !== 'undefined' ? navigator.languages : [];
  i18n.preference = preference;
  i18n.locale = resolveLocale(preference, supportedLocales, browser);
  if (typeof document !== 'undefined') {
    document.documentElement.lang = i18n.locale;
    document.documentElement.dir = directionOf(i18n.locale);
  }
}
