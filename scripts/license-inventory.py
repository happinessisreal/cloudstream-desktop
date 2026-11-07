#!/usr/bin/env python3
"""Generate THIRD_PARTY_LICENSES.md from the real dependency data.

Sources: `cargo metadata` (Rust crates reachable from the desktop app, excluding dev-only ones),
package-lock.json (production npm packages) and the worker's resolved Gradle runtime classpath.
Gradle coordinates are matched to licences with a curated table; anything not in the table is
listed as UNVERIFIED instead of guessed.

  scripts/license-inventory.py            write THIRD_PARTY_LICENSES.md
  scripts/license-inventory.py --check    fail if the file is out of date or has unverified entries
"""
import json, os, re, subprocess, sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT = os.path.join(ROOT, "THIRD_PARTY_LICENSES.md")

# Licences for the worker's JVM dependencies, matched on `group:artifact` prefixes. Each was taken
# from the project's own published licence; update this table when the classpath changes.
GRADLE_LICENSES = [
    ("com.github.recloudstream.cloudstream:", "GPL-3.0-only", "https://github.com/recloudstream/cloudstream"),
    ("com.github.teamnewpipe:NewPipeExtractor", "GPL-3.0-or-later", "https://github.com/TeamNewPipe/NewPipeExtractor"),
    ("com.github.TeamNewPipe:nanojson", "Apache-2.0", "https://github.com/TeamNewPipe/nanojson"),
    ("com.github.Blatzar:NiceHttp", "MIT", "https://github.com/Blatzar/NiceHttp"),
    ("de.femtopedia.dex2jar:", "Apache-2.0", "https://github.com/ThexXTURBOXx/dex2jar"),
    ("org.jetbrains.kotlin:", "Apache-2.0", "https://kotlinlang.org/"),
    ("org.jetbrains.kotlinx:", "Apache-2.0", "https://github.com/Kotlin"),
    ("org.jetbrains:annotations", "Apache-2.0", "https://github.com/JetBrains/java-annotations"),
    ("com.fasterxml.jackson", "Apache-2.0", "https://github.com/FasterXML/jackson"),
    ("com.fleeksoft.ksoup:", "Apache-2.0", "https://github.com/fleeksoft/ksoup"),
    ("com.squareup.okhttp3:", "Apache-2.0", "https://square.github.io/okhttp/"),
    ("io.ktor:", "Apache-2.0", "https://ktor.io/"),
    ("dev.whyoleg.cryptography:", "Apache-2.0", "https://github.com/whyoleg/cryptography-kotlin"),
    ("org.jsoup:", "MIT", "https://jsoup.org/"),
    ("org.json:json", "JSON (public domain style; see upstream)", "https://github.com/stleary/JSON-java"),
    ("org.mozilla:rhino", "MPL-2.0", "https://github.com/mozilla/rhino"),
    ("org.ow2.asm:", "BSD-3-Clause", "https://asm.ow2.io/"),
    ("com.google.protobuf:", "BSD-3-Clause", "https://github.com/protocolbuffers/protobuf"),
    ("com.google.code.findbugs:jsr305", "Apache-2.0 (as published; see upstream)", "https://code.google.com/archive/p/jsr-305/"),
    ("androidx.annotation:", "Apache-2.0", "https://developer.android.com/jetpack/androidx"),
    # Read from each artifact's published POM in the local Gradle cache.
    ("co.touchlab:stately", "Apache-2.0", "https://github.com/touchlab/Stately"),
    ("com.fleeksoft.charset:", "Apache-2.0", "https://github.com/fleeksoft/charset"),
    ("com.fleeksoft.io:", "Apache-2.0", "https://github.com/fleeksoft/io"),
    ("com.squareup.okio:", "Apache-2.0", "https://square.github.io/okio/"),
    ("org.slf4j:", "MIT", "https://www.slf4j.org/license.html"),
]


def rust():
    meta = json.loads(subprocess.check_output(["cargo", "metadata", "--format-version", "1", "--locked"], cwd=ROOT))
    by_id = {p["id"]: p for p in meta["packages"]}
    nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
    workspace = set(meta["workspace_members"])
    seen, stack = set(), list(workspace)
    while stack:
        current = stack.pop()
        if current in seen:
            continue
        seen.add(current)
        for dep in nodes[current]["deps"]:
            # Keep normal and build dependencies; skip dependencies used only by tests.
            if any(k["kind"] in (None, "build") for k in dep["dep_kinds"]):
                stack.append(dep["pkg"])
    rows = []
    for pid in seen:
        if pid in workspace:
            continue
        p = by_id[pid]
        rows.append((p["name"], p["version"], p.get("license") or "UNVERIFIED (no licence field)", p.get("repository") or ""))
    return sorted(rows)


