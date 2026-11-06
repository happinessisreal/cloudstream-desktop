package dev.cloudstream.worker

import com.lagradost.cloudstream3.MainAPI
import com.lagradost.cloudstream3.SubtitleFile
import com.lagradost.cloudstream3.TvType
import com.lagradost.cloudstream3.utils.ExtractorLink
import com.lagradost.cloudstream3.utils.ExtractorLinkType
import com.sun.net.httpserver.HttpExchange
import com.sun.net.httpserver.HttpServer
import dev.cloudstream.worker.transport.MediaTransportService
import dev.cloudstream.worker.webview.DefaultWebViewResolverBridge
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import okhttp3.OkHttpClient
import okhttp3.Request
import org.junit.jupiter.api.AfterAll
import org.junit.jupiter.api.BeforeAll
import org.junit.jupiter.api.Test
import java.net.InetAddress
import java.net.InetSocketAddress
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicInteger
import kotlin.test.*

class MediaTransportTest {
    companion object {
        private var mockServer: HttpServer? = null
        private var mockPort: Int = 0
        private val client = OkHttpClient()

        @JvmStatic
        @BeforeAll
        fun setUp() {
            MediaTransportService.start()
            // Start a local mock upstream server on loopback
            val s = HttpServer.create(InetSocketAddress(InetAddress.getByName("127.0.0.1"), 0), 0)
            s.createContext("/video.mp4") { exchange: HttpExchange ->
                val range = exchange.requestHeaders.getFirst("Range")
                val fullBytes = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ".toByteArray(Charsets.UTF_8)
                if (range != null && range.startsWith("bytes=")) {
                    // e.g. bytes=0-4
                    val parts = range.removePrefix("bytes=").split('-')
                    val start = parts[0].toInt()
                    val end = if (parts.size > 1 && parts[1].isNotEmpty()) parts[1].toInt() else fullBytes.size - 1
                    val slice = fullBytes.sliceArray(start..end)
                    exchange.responseHeaders.set("Content-Type", "video/mp4")
                    exchange.responseHeaders.set("Content-Range", "bytes $start-$end/${fullBytes.size}")
                    exchange.responseHeaders.set("Accept-Ranges", "bytes")
                    exchange.sendResponseHeaders(206, slice.size.toLong())
                    exchange.responseBody.use { it.write(slice) }
                } else {
                    exchange.responseHeaders.set("Content-Type", "video/mp4")
                    exchange.responseHeaders.set("Content-Length", fullBytes.size.toString())
                    exchange.sendResponseHeaders(200, fullBytes.size.toLong())
                    exchange.responseBody.use { it.write(fullBytes) }
                }
            }

            s.createContext("/playlist.m3u8") { exchange: HttpExchange ->
                val m3u8 = """
                    #EXTM3U
                    #EXT-X-VERSION:3
                    #EXT-X-TARGETDURATION:10
                    #EXT-X-KEY:METHOD=AES-128,URI="https://127.0.0.1:$mockPort/key.bin"
                    #EXTINF:10.0,
                    segment1.ts
                    #EXTINF:10.0,
                    https://127.0.0.1:$mockPort/segment2.ts
                    #EXT-X-ENDLIST
                """.trimIndent()
                val bytes = m3u8.toByteArray(Charsets.UTF_8)
                exchange.responseHeaders.set("Content-Type", "application/vnd.apple.mpegurl")
                exchange.sendResponseHeaders(200, bytes.size.toLong())
                exchange.responseBody.use { it.write(bytes) }
            }

            s.createContext("/redirect_safe") { exchange: HttpExchange ->
                exchange.responseHeaders.set("Location", "http://127.0.0.1:$mockPort/video.mp4")
                exchange.sendResponseHeaders(302, -1)
                exchange.responseBody.close()
            }

            s.createContext("/redirect_evil") { exchange: HttpExchange ->
                exchange.responseHeaders.set("Location", "http://unrelated-evil.com/video.mp4")
                exchange.sendResponseHeaders(302, -1)
                exchange.responseBody.close()
            }

            s.start()
            mockServer = s
            mockPort = s.address.port
        }

        @JvmStatic
        @AfterAll
        fun tearDown() {
            mockServer?.stop(0)
            MediaTransportService.stop()
        }
    }

    private class TestProvider(
        val verifierStarted: AtomicBoolean,
        val verifierCancelled: AtomicBoolean
    ) : MainAPI() {
        override var name = "Test Transport Provider"
        override var mainUrl = "http://127.0.0.1:$mockPort"
        override var supportedTypes = setOf(TvType.Movie)

        override suspend fun extractorVerifierJob(extractorData: String?) {
            verifierStarted.set(true)
            try {
                while (true) {
                    delay(100)
                }
            } finally {
                verifierCancelled.set(true)
            }
        }
    }

    @Test
    fun `media transport runs on ephemeral loopback port only`() {
        assertTrue(MediaTransportService.port > 0)
        assertTrue(MediaTransportService.port < 65536)
    }

