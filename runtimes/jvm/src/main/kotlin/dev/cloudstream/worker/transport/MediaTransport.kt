package dev.cloudstream.worker.transport

import com.lagradost.cloudstream3.MainAPI
import com.lagradost.cloudstream3.app
import com.lagradost.cloudstream3.utils.ExtractorLink
import com.sun.net.httpserver.HttpExchange
import com.sun.net.httpserver.HttpHandler
import com.sun.net.httpserver.HttpServer
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.launch
import okhttp3.Interceptor
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.Response
import java.io.IOException
import java.net.InetAddress
import java.net.InetSocketAddress
import java.net.URI
import java.net.URLDecoder
import java.net.URLEncoder
import java.security.SecureRandom
import java.util.UUID
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.Executors

data class ResourceDescriptor(
    val id: String,
    val providerId: String,
    val upstreamUrl: String,
    val headers: Map<String, String>,
    val referer: String?,
    val extractorData: String?,
    val streamType: String,
    val allowedOrigins: Set<String>,
    val interceptor: Interceptor?
)

class MediaSession(
    val token: String,
    val resource: ResourceDescriptor,
    var verifierJob: Job?,
    val createdAtMs: Long = System.currentTimeMillis()
)

object MediaTransportService {
    private val resources = ConcurrentHashMap<String, ResourceDescriptor>()
    private val sessions = ConcurrentHashMap<String, MediaSession>()
    private val random = SecureRandom()
    private var server: HttpServer? = null
    var port: Int = 0
        private set

    @Synchronized
    fun start() {
        if (server != null) return
        val address = InetSocketAddress(InetAddress.getByName("127.0.0.1"), 0)
        val s = HttpServer.create(address, 0)
        s.createContext("/media", MediaHttpHandler)
        s.executor = Executors.newCachedThreadPool { r ->
            Thread(r, "media-transport-worker").apply { isDaemon = true }
        }
        s.start()
        server = s
        port = s.address.port
        System.err.println("MediaTransportService started on 127.0.0.1:$port")
    }

    @Synchronized
    fun stop() {
        sessions.values.forEach { it.verifierJob?.cancel() }
        sessions.clear()
        resources.clear()
        server?.stop(0)
        server = null
        port = 0
    }

    fun registerResource(
        providerId: String,
        link: ExtractorLink,
        api: MainAPI
    ): String {
        val id = "res_" + UUID.randomUUID().toString().replace("-", "")
        val upstreamUrl = link.url
        val allowed = mutableSetOf<String>()
        try {
            URI(upstreamUrl).host?.let { allowed.add(it.lowercase()) }
        } catch (_: Exception) {}
        try {
            URI(api.mainUrl).host?.let { allowed.add(it.lowercase()) }
        } catch (_: Exception) {}

        val interceptor = try {
            api.getVideoInterceptor(link)
        } catch (_: Exception) {
            null
        }

        val desc = ResourceDescriptor(
            id = id,
            providerId = providerId,
            upstreamUrl = upstreamUrl,
            headers = link.getAllHeaders(),
            referer = link.referer,
            extractorData = link.extractorData,
            streamType = link.type.name,
            allowedOrigins = allowed,
            interceptor = interceptor
        )
        resources[id] = desc
        return id
    }

    fun registerRawResource(
        providerId: String,
        url: String,
        headers: Map<String, String>,
        referer: String? = null,
        allowedOrigins: Set<String> = emptySet(),
        extractorData: String? = null,
        streamType: String = "DIRECT",
        interceptor: Interceptor? = null
    ): String {
        val id = "res_" + UUID.randomUUID().toString().replace("-", "")
        val allowed = allowedOrigins.toMutableSet()
        try {
            URI(url).host?.let { allowed.add(it.lowercase()) }
        } catch (_: Exception) {}
        val desc = ResourceDescriptor(
            id = id,
            providerId = providerId,
            upstreamUrl = url,
            headers = headers,
            referer = referer,
            extractorData = extractorData,
            streamType = streamType,
            allowedOrigins = allowed,
            interceptor = interceptor
        )
        resources[id] = desc
        return id
    }

    fun getResource(id: String): ResourceDescriptor? = resources[id]

    fun createSession(
        resourceId: String,
        scope: CoroutineScope,
        apiLookup: (String) -> MainAPI?
    ): Map<String, Any?> {
        val res = resources[resourceId] ?: error("Unknown resource handle: $resourceId")
        val bytes = ByteArray(16)
        random.nextBytes(bytes)
        val token = bytes.joinToString("") { "%02x".format(it) }

        val api = apiLookup(res.providerId)
        val verifierJob = if (api != null && res.extractorData != null) {
            scope.launch {
                try {
                    api.extractorVerifierJob(res.extractorData)
                } catch (_: CancellationException) {
                } catch (t: Throwable) {
                    if (t !is NotImplementedError) {
                        System.err.println("Extractor verifier job error: ${t.message}")
                    }
                }
            }
        } else null

        val session = MediaSession(token, res, verifierJob)
        sessions[token] = session

        return mapOf(
            "sessionToken" to token,
            "transportUrl" to "http://127.0.0.1:$port/media/$token/$resourceId",
            "port" to port
        )
    }

