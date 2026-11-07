@file:OptIn(com.lagradost.cloudstream3.InternalAPI::class, com.lagradost.cloudstream3.Prerelease::class)
package dev.cloudstream.worker

import com.lagradost.cloudstream3.*
import com.lagradost.cloudstream3.utils.*
import kotlinx.coroutines.runBlocking
import java.util.UUID
import kotlin.test.*

class MappingTest {

    private class TestProvider : MainAPI() {
        override var name = "Test Provider"
        override var mainUrl = "https://example.com"
        override var lang = "en"
        override val hasMainPage = true
        override val supportedTypes = setOf(TvType.Movie, TvType.TvSeries, TvType.Anime)
        override var hasQuickSearch = true
        override var hasDownloadSupport = true
        override var hasChromecastSupport = false
        override val searchTimeoutMs = 12_000L
        override val loadTimeoutMs = 25_000L
    }

    @Test
    fun mapsProviderCapabilitiesAndTimeouts() {
        val api = TestProvider()
        api.sourcePlugin = "test-plugin"
        val mapped = mapProvider(api)

        assertEquals("test-plugin:Test Provider", mapped["id"])
        assertEquals("test-plugin", mapped["pluginId"])
        assertEquals("Test Provider", mapped["name"])
        assertEquals(true, mapped["hasMainPage"])

        @Suppress("UNCHECKED_CAST")
        val capabilities = mapped["capabilities"] as Map<String, Any?>
        assertEquals(true, capabilities["hasMainPage"])
        assertEquals(true, capabilities["hasQuickSearch"])
        assertEquals(true, capabilities["hasDownloadSupport"])
        assertEquals(false, capabilities["hasChromecastSupport"])
        assertEquals(false, capabilities["usesWebView"])

        @Suppress("UNCHECKED_CAST")
        val timeouts = mapped["timeouts"] as Map<String, Any?>
        assertEquals(12_000L, timeouts["searchTimeoutMs"])
        assertEquals(25_000L, timeouts["loadTimeoutMs"])
    }

    @Test
    fun mapsAnimeSearchResponseWithDubAndScore() {
        val api = TestProvider()
        api.sourcePlugin = "test-plugin"

        val searchResponse = api.newAnimeSearchResponse(
            name = "Attack on Titan",
            url = "https://example.com/aot",
            type = TvType.Anime
        ) {
            posterUrl = "https://example.com/aot.jpg"
            year = 2013
            dubStatus = mutableSetOf(DubStatus.Dubbed, DubStatus.Subbed)
            otherName = "Shingeki no Kyojin"
            episodes = mutableMapOf(DubStatus.Dubbed to 25, DubStatus.Subbed to 25)
            id = 101
            quality = SearchQuality.HD
            posterHeaders = mapOf("Referer" to "https://example.com")
            score = Score.from10(9.0)
        }

        val mapped = mapSearchItem(api, searchResponse)
        assertEquals("Attack on Titan", mapped["name"])
        assertEquals(2013, mapped["year"])
        assertEquals("HD", mapped["quality"])
        assertEquals("both", mapped["dubStatus"])
        assertEquals("Shingeki no Kyojin", mapped["otherName"])
        assertEquals(101, mapped["id"])

        @Suppress("UNCHECKED_CAST")
        val dubEpisodes = mapped["dubEpisodes"] as Map<String, Int>
        assertEquals(25, dubEpisodes["dubbed"])
        assertEquals(25, dubEpisodes["subbed"])
        assertNotNull(mapped["score"])
    }

