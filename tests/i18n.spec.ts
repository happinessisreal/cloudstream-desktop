// Unit tests for the pure i18n helpers: locale resolution, fallback, interpolation, plurals,
// direction and catalog merging. They run in Node without a browser page.
import { test, expect } from '@playwright/test';
import { buildCatalogs, directionOf, fallbackChain, interpolate, languageOf, resolveLocale, translate, translatePlural } from '../src/lib/i18n-core';
import { readFileSync, readdirSync } from 'node:fs';

const available = ['en', 'ar', 'de', 'pt', 'pt-BR', 'zh', 'zh-TW', 'he'];

test('locale resolution follows the preference, then the browser, with language and English fallback', () => {
  expect(resolveLocale('de', available)).toBe('de');
  expect(resolveLocale('pt-BR', available)).toBe('pt-BR');
  expect(resolveLocale('pt-PT', available)).toBe('pt');
  expect(resolveLocale('zh-CN', available)).toBe('zh');
  expect(resolveLocale('zh-TW', available)).toBe('zh-TW');
  expect(resolveLocale('xx', available)).toBe('en');
  expect(resolveLocale('system', available, ['fr-FR', 'de-DE', 'en'])).toBe('de');
  expect(resolveLocale('system', available, ['fr-FR'])).toBe('en');
  expect(resolveLocale('system', available, [])).toBe('en');
  expect(resolveLocale('de_AT', available)).toBe('de');
  expect(resolveLocale('', available, ['ar'])).toBe('ar');
  expect(fallbackChain('pt-BR')).toEqual(['pt-BR', 'pt', 'en']);
  expect(fallbackChain('en')).toEqual(['en']);
  expect(languageOf('pt-BR')).toBe('pt');
});

test('text direction is right to left for the right languages only', () => {
  for (const code of ['ar', 'he', 'fa', 'ur', 'ar-EG', 'iw']) expect(directionOf(code), code).toBe('rtl');
  for (const code of ['en', 'de', 'ja', 'pt-BR', 'zh-TW', 'hi']) expect(directionOf(code), code).toBe('ltr');
});

test('missing keys fall back one key at a time, and interpolation never executes anything', () => {
  const catalogs = {
    en: { a: 'Hello {name}', b: 'Only English', c: 'Total {count}' },
    de: { a: 'Hallo {name}' },
  };
  expect(translate(catalogs, 'de', 'a', { name: 'Welt' })).toBe('Hallo Welt');
  expect(translate(catalogs, 'de', 'b')).toBe('Only English');
  expect(translate(catalogs, 'de-AT', 'a', { name: 'x' })).toBe('Hallo x');
  expect(translate(catalogs, 'xx', 'a', { name: 'x' })).toBe('Hello x');
  expect(translate(catalogs, 'de', 'nope')).toBe('nope');
  expect(translate(catalogs, 'en', 'c')).toBe('Total {count}');
  expect(interpolate('{a}{b}', { a: '<b>x</b>', b: 7 })).toBe('<b>x</b>7');
  expect(interpolate('no params')).toBe('no params');
  expect(interpolate('{a}', { b: 1 })).toBe('{a}');
});

test('plural forms follow each language rule', () => {
  const catalogs = {
    en: { 'n.one': '{count} title', 'n.other': '{count} titles' },
    ar: { 'n.zero': 'لا عناوين', 'n.one': 'عنوان واحد', 'n.two': 'عنوانان', 'n.few': '{count} عناوين', 'n.many': '{count} عنوانًا', 'n.other': '{count} عنوان' },
    ru: { 'n.one': '{count} название', 'n.few': '{count} названия', 'n.many': '{count} названий', 'n.other': '{count} названия' },
  };
  expect(translatePlural(catalogs, 'en', 'n', 1)).toBe('1 title');
  expect(translatePlural(catalogs, 'en', 'n', 0)).toBe('0 titles');
  expect(translatePlural(catalogs, 'en', 'n', 5)).toBe('5 titles');
  expect([0, 1, 2, 3, 11, 100].map((n) => translatePlural(catalogs, 'ar', 'n', n))).toEqual(['لا عناوين', 'عنوان واحد', 'عنوانان', '3 عناوين', '11 عنوانًا', '100 عنوان']);
  expect([1, 2, 5, 21, 22, 25].map((n) => translatePlural(catalogs, 'ru', 'n', n))).toEqual(['1 название', '2 названия', '5 названий', '21 название', '22 названия', '25 названий']);
  // A language without its own plural entries uses English's, per key.
  expect(translatePlural(catalogs, 'de', 'n', 3)).toBe('3 titles');
});

test('catalogs merge Android-ported entries under desktop-authored ones', () => {
  const merged = buildCatalogs({
    './locales/de.android.json': { 'nav.home': 'Startseite', 'nav.library': 'Bibliothek' },
    './locales/de.json': { 'nav.library': 'Meine Bibliothek', 'firstRun.finish': 'Fertig' },
    './locales/en.android.json': { 'nav.library': 'Library' },
    './locales/en.json': { 'nav.library': 'My library' },
  });
  expect(merged.de).toEqual({ 'nav.home': 'Startseite', 'nav.library': 'Meine Bibliothek', 'firstRun.finish': 'Fertig' });
  expect(merged.en['nav.library']).toBe('My library');
});

test('every shipped catalog only uses keys the English catalog defines, and keeps placeholders intact', () => {
  const dir = 'src/lib/locales';
  const english = { ...JSON.parse(readFileSync(`${dir}/en.android.json`, 'utf8')), ...JSON.parse(readFileSync(`${dir}/en.json`, 'utf8')) } as Record<string, string>;
  const placeholders = (text: string) => [...text.matchAll(/\{([a-zA-Z0-9_]+)\}/g)].map((m) => m[1]).sort();
  const files = readdirSync(dir).filter((f) => f.endsWith('.json'));
  expect(files.length).toBeGreaterThan(15);
  for (const file of files) {
    const catalog = JSON.parse(readFileSync(`${dir}/${file}`, 'utf8')) as Record<string, string>;
    for (const [key, text] of Object.entries(catalog)) {
      const base = key.replace(/\.(zero|one|two|few|many|other)$/, '');
      expect(Object.keys(english).some((k) => k === key || k.replace(/\.(zero|one|two|few|many|other)$/, '') === base), `${file}: ${key} is not an English key`).toBe(true);
      expect(typeof text).toBe('string');
      expect(text.trim().length, `${file}: ${key} is empty`).toBeGreaterThan(0);
      if (english[key] !== undefined && file !== 'en.json' && file !== 'en.android.json') {
        expect(placeholders(text), `${file}: ${key} changed its placeholders`).toEqual(placeholders(english[key]));
      }
    }
  }
});