    fun closeSession(token: String): Boolean {
        val session = sessions.remove(token) ?: return false
        session.verifierJob?.cancel()
        return true
    }

    fun getSession(token: String): MediaSession? = sessions[token]

    fun isVerifierActive(token: String): Boolean {
        return sessions[token]?.verifierJob?.isActive == true
    }

    fun unregisterPluginResources(pluginId: String) {
        val prefix = "$pluginId:"
        val toRemove = resources.filter { it.value.providerId.startsWith(prefix) || it.value.providerId == pluginId }.keys
        toRemove.forEach { resources.remove(it) }
        val sessionsToRemove = sessions.filter { toRemove.contains(it.value.resource.id) }.keys
        sessionsToRemove.forEach { closeSession(it) }
    }
}

object MediaHttpHandler : HttpHandler {
    override fun handle(exchange: HttpExchange) {
        try {
            process(exchange)
        } catch (e: Exception) {
            try {
                sendError(exchange, 500, e.message ?: "Internal transport error")
            } catch (_: Exception) {}
        }
    }

    private fun process(exchange: HttpExchange) {
        val method = exchange.requestMethod.uppercase()
        if (method != "GET" && method != "HEAD") {
            exchange.responseHeaders.set("Allow", "GET, HEAD")
            sendError(exchange, 405, "Method not allowed")
            return
        }

        val rawPath = exchange.requestURI.path ?: ""
        val parts = rawPath.split('/').filter { it.isNotEmpty() }
        // Expected format: /media/{token}/{resourceId}
        if (parts.size < 3 || parts[0] != "media") {
            sendError(exchange, 404, "Invalid media transport path")
            return
        }

        val token = parts[1]
        val resourceId = parts[2]

        val session = MediaTransportService.getSession(token)
        if (session == null || session.resource.id != resourceId) {
            sendError(exchange, 403, "Invalid or expired session token")
            return
        }

        val query = exchange.requestURI.query
        var targetUrl = session.resource.upstreamUrl
        if (!query.isNullOrEmpty()) {
            val queryParams = query.split('&').associate {
                val idx = it.indexOf('=')
                if (idx > 0) it.substring(0, idx) to it.substring(idx + 1) else it to ""
            }
            queryParams["subpath"]?.let { encoded ->
                val decoded = URLDecoder.decode(encoded, "UTF-8")
                targetUrl = try {
                    URI(session.resource.upstreamUrl).resolve(decoded).toString()
                } catch (_: Exception) {
                    decoded
                }
            }
        }

        val targetUri = try {
            URI(targetUrl)
        } catch (_: Exception) {
            sendError(exchange, 400, "Malformed upstream URL: $targetUrl")
            return
        }

        val targetHost = targetUri.host?.lowercase()
        if (targetHost == null || !session.resource.allowedOrigins.contains(targetHost)) {
            sendError(exchange, 403, "Target origin not allowed: $targetHost")
            return
        }

        val defaultClient = try { app.baseClient } catch (_: Throwable) { OkHttpClient() }
        val clientBuilder = defaultClient.newBuilder()
            .followRedirects(false)
            .followSslRedirects(false)
        if (session.resource.interceptor != null) {
            clientBuilder.addInterceptor(session.resource.interceptor)
        }
        val client = clientBuilder.build()

        var currentUrl = targetUrl
        var currentResponse: Response? = null
        var redirectCount = 0
        val maxRedirects = 5

        while (redirectCount <= maxRedirects) {
            val reqBuilder = Request.Builder().url(currentUrl)
            if (method == "HEAD") {
                reqBuilder.head()
            } else {
                reqBuilder.get()
            }

            session.resource.headers.forEach { (k, v) ->
                if (!k.equals("Range", ignoreCase = true) && !k.equals("Host", ignoreCase = true)) {
                    reqBuilder.header(k, v)
                }
            }
            if (session.resource.referer != null && !session.resource.headers.containsKey("Referer")) {
                reqBuilder.header("Referer", session.resource.referer)
            }

            val rangeHeader = exchange.requestHeaders.getFirst("Range")
            if (!rangeHeader.isNullOrEmpty()) {
                reqBuilder.header("Range", rangeHeader)
            }

            val call = client.newCall(reqBuilder.build())
            val resp = call.execute()

            if (resp.isRedirect) {
                resp.close()
                val loc = resp.header("Location")
                if (loc.isNullOrEmpty()) {
                    sendError(exchange, 502, "Redirect without Location header")
                    return
                }
                val redirectUri = try {
                    URI(currentUrl).resolve(loc)
                } catch (_: Exception) {
                    sendError(exchange, 502, "Invalid redirect URI: $loc")
                    return
                }
                val redirectHost = redirectUri.host?.lowercase()
                if (redirectHost == null || !session.resource.allowedOrigins.contains(redirectHost)) {
                    sendError(exchange, 502, "Redirect to unrelated origin rejected: $redirectHost")
                    return
                }
                currentUrl = redirectUri.toString()
                redirectCount++
            } else {
                currentResponse = resp
                break
            }
        }

        if (currentResponse == null) {
            sendError(exchange, 502, "Exceeded maximum redirects")
            return
        }

        currentResponse.use { response ->
            val code = response.code
            val responseHeaders = exchange.responseHeaders
            responseHeaders.set("Access-Control-Allow-Origin", "*")
            responseHeaders.set("Accept-Ranges", "bytes")

            response.header("Content-Type")?.let { responseHeaders.set("Content-Type", it) }
            response.header("Content-Range")?.let { responseHeaders.set("Content-Range", it) }

            val contentType = response.header("Content-Type")?.lowercase() ?: ""
            val isHls = currentUrl.contains(".m3u8", ignoreCase = true) ||
                    contentType.contains("mpegurl") ||
                    contentType.contains("application/x-mpegurl")

            if (method == "HEAD") {
                val len = response.body?.contentLength() ?: -1L
                exchange.sendResponseHeaders(code, if (len >= 0) len else -1)
                exchange.responseBody.close()
                return
            }

            if (isHls && response.isSuccessful) {
                val manifestText = response.body?.string().orEmpty()
                val rewritten = rewriteHlsManifest(
                    manifestText,
                    currentUrl,
                    token,
                    session.resource.id,
                    MediaTransportService.port
                )
                val bytes = rewritten.toByteArray(Charsets.UTF_8)
                responseHeaders.set("Content-Type", "application/vnd.apple.mpegurl")
                exchange.sendResponseHeaders(code, bytes.size.toLong())
                exchange.responseBody.use { it.write(bytes) }
                return
            }

            val contentLength = response.body?.contentLength() ?: -1L
            val bodyStream = response.body?.byteStream()
            if (bodyStream == null) {
                exchange.sendResponseHeaders(code, -1)
                exchange.responseBody.close()
                return
            }

            exchange.sendResponseHeaders(code, if (contentLength >= 0) contentLength else 0)
            val out = exchange.responseBody
            val buffer = ByteArray(64 * 1024)
            try {
                out.use { outputStream ->
                    bodyStream.use { inputStream ->
                        var read: Int
                        while (inputStream.read(buffer).also { read = it } != -1) {
                            outputStream.write(buffer, 0, read)
                        }
                        outputStream.flush()
                    }
                }
            } catch (_: IOException) {
                // Client closed socket (e.g. seek or normal termination)
            }
        }
    }

