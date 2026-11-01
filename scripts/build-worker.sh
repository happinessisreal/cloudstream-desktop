#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
if [[ -z "${JAVA_HOME:-}" && -d /usr/lib/jvm/java-21-openjdk ]]; then
  export JAVA_HOME=/usr/lib/jvm/java-21-openjdk
fi
(cd runtimes/jvm && ./gradlew --no-daemon jar :sample:jar)
mkdir -p src-tauri/resources/worker
cp runtimes/jvm/build/libs/worker.jar src-tauri/resources/worker/worker.jar
cp runtimes/jvm/sample/build/libs/desktop-sample.jar src-tauri/resources/worker/desktop-sample.jar
if [[ ! -x src-tauri/resources/jre/bin/java ]]; then
  "${JAVA_HOME:?Set JAVA_HOME to a Java 21 JDK}/bin/jlink" --add-modules ALL-MODULE-PATH --strip-debug --no-header-files --no-man-pages --output src-tauri/resources/jre
fi
# jlink copies read-only license files; make this owned build output replaceable
# so Tauri can update its resource copies on subsequent builds.
find src-tauri/resources/jre -type f -exec chmod u+w {} +
printf 'Extension worker and bundled Java runtime are ready.\n'
