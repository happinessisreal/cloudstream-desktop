package dev.cloudstream.worker

import com.lagradost.cloudstream3.MainAPI
import com.lagradost.cloudstream3.SubtitleFile
import com.lagradost.cloudstream3.TvType
import com.lagradost.cloudstream3.utils.ExtractorLink
import com.lagradost.cloudstream3.utils.ExtractorLinkPlayList
import com.lagradost.cloudstream3.utils.PlayListItem
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
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.util.concurrent.TimeUnit
import org.junit.jupiter.api.Assumptions.assumeTrue
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicInteger
import kotlin.test.*

class MediaTransportTest {
    companion object {
        private var mockServer: HttpServer? = null
        private var mockPort: Int = 0
        private val client = OkHttpClient()
        private val protectedWaveRequests = arrayOf(AtomicInteger(), AtomicInteger())
        private val lastEncoding = java.util.concurrent.atomic.AtomicReference<String?>()

        private fun silentWave(): ByteArray {
            val pcmSize = 8_000 * 2
            val b = ByteBuffer.allocate(44 + pcmSize).order(ByteOrder.LITTLE_ENDIAN)
            b.put("RIFF".toByteArray()).putInt(36 + pcmSize).put("WAVEfmt ".toByteArray())
            b.putInt(16).putShort(1).putShort(1).putInt(8_000).putInt(16_000).putShort(2).putShort(16)
            b.put("data".toByteArray()).putInt(pcmSize)
            return b.array()
        }

        @JvmStatic
        @BeforeAll
        fun setUp() {
            MediaTransportService.start()
            // Start a local mock upstream server on loopback
            val s = HttpServer.create(InetSocketAddress(InetAddress.getByName("127.0.0.1"), 0), 0)
            s.createContext("/video.mp4") { exchange: HttpExchange ->
                val range = exchange.requestHeaders.getFirst("Range")
                lastEncoding.set(exchange.requestHeaders.getFirst("Accept-Encoding"))
                val fullBytes = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ".toByteArray(Charsets.UTF_8)
                val ifRange = exchange.requestHeaders.getFirst("If-Range")
                exchange.responseHeaders.set("ETag", "\"fixture-v1\"")
                exchange.responseHeaders.set("Last-Modified", "Wed, 21 Oct 2015 07:28:00 GMT")
                if (exchange.requestMethod == "HEAD") {
                    exchange.responseHeaders.set("Content-Length", fullBytes.size.toString())
                    exchange.sendResponseHeaders(200, -1)
                    exchange.close()
                    return@createContext
                }
                if (range != null && range.startsWith("bytes=") && (ifRange == null || ifRange == "\"fixture-v1\"")) {
                    // e.g. bytes=0-4
                    val parts = range.removePrefix("bytes=").split('-')
                    val start = parts[0].toInt()
                    if (start >= fullBytes.size) {
                        exchange.responseHeaders.set("Content-Range", "bytes */${fullBytes.size}")
                        exchange.sendResponseHeaders(416, -1)
                        exchange.close()
                        return@createContext
                    }
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

            s.createContext("/protected") { exchange: HttpExchange ->
                val valid = exchange.requestHeaders.getFirst("X-Provider") == "secret" &&
                    exchange.requestHeaders.getFirst("Range") == "bytes=0-2"
                val bytes = exchange.requestURI.rawQuery.toByteArray()
                exchange.sendResponseHeaders(if (valid) 206 else 403, bytes.size.toLong())
                exchange.responseBody.use { it.write(bytes) }
            }

            s.createContext("/protected-wave") { exchange: HttpExchange ->
                if (exchange.requestHeaders.getFirst("X-Provider") != "secret") {
                    exchange.sendResponseHeaders(403, -1)
                    exchange.close()
                } else {
                    val index = if (exchange.requestURI.rawQuery == "part=1") 0 else 1
                    protectedWaveRequests[index].incrementAndGet()
                    val bytes = silentWave()
                    val start = exchange.requestHeaders.getFirst("Range")?.substringAfter("bytes=")
                        ?.substringBefore('-')?.toIntOrNull() ?: 0
                    val data = bytes.copyOfRange(start.coerceIn(0, bytes.size), bytes.size)
                    exchange.responseHeaders.set("Content-Type", "audio/wav")
                    exchange.responseHeaders.set("Accept-Ranges", "bytes")
                    if (exchange.requestHeaders.getFirst("Range") != null) {
                        exchange.responseHeaders.set("Content-Range", "bytes $start-${bytes.size - 1}/${bytes.size}")
                    }
                    exchange.sendResponseHeaders(if (exchange.requestHeaders.getFirst("Range") != null) 206 else 200, data.size.toLong())
                    exchange.responseBody.use { it.write(data) }
                }
            }

            s.createContext("/playlist.m3u8") { exchange: HttpExchange ->
                val m3u8 = """
                    #EXTM3U
                    #EXT-X-VERSION:3
                    #EXT-X-TARGETDURATION:10
                    #EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID="audio",NAME="English",URI="audio.m3u8"
                    #EXT-X-MAP:URI="init.mp4",BYTERANGE="4@0"
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

            listOf("/audio.m3u8", "/init.mp4").forEach { path ->
                s.createContext(path) { exchange: HttpExchange ->
                    if (exchange.requestHeaders.getFirst("X-Hls-Fixture") != "secret") {
                        exchange.sendResponseHeaders(403, -1)
                        exchange.close()
                    } else {
                        val bytes = if (path.endsWith("m3u8")) "#EXTM3U\n#EXT-X-ENDLIST\n".toByteArray() else "init".toByteArray()
                        exchange.sendResponseHeaders(200, bytes.size.toLong())
                        exchange.responseBody.use { it.write(bytes) }
                    }
                }
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
    fun `direct sessions preserve range validators and identity encoding`() {
        val provider = TestProvider(AtomicBoolean(), AtomicBoolean())
        val resource = MediaTransportService.registerRawResource(provider.name, "http://127.0.0.1:$mockPort/video.mp4", mapOf("If-Range" to "\"stale-provider-header\""))
        val session = MediaTransportService.createSession(resource, CoroutineScope(Dispatchers.Default)) { provider }
        val url = session["transportUrl"] as String
        try {
            client.newCall(Request.Builder().url(url).header("Range", "bytes=3-7").header("If-Range", "\"fixture-v1\"").header("Accept-Encoding", "identity").build()).execute().use {
                assertEquals(206, it.code)
                assertEquals("34567", it.body?.string())
                assertEquals("\"fixture-v1\"", it.header("ETag"))
                assertEquals("Wed, 21 Oct 2015 07:28:00 GMT", it.header("Last-Modified"))
                assertNotNull(it.header("Date"))
                assertEquals("identity", lastEncoding.get())
            }
            client.newCall(Request.Builder().url(url).header("Range", "bytes=36-").header("If-Range", "\"fixture-v1\"").build()).execute().use {
                assertEquals(416, it.code)
                assertEquals("bytes */36", it.header("Content-Range"))
                assertEquals("\"fixture-v1\"", it.header("ETag"))
            }
            client.newCall(Request.Builder().url(url).header("Range", "bytes=3-7").header("If-Range", "\"different\"").build()).execute().use {
                assertEquals(200, it.code)
                assertEquals("0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ", it.body?.string())
            }
        } finally { MediaTransportService.closeSession(session["sessionToken"] as String) }
    }

    @Test
    fun `playlist segments keep provider context and expire with session`() {
        val provider = TestProvider(AtomicBoolean(), AtomicBoolean())
        val link = ExtractorLinkPlayList(
            source = "Test", name = "Parts",
            playlist = listOf(
                PlayListItem("http://127.0.0.1:$mockPort/protected?part=one%26two", 1_000_000),
                PlayListItem("http://127.0.0.1:$mockPort/protected?part=second", 2_000_000)
            ), referer = "", quality = 720,
            headers = mapOf("X-Provider" to "secret"), type = ExtractorLinkType.VIDEO
        )
        val resource = MediaTransportService.registerResource(provider.name, link, provider)
        val session = MediaTransportService.createSession(resource, CoroutineScope(Dispatchers.Default)) { provider }
        val urls = session["playlistUrls"] as List<*>
        assertEquals(2, urls.size)
        urls.forEachIndexed { index, url ->
            val response = client.newCall(Request.Builder().url(url as String).header("Range", "bytes=0-2").build()).execute()
            response.use {
                assertEquals(206, it.code)
                assertEquals(if (index == 0) "part=one%26two" else "part=second", it.body?.string())
            }
        }
        MediaTransportService.closeSession(session["sessionToken"] as String)
        client.newCall(Request.Builder().url(urls[0] as String).build()).execute().use { assertEquals(403, it.code) }
    }

    @Test
    fun `mpv plays both protected playlist segments through provider session`() {
        val available = try { ProcessBuilder("mpv", "--version").start().waitFor() == 0 } catch (_: Exception) { false }
        assumeTrue(available, "mpv is required for the native playback smoke test")
        protectedWaveRequests.forEach { it.set(0) }
        val provider = TestProvider(AtomicBoolean(), AtomicBoolean())
        val link = ExtractorLinkPlayList(
            source = "Test", name = "Protected timeline", referer = "", quality = 720,
            playlist = (1..2).map { PlayListItem("http://127.0.0.1:$mockPort/protected-wave?part=$it", 1_000_000) },
            headers = mapOf("X-Provider" to "secret"), type = ExtractorLinkType.VIDEO
        )
        val resource = MediaTransportService.registerResource(provider.name, link, provider)
        val session = MediaTransportService.createSession(resource, CoroutineScope(Dispatchers.Default)) { provider }
        try {
            val urls = (session["playlistUrls"] as List<*>).map { it as String }
            val timeline = "edl://" + urls.joinToString(";") { "%${it.toByteArray().size}%$it,length=1.000000" }
            val process = ProcessBuilder("mpv", "--no-config", "--vo=null", "--ao=null", "--force-window=no", "--no-terminal", timeline)
                .redirectErrorStream(true).start()
            if (!process.waitFor(20, TimeUnit.SECONDS)) {
                process.destroyForcibly()
                fail("mpv did not finish playing the protected timeline")
            }
            assertEquals(0, process.exitValue(), "mpv did not play the timeline")
            assertTrue(protectedWaveRequests.all { it.get() > 0 }, "both protected segments must be fetched")
        } finally {
            MediaTransportService.closeSession(session["sessionToken"] as String)
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
        client.newCall(Request.Builder().url(badUrl).build()).execute().use { assertEquals(403, it.code) }

        // 3. Close session
        assertTrue(MediaTransportService.closeSession(token))
        client.newCall(Request.Builder().url(transportUrl).build()).execute().use { assertEquals(403, it.code) }
    }

    @Test
    fun `hls manifest rewriting rewrites segments and keys with session token`() {
        val link = ExtractorLink(
            source = "Test",
            name = "Test HLS",
            url = "http://127.0.0.1:$mockPort/playlist.m3u8",
            referer = "",
            quality = 1080,
            headers = mapOf("X-Hls-Fixture" to "secret"),
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
        assertTrue(manifest.contains("subpath=http%3A%2F%2F127.0.0.1%3A$mockPort%2Faudio.m3u8"), "Separate audio must use the provider session")
        assertTrue(manifest.contains("subpath=http%3A%2F%2F127.0.0.1%3A$mockPort%2Finit.mp4"), "Initialisation maps must use the provider session")
        val audio = Regex("""#EXT-X-MEDIA:[^\n]*URI="([^"]+)"""").find(manifest)!!.groupValues[1]
        val init = Regex("""#EXT-X-MAP:[^\n]*URI="([^"]+)"""").find(manifest)!!.groupValues[1]
        client.newCall(Request.Builder().url(audio).build()).execute().use { assertEquals(200, it.code) }
        client.newCall(Request.Builder().url(init).build()).execute().use { assertEquals("init", it.body.string()) }

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
        client.newCall(Request.Builder().url(sessionSafe["transportUrl"] as String).build()).execute().use {
            assertEquals(200, it.code)
            assertEquals("0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ", it.body.string())
        }

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
