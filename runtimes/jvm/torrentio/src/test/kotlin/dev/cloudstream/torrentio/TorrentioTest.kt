package dev.cloudstream.torrentio

import com.lagradost.cloudstream3.utils.ExtractorLink
import kotlinx.coroutines.runBlocking
import java.io.File
import java.net.URLDecoder
import kotlin.io.path.createTempDirectory
import kotlin.test.*

class TorrentioTest {

    @Test
    fun verifyEffectiveTrackersLoadedFromFileWithDeduplication() {
        val tempDir = createTempDirectory("torrentio_test").toFile()
        val trackerFile = File(tempDir, "trackers.txt")
        try {
            // Write custom trackers with duplicates, case variations, trailing slashes, and comments
            trackerFile.writeText(
                """
                # Custom BitTorrent Trackers
                udp://custom-tracker1.example.org:1337/announce
                UDP://custom-tracker1.example.org:1337/announce/
                   udp://custom-tracker2.example.org:6969/announce   
                
                # Preset duplicate that should not appear twice
                UDP://TRACKER.OPENTRACKR.ORG:1337/announce/
                """.trimIndent()
            )

            System.setProperty("CLOUDSTREAM_TRACKERS_FILE", trackerFile.absolutePath)

            val torrentio = Torrentio()
            val trackers = torrentio.getEffectiveTrackers()

            // 1. Check custom tracker is loaded
            assertTrue(
                trackers.any { it.contains("custom-tracker1.example.org") },
                "Expected custom-tracker1 to be loaded from file, got: $trackers"
            )
            assertTrue(
                trackers.any { it.contains("custom-tracker2.example.org") },
                "Expected custom-tracker2 to be loaded from file, got: $trackers"
            )

            // 2. Check no duplicates (case-insensitive canonical check)
            val lowerTrackers = trackers.map { it.lowercase().removeSuffix("/") }
            assertEquals(lowerTrackers.distinct().size, lowerTrackers.size, "Expected zero duplicates in loaded trackers")

            // 3. Check opentrackr is present exactly once
            val opentrackrCount = trackers.count { it.lowercase().contains("opentrackr.org") }
            assertEquals(1, opentrackrCount, "Expected opentrackr to be present exactly once without duplicate")
        } finally {
            tempDir.deleteRecursively()
            System.clearProperty("CLOUDSTREAM_TRACKERS_FILE")
        }
    }

    @Test
    fun verifyLoadLinksEmbedsLoadedTrackersIntoMagnet() = runBlocking {
        val tempDir = createTempDirectory("torrentio_magnet_test").toFile()
        val trackerFile = File(tempDir, "trackers.txt")
        try {
            val customTracker = "udp://tracker.verify-load.org:8080/announce"
            trackerFile.writeText(customTracker)

            System.setProperty("CLOUDSTREAM_TRACKERS_FILE", trackerFile.absolutePath)

            val torrentio = Torrentio()
            val emittedLinks = mutableListOf<ExtractorLink>()

            // Load links for Big Buck Bunny (local item tt1254207)
            val found = torrentio.loadLinks(
                data = "https://torrentio.strem.fun/movie/tt1254207",
                isCasting = false,
                subtitleCallback = {},
                callback = { link -> emittedLinks.add(link) }
            )

            assertTrue(found, "loadLinks must return true")
            assertTrue(emittedLinks.isNotEmpty(), "At least one stream link must be produced")

            val magnet = emittedLinks.first { it.url.startsWith("magnet:?") }.url
            val decodedMagnet = URLDecoder.decode(magnet, "UTF-8")

            // Verify custom tracker actually got loaded into the magnet link
            assertTrue(
                decodedMagnet.contains(customTracker),
                "Magnet link must contain custom loaded tracker '$customTracker'. Magnet: $decodedMagnet"
            )

            // Verify no duplicate &tr= entries
            val trParams = magnet.split("&tr=").drop(1).map { URLDecoder.decode(it.substringBefore("&"), "UTF-8") }
            assertEquals(trParams.distinct().size, trParams.size, "Magnet link must not contain duplicate trackers")
        } finally {
            tempDir.deleteRecursively()
            System.clearProperty("CLOUDSTREAM_TRACKERS_FILE")
        }
    }

    @Test
    fun verifyLetterboxdAndCinemetaSectionsExist() {
        val torrentio = Torrentio()
        val sections = torrentio.mainPage.map { it.name }
        assertTrue(sections.any { it.contains("Cinemeta") }, "Expected Cinemeta in main page")
        assertTrue(sections.any { it.contains("Letterboxd") }, "Expected Letterboxd in main page")
    }
}
