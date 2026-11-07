// Pure internationalisation helpers. No framework code lives here, so locale resolution, fallback,
// interpolation, plurals and text direction can be tested directly.

export type Catalog = Record<string, string>;
export type Catalogs = Record<string, Catalog>;

export const DEFAULT_LOCALE = 'en';

/** Languages written right to left. */
export const RTL_LANGUAGES = new Set(['ar', 'arz', 'he', 'iw', 'fa', 'ur', 'ckb', 'ps', 'sd', 'yi', 'dv']);

export function languageOf(locale: string): string {
  return locale.split('-')[0].toLowerCase();
}

export function directionOf(locale: string): 'ltr' | 'rtl' {
  return RTL_LANGUAGES.has(languageOf(locale)) ? 'rtl' : 'ltr';
}

/** `pt-BR` -> `pt-BR`, `pt`, then English. Used for per-key fallback. */
export function fallbackChain(locale: string): string[] {
  const chain = [locale];
  const language = languageOf(locale);
  if (language !== locale) chain.push(language);
  if (!chain.includes(DEFAULT_LOCALE)) chain.push(DEFAULT_LOCALE);
  return chain;
}

/**
 * Pick the locale to show. `preference` is `system` or a tag; for `system` the browser's languages
 * are tried in order. A regional tag falls back to its language (`pt-PT` -> `pt`) and anything
 * unknown ends at English.
 */
export function resolveLocale(preference: string, available: readonly string[], browser: readonly string[] = []): string {
  const have = new Set(available);
  const candidates = preference && preference !== 'system' ? [preference] : [...browser];
  for (const candidate of candidates) {
    const tag = candidate.replace('_', '-');
    if (have.has(tag)) return tag;
    const language = languageOf(tag);
    if (have.has(language)) return language;
    // Chinese variants: zh-TW needs its own catalog, but zh-CN and zh-SG use plain zh.
    const sameLanguage = available.find((a) => languageOf(a) === language && !a.includes('-'));
    if (sameLanguage) return sameLanguage;
  }
  return DEFAULT_LOCALE;
}

/** Replace `{name}` with the given values. Missing values leave the placeholder visible. */
export function interpolate(template: string, params?: Record<string, string | number>): string {
  if (!params) return template;
  return template.replace(/\{([a-zA-Z0-9_]+)\}/g, (whole, name: string) => (name in params ? String(params[name]) : whole));
}

export function translate(catalogs: Catalogs, locale: string, key: string, params?: Record<string, string | number>): string {
  for (const code of fallbackChain(locale)) {
    const text = catalogs[code]?.[key];
    if (text !== undefined) return interpolate(text, params);
  }
  // A missing key is shown as itself so it is noticed, never as an empty label.
  return key;
}

export function pluralCategory(locale: string, count: number): Intl.LDMLPluralRule {
  try {
    return new Intl.PluralRules(locale).select(count);
  } catch {
    return count === 1 ? 'one' : 'other';
  }
}

/** Look up `key.<category>` for the language's plural rule, falling back to `key.other`. */
export function translatePlural(catalogs: Catalogs, locale: string, key: string, count: number, params?: Record<string, string | number>): string {
  const category = pluralCategory(locale, count);
  const values = { count, ...params };
  for (const candidate of [`${key}.${category}`, `${key}.other`]) {
    for (const code of fallbackChain(locale)) {
      const text = catalogs[code]?.[candidate];
      if (text !== undefined) return interpolate(text, values);
    }
  }
  return `${key}`;
}

/** Merge Android-ported and desktop-authored catalog files into one catalog per locale. */
export function buildCatalogs(files: Record<string, Catalog>): Catalogs {
  const catalogs: Catalogs = {};
  // Android-ported first, then desktop-authored entries on top.
  for (const [path, content] of Object.entries(files).sort(([a], [b]) => Number(a.includes('.android.')) === Number(b.includes('.android.')) ? a.localeCompare(b) : a.includes('.android.') ? -1 : 1)) {
    const name = path.split('/').pop()!.replace(/\.android\.json$|\.json$/, '');
    catalogs[name] = { ...(catalogs[name] ?? {}), ...content };
  }
  return catalogs;
}