def npm():
    lock = json.load(open(os.path.join(ROOT, "package-lock.json")))
    rows = []
    for path, info in lock.get("packages", {}).items():
        if not path or info.get("dev") or info.get("peer") and info.get("optional"):
            continue
        name = path.split("node_modules/")[-1]
        license_ = info.get("license")
        if not license_:
            manifest = os.path.join(ROOT, path, "package.json")
            if os.path.isfile(manifest):
                license_ = json.load(open(manifest)).get("license")
        if isinstance(license_, dict):
            license_ = license_.get("type")
        rows.append((name, info.get("version", ""), license_ or "UNVERIFIED (no licence field)", info.get("resolved", "")))
    return sorted(set(rows))


def gradle():
    wrapper = os.path.join(ROOT, "runtimes", "jvm", "gradlew")
    if not os.path.isfile(wrapper):
        return [], "The Gradle wrapper is missing, so the worker's JVM dependencies were not listed."
    try:
        out = subprocess.run([wrapper, "--offline", "dependencies", "--configuration", "runtimeClasspath", "-q"], cwd=os.path.dirname(wrapper), capture_output=True, text=True, timeout=300).stdout
    except Exception as error:  # the listing is best effort; the caller reports it
        return [], f"The worker's JVM dependencies could not be listed ({error})."
    coordinates = set()
    for line in out.splitlines():
        match = re.search(r"[+\\|-]--- ([\w.\-]+:[\w.\-]+:[\w.\-]+)(?: -> ([\w.\-]+))?", line)
        if match:
            group_artifact_version, resolved = match.groups()
            group, artifact, version = group_artifact_version.split(":")
            coordinates.add((group, artifact, resolved or version))
    rows = []
    for group, artifact, version in sorted(coordinates):
        key = f"{group}:{artifact}"
        license_, url = next(((l, u) for prefix, l, u in GRADLE_LICENSES if key.startswith(prefix)), ("UNVERIFIED (not in the licence table)", ""))
        rows.append((key, version, license_, url))
    return rows, None


def table(title, rows, columns):
    lines = [f"## {title}", "", "| " + " | ".join(columns) + " |", "|" + "---|" * len(columns)]
    for row in rows:
        lines.append("| " + " | ".join(str(c).replace("|", "\\|") for c in row) + " |")
    return "\n".join(lines) + "\n"


def build():
    rust_rows, npm_rows = rust(), npm()
    gradle_rows, gradle_note = gradle()
    parts = [
        "# Third-party licences\n",
        "Generated by `scripts/license-inventory.py` from `Cargo.lock`, `package-lock.json` and the worker's resolved Gradle runtime classpath. Do not edit by hand; run the script again after changing dependencies. See `NOTICE.md` for the project's own notices and `LICENSE` for the GPL-3.0 text.\n",
        "Entries marked UNVERIFIED have no machine-readable licence and must be reviewed before a release.\n",
        "## Not covered by the lists below\n\n"
        "- The **Java runtime** shipped with a package (OpenJDK, GPL-2.0 with the Classpath Exception). Its `legal/` folder must be included by whoever builds the runtime.\n"
        "- **mpv** and **libnotify** are system packages the app depends on; they are not bundled.\n"
        "- Cloudstream Android launcher artwork adapted for the icon (GPL-3.0) and the Blender Foundation trailers linked by the sample extension (CC-BY-3.0); see `NOTICE.md`.\n",
        table("Rust crates", rust_rows, ["Crate", "Version", "Licence", "Source"]),
        table("Frontend packages (production)", npm_rows, ["Package", "Version", "Licence", "Source"]),
    ]
    if gradle_note:
        parts.append(f"## Worker (JVM) dependencies\n\n{gradle_note}\n")
    else:
        parts.append(table("Worker (JVM) dependencies", gradle_rows, ["Artifact", "Version", "Licence", "Project"]))
    text = "\n".join(parts)
    unverified = [r[0] for r in rust_rows + npm_rows + gradle_rows if str(r[2]).startswith("UNVERIFIED")]
    return text, unverified, bool(gradle_note)


if __name__ == "__main__":
    text, unverified, gradle_missing = build()
    if "--check" in sys.argv:
        current = open(OUT).read() if os.path.isfile(OUT) else ""
        problems = []
        if current != text:
            problems.append("THIRD_PARTY_LICENSES.md is out of date; run scripts/license-inventory.py")
        if unverified:
            problems.append("Unverified licences: " + ", ".join(unverified))
        if gradle_missing:
            problems.append("The worker dependencies could not be listed")
        if problems:
            print("\n".join(problems))
            sys.exit(1)
        print("Licence inventory is current and fully verified.")
    else:
        open(OUT, "w").write(text)
        print(f"Wrote {OUT}: {text.count(chr(10))} lines, {len(unverified)} unverified entries")
        for name in unverified:
            print("  UNVERIFIED:", name)
