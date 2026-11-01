# Notices

This independent prototype uses Cloudstream's GPL-3.0 library, pinned to commit `5024ace2922a148d38df2a816b483b529789cf58`. Original project: https://github.com/recloudstream/cloudstream. Corresponding library source: https://github.com/recloudstream/cloudstream/tree/5024ace2922a148d38df2a816b483b529789cf58/library. The original project's license is included in `LICENSE`. The Gradle wrapper is taken from the same repository and is governed by Gradle's applicable license.

The worker includes Kotlin, kotlinx libraries, Jackson, NiceHttp, jsoup, Ksoup, Ktor, Rhino, ASM, NewPipeExtractor, cryptography providers, dex2jar (Apache-2.0, https://github.com/pxb1988/dex2jar and the de.femtopedia distribution), and their transitive dependencies. The application uses Tauri, Svelte, Lucide, Roboto and Rust dependencies recorded in the lock files. Preserve the license and notice files supplied by those dependencies. This file is an initial development inventory, not a completed distribution-license audit.

The generated Java runtime includes its `legal/` directory. OpenJDK source and licensing information are available from https://openjdk.org/ and the JDK distributor used to create that runtime. Runtime selection is a build-time choice, and redistributors must supply the corresponding notices and source as required by that distribution.

The bundled sample extension contains no video files. It links to official Blender Foundation trailers for **Big Buck Bunny** (2008, https://peach.blender.org/) and **Sintel** (2010, https://durian.blender.org/), distributed under Creative Commons Attribution 3.0. Credit: Blender Foundation and the respective open movie teams. Media is fetched only when the user plays it.

The desktop cloud symbol and icon are adapted from Cloudstream’s GPL-3.0 Android launcher vector. The desktop palette and layouts take influence from the upstream Android app. No affiliation or endorsement by the upstream project is implied.
