#!/usr/bin/env python3
"""Port a mapped subset of Cloudstream Android's translations (T31).

The desktop UI keeps its own message keys. This script copies the *existing, maintained* Android
translations for the strings both apps share (navigation, common buttons, library statuses) into
`src/lib/locales/<code>.json`. Nothing is machine-translated here: a locale is written only if
Android has a translation for at least 80% of the mapped keys, and any key Android lacks stays
missing so the app falls back to English for it.

Android strings with placeholders or markup (`%s`, `<b>`) are skipped: their syntax does not map
onto the desktop interpolation. Source: https://github.com/recloudstream/cloudstream (GPL-3.0),
`app/src/main/res/values-*/strings.xml`, pinned in `reference/cloudstream-android`.

  scripts/port-android-translations.py
"""
import json, os, re, sys
import xml.etree.ElementTree as ET

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
RES = os.path.join(ROOT, "reference", "cloudstream-android", "app", "src", "main", "res")
OUT = os.path.join(ROOT, "src", "lib", "locales")

# desktop key -> Android string name
MAPPING = {
    "nav.home": "title_home",
    "nav.search": "search",
    "nav.library": "library",
    "nav.downloads": "title_downloads",
    "nav.extensions": "extensions",
    "nav.settings": "title_settings",
    "common.cancel": "cancel",
    "common.close": "sort_close",
    "common.save": "sort_save",
    "common.delete": "delete",
    "common.remove": "action_remove_from_bookmarks",
    "common.next": "next",
    "common.previous": "previous",
    "common.done": "setup_done",
    "common.yes": "yes",
    "common.no": "no",
    "common.add": "duplicate_add",
    "common.play": "home_play",
    "common.pause": "pause",
    "common.resume": "resume",
    "common.stop": "stop",
    "settings.general": "category_general",
    "settings.player": "category_player",
    "settings.subtitles": "pref_category_subtitles",
    "settings.providers": "category_providers",
    "settings.accounts": "pref_category_accounts",
    "settings.backup": "pref_category_backup",
    "settings.language": "extension_language",
    "status.watching": "type_watching",
    "status.on_hold": "type_on_hold",
    "status.completed": "type_completed",
    "status.dropped": "type_dropped",
    "status.plan_to_watch": "type_plan_to_watch",
    "library.favorites": "favorites_list_name",
}

# Android folder suffix -> desktop locale code
FOLDERS = {
    "b+ar": "ar", "b+de": "de", "b+es": "es", "b+fr": "fr", "b+hi": "hi", "b+id": None, "b+in": "id",
    "b+it": "it", "b+iw": "he", "b+ja": "ja", "b+ko": "ko", "b+nl": "nl", "b+pl": "pl", "b+pt": "pt",
    "b+pt+BR": "pt-BR", "b+ru": "ru", "b+tr": "tr", "b+uk": "uk", "b+vi": "vi", "b+zh": "zh", "b+fa": "fa",
}
MIN_COVERAGE = 0.8


def unescape(text):
    # Android resource escapes.
    return re.sub(r"\\(['\"@?\\])", r"\1", text.replace("\\n", "\n")).replace("\\u2026", "\u2026")


def read(folder):
    path = os.path.join(RES, folder, "strings.xml")
    if not os.path.isfile(path):
        return {}
    out = {}
    for node in ET.parse(path).getroot().iter("string"):
        text = "".join(node.itertext()).strip()
        if node.get("translatable") == "false" or not text:
            continue
        if "%" in text or "<" in text or len(list(node)):
            continue
        out[node.get("name")] = unescape(text)
    return out


def main():
    if not os.path.isdir(RES):
        sys.exit("reference/cloudstream-android is missing; cannot port translations")
    os.makedirs(OUT, exist_ok=True)
    source = read("values")
    english = {k: source[a] for k, a in MAPPING.items() if a in source}
    missing = [k for k, a in MAPPING.items() if a not in source]
    if missing:
        sys.exit(f"English source lacks: {missing}")
    with open(os.path.join(OUT, "en.android.json"), "w", encoding="utf-8") as handle:
        json.dump(english, handle, ensure_ascii=False, indent=2, sort_keys=True)
        handle.write("\n")
    written, skipped = [], []
    for folder, code in sorted(FOLDERS.items(), key=lambda kv: kv[1] or ""):
        if not code:
            continue
        strings = read(f"values-{folder}")
        ported = {k: strings[a] for k, a in MAPPING.items() if a in strings}
        coverage = len(ported) / len(MAPPING)
        if coverage < MIN_COVERAGE:
            skipped.append((code, f"{coverage:.0%}"))
            continue
        with open(os.path.join(OUT, f"{code}.android.json"), "w", encoding="utf-8") as handle:
            json.dump(ported, handle, ensure_ascii=False, indent=2, sort_keys=True)
            handle.write("\n")
        written.append((code, f"{coverage:.0%}"))
    print("Ported:", ", ".join(f"{c} ({cov})" for c, cov in written))
    if skipped:
        print("Skipped (too little coverage):", ", ".join(f"{c} ({cov})" for c, cov in skipped))


if __name__ == "__main__":
    main()