    private fun rewriteHlsManifest(
        manifest: String,
        manifestUrl: String,
        token: String,
        resourceId: String,
        port: Int
    ): String {
        val lines = manifest.lines()
        val baseUri = try { URI(manifestUrl) } catch (_: Exception) { null }

        return lines.joinToString("\n") { line ->
            val trimmed = line.trim()
            if (trimmed.isEmpty()) {
                line
            } else if (trimmed.startsWith("#EXT-X-KEY:", ignoreCase = true)) {
                // Example: #EXT-X-KEY:METHOD=AES-128,URI="https://example.com/key.bin",IV=...
                val uriRegex = Regex("""URI="([^"]+)"""")
                uriRegex.replace(line) { match ->
                    val originalUri = match.groupValues[1]
                    val resolved = if (baseUri != null) {
                        try { baseUri.resolve(originalUri).toString() } catch (_: Exception) { originalUri }
                    } else originalUri
                    val proxyUrl = "http://127.0.0.1:$port/media/$token/$resourceId?subpath=" +
                            URLEncoder.encode(resolved, "UTF-8")
                    """URI="$proxyUrl""""
                }
            } else if (trimmed.startsWith("#")) {
                line
            } else {
                // Segment URL line
                val resolved = if (baseUri != null) {
                    try { baseUri.resolve(trimmed).toString() } catch (_: Exception) { trimmed }
                } else trimmed
                "http://127.0.0.1:$port/media/$token/$resourceId?subpath=" +
                        URLEncoder.encode(resolved, "UTF-8")
            }
        }
    }

    private fun sendError(exchange: HttpExchange, code: Int, message: String) {
        val json = """{"error":"$message"}"""
        val bytes = json.toByteArray(Charsets.UTF_8)
        exchange.responseHeaders.set("Content-Type", "application/json; charset=utf-8")
        exchange.responseHeaders.set("Access-Control-Allow-Origin", "*")
        exchange.sendResponseHeaders(code, bytes.size.toLong())
        exchange.responseBody.use { it.write(bytes) }
    }
}