    @Test
    fun `token authentication and range header preservation`() {
        val link = ExtractorLink(
            source = "Test",
            name = "Test Video",
            url = "http://127.0.0.1:$mockPort/video.mp4",
            referer = "",
            quality = 1080,
            type = ExtractorLinkType.VIDEO
        )
        val provider = TestProvider(AtomicBoolean(), AtomicBoolean())
        val resourceId = MediaTransportService.registerResource(provider.name, link, provider)

        val sessionInfo = MediaTransportService.createSession(resourceId, CoroutineScope(Dispatchers.Default)) { provider }
        val token = sessionInfo["sessionToken"] as String
        val transportUrl = sessionInfo["transportUrl"] as String

        // 1. Fetch with Range header
        val req = Request.Builder()
            .url(transportUrl)
            .header("Range", "bytes=0-4")
            .build()
        val resp = client.newCall(req).execute()
        assertEquals(206, resp.code)
        assertEquals("video/mp4", resp.header("Content-Type"))
        assertEquals("bytes 0-4/36", resp.header("Content-Range"))
        val bodyBytes = resp.body?.bytes()
        assertNotNull(bodyBytes)
        assertEquals("01234", String(bodyBytes, Charsets.UTF_8))

        // 2. Access with invalid token should fail closed with 403 Forbidden
        val badUrl = transportUrl.replace(token, "00000000000000000000000000000000")
        val badResp = client.newCall(Request.Builder().url(badUrl).build()).execute()
        assertEquals(403, badResp.code)

        // 3. Close session
        assertTrue(MediaTransportService.closeSession(token))
        val closedResp = client.newCall(Request.Builder().url(transportUrl).build()).execute()
        assertEquals(403, closedResp.code)
    }

    @Test
    fun `hls manifest rewriting rewrites segments and keys with session token`() {
        val link = ExtractorLink(
            source = "Test",
            name = "Test HLS",
            url = "http://127.0.0.1:$mockPort/playlist.m3u8",
            referer = "",
            quality = 1080,
            type = ExtractorLinkType.M3U8
        )
        val provider = TestProvider(AtomicBoolean(), AtomicBoolean())
        val resourceId = MediaTransportService.registerResource(provider.name, link, provider)

        val sessionInfo = MediaTransportService.createSession(resourceId, CoroutineScope(Dispatchers.Default)) { provider }
        val token = sessionInfo["sessionToken"] as String
        val transportUrl = sessionInfo["transportUrl"] as String

        val resp = client.newCall(Request.Builder().url(transportUrl).build()).execute()
        assertEquals(200, resp.code)
        val manifest = resp.body?.string().orEmpty()

        // Key URI should be rewritten through loopback proxy with session token
        assertTrue(manifest.contains("""URI="http://127.0.0.1:${MediaTransportService.port}/media/$token/$resourceId?subpath="""), "Key URI must be rewritten through proxy")
        // Relative segment1.ts should be rewritten
        assertTrue(manifest.contains("subpath=http%3A%2F%2F127.0.0.1%3A$mockPort%2Fsegment1.ts"), "Relative segment must be rewritten")
        // Absolute segment2.ts should be rewritten
        assertTrue(manifest.contains("subpath=https%3A%2F%2F127.0.0.1%3A$mockPort%2Fsegment2.ts"), "Absolute segment must be rewritten")

        MediaTransportService.closeSession(token)
    }

    @Test
    fun `redirect to unrelated origin fails closed with 502`() {
        val linkSafe = ExtractorLink(
            source = "Test",
            name = "Safe Redirect",
            url = "http://127.0.0.1:$mockPort/redirect_safe",
            referer = "",
            quality = 1080,
            type = ExtractorLinkType.VIDEO
        )
        val linkEvil = ExtractorLink(
            source = "Test",
            name = "Evil Redirect",
            url = "http://127.0.0.1:$mockPort/redirect_evil",
            referer = "",
            quality = 1080,
            type = ExtractorLinkType.VIDEO
        )
        val provider = TestProvider(AtomicBoolean(), AtomicBoolean())

        val resSafe = MediaTransportService.registerResource(provider.name, linkSafe, provider)
        val sessionSafe = MediaTransportService.createSession(resSafe, CoroutineScope(Dispatchers.Default)) { provider }
        val respSafe = client.newCall(Request.Builder().url(sessionSafe["transportUrl"] as String).build()).execute()
        assertEquals(200, respSafe.code)

        val resEvil = MediaTransportService.registerResource(provider.name, linkEvil, provider)
        val sessionEvil = MediaTransportService.createSession(resEvil, CoroutineScope(Dispatchers.Default)) { provider }
        val respEvil = client.newCall(Request.Builder().url(sessionEvil["transportUrl"] as String).build()).execute()
        assertEquals(502, respEvil.code)
        val evilBody = respEvil.body?.string().orEmpty()
        assertTrue(evilBody.contains("Redirect to unrelated origin rejected"))

        MediaTransportService.closeSession(sessionSafe["sessionToken"] as String)
        MediaTransportService.closeSession(sessionEvil["sessionToken"] as String)
    }

    @Test
    fun `verifier lifecycle starts on session create and cancels on session close`() {
        val started = AtomicBoolean(false)
        val cancelled = AtomicBoolean(false)
        val provider = TestProvider(started, cancelled)

        val link = ExtractorLink(
            source = "Test",
            name = "Verifier Video",
            url = "http://127.0.0.1:$mockPort/video.mp4",
            referer = "",
            quality = 1080,
            extractorData = "sample_opaque_verifier_data",
            type = ExtractorLinkType.VIDEO
        )
        val resId = MediaTransportService.registerResource(provider.name, link, provider)
        val session = MediaTransportService.createSession(resId, CoroutineScope(Dispatchers.Default)) { provider }
        val token = session["sessionToken"] as String

        Thread.sleep(150)
        assertTrue(started.get(), "Extractor verifier job must have started")
        assertTrue(MediaTransportService.isVerifierActive(token), "Verifier job must be active")

        MediaTransportService.closeSession(token)
        Thread.sleep(150)
        assertTrue(cancelled.get(), "Extractor verifier job must have been cancelled")
    }

    @Test
    fun `webview bridge returns structured unsupported report`() {
        assertFalse(DefaultWebViewResolverBridge.isSupported())
        val report = DefaultWebViewResolverBridge.getUnsupportedReport()
        assertEquals(false, report["supported"])
        assertEquals("webview_unsupported", report["reason"])
        assertNotNull(report["missingFeatures"])
    }
}