    @Test
    fun mapsDetailsWithRichMetadataAndStructuredDubGroups() = runBlocking {
        val api = TestProvider()
        api.sourcePlugin = "test-plugin"

        val ep1 = api.newEpisode("https://example.com/watch/ep1") {
            name = "To You, in 2000 Years"
            season = 1
            episode = 1
            posterUrl = "https://example.com/ep1.jpg"
            score = Score.from10(9.5)
            description = "The citizens of Paradis are attacked."
            date = 1365292800000L
            runTime = 24
        }
        val ep1Dub = api.newEpisode("https://example.com/watch/ep1-dub") {
            name = "To You, in 2000 Years"
            season = 1
            episode = 1
            posterUrl = "https://example.com/ep1.jpg"
            score = Score.from10(9.5)
            description = "The citizens of Paradis are attacked."
            date = 1365292800000L
            runTime = 24
        }

        val load = api.newAnimeLoadResponse("Attack on Titan", "https://example.com/aot", TvType.Anime) {
            posterUrl = "https://example.com/aot.jpg"
            year = 2013
            plot = "Humanity lives behind walls."
            tags = listOf("Action", "Fantasy")
            score = Score.from10(9.1)
            trailers = mutableListOf(TrailerData("https://example.com/trailer.mp4", "https://example.com", false, emptyMap()))
            actors = listOf(ActorData(Actor("Yuki Kaji", "https://example.com/kaji.jpg"), ActorRole.Main, "Eren Yeager", null))
            syncData = mutableMapOf("mal" to "16498", "anilist" to "16498")
            uniqueUrl = "https://example.com/unique-aot"
            backgroundPosterUrl = "https://example.com/aot-bg.jpg"
            logoUrl = "https://example.com/aot-logo.png"
            contentRating = "TV-MA"
            addEpisodes(DubStatus.Subbed, listOf(ep1))
            addEpisodes(DubStatus.Dubbed, listOf(ep1Dub))
        }

        val mapped = mapDetails(api, load)
        assertEquals("https://example.com/unique-aot", mapped["uniqueUrl"])
        assertEquals("https://example.com/aot-bg.jpg", mapped["backgroundPosterUrl"])
        assertEquals("https://example.com/aot-logo.png", mapped["logoUrl"])
        assertEquals("TV-MA", mapped["contentRating"])

        @Suppress("UNCHECKED_CAST")
        val episodes = mapped["episodes"] as List<Map<String, Any?>>
        assertEquals(2, episodes.size)
        // Ensure episode name does not have translated prefix prepended:
        assertEquals("To You, in 2000 Years", episodes[0]["name"])
        assertEquals("subbed", episodes[0]["dubStatus"])
        assertEquals("dubbed", episodes[1]["dubStatus"])

        @Suppress("UNCHECKED_CAST")
        val dubGroups = mapped["dubGroups"] as Map<String, List<Map<String, Any?>>>
        assertTrue(dubGroups.containsKey("subbed"))
        assertTrue(dubGroups.containsKey("dubbed"))
        assertEquals(1, dubGroups["subbed"]?.size)
        assertEquals(1, dubGroups["dubbed"]?.size)

        @Suppress("UNCHECKED_CAST")
        val trailers = mapped["trailers"] as List<Map<String, Any?>>
        assertEquals(1, trailers.size)
        assertEquals("https://example.com/trailer.mp4", trailers[0]["extractorUrl"])

        @Suppress("UNCHECKED_CAST")
        val syncData = mapped["syncData"] as Map<String, String>
        assertEquals("16498", syncData["mal"])
    }

    @Test
    fun mapsDiscriminatedStreamsAndSubtitles() = runBlocking {
        val audioTrack = newAudioFile("https://example.com/audio.aac")
        val directLink = ExtractorLink(
            source = "DirectSource",
            name = "1080p MP4",
            url = "https://example.com/video.mp4",
            referer = "https://example.com",
            quality = 1080,
            headers = mapOf("User-Agent" to "TestAgent"),
            extractorData = "token123",
            type = ExtractorLinkType.VIDEO,
            audioTracks = listOf(audioTrack)
        )
        val mappedDirect = mapStream(directLink)
        assertEquals("direct", mappedDirect["kind"])
        assertEquals("VIDEO", mappedDirect["streamType"])
        assertEquals(1080, mappedDirect["quality"])
        assertEquals("token123", mappedDirect["extractorData"])

        @Suppress("UNCHECKED_CAST")
        val audioTracks = mappedDirect["audioTracks"] as List<Map<String, Any?>>
        assertEquals(1, audioTracks.size)
        assertEquals("https://example.com/audio.aac", audioTracks[0]["url"])

        val hlsLink = ExtractorLink(
            source = "HlsSource",
            name = "HLS Stream",
            url = "https://example.com/index.m3u8",
            referer = "https://example.com",
            quality = 720,
            headers = emptyMap(),
            extractorData = null,
            type = ExtractorLinkType.M3U8,
            audioTracks = emptyList()
        )
        val mappedHls = mapStream(hlsLink)
        assertEquals("hls", mappedHls["kind"])
        assertEquals("M3U8", mappedHls["streamType"])

        val drmUuid = UUID.randomUUID()
        val drmLink = newDrmExtractorLink(
            source = "DrmSource",
            name = "Encrypted Stream",
            url = "https://example.com/stream.mpd",
            type = ExtractorLinkType.DASH,
            uuid = drmUuid
        ) {
            referer = "https://example.com"
            quality = 1080
            kid = "kid-abc"
            key = "key-value"
            licenseUrl = "https://example.com/license"
        }
        val mappedDrm = mapStream(drmLink)
        assertEquals("drm", mappedDrm["kind"])
        assertEquals("DRM", mappedDrm["streamType"])
        assertEquals("DASH", mappedDrm["linkType"], "the container behind the DRM link is kept")
        assertEquals("M3U8", mappedHls["linkType"])

        @Suppress("UNCHECKED_CAST")
        val drm = mappedDrm["drm"] as Map<String, Any?>
        assertEquals(drmUuid.toString(), drm["uuid"])
        assertEquals("kid-abc", drm["kid"])
        assertEquals("key-value", drm["key"])
        assertEquals("https://example.com/license", drm["licenseUrl"])

        @Suppress("DEPRECATION")
        val sub = SubtitleFile("English", "https://example.com/sub.vtt")
        sub.headers = mapOf("Accept" to "text/vtt")
        val mappedSub = mapSubtitle(sub)
        assertEquals("English", mappedSub["lang"])
        assertEquals("https://example.com/sub.vtt", mappedSub["url"])
        assertEquals("text/vtt", (mappedSub["headers"] as Map<*, *>)["Accept"])
    }
}
