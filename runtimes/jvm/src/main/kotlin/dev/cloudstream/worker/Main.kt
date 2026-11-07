@file:OptIn(com.lagradost.cloudstream3.InternalAPI::class, com.lagradost.cloudstream3.Prerelease::class)
package dev.cloudstream.worker

import com.fasterxml.jackson.databind.JsonNode
import com.fasterxml.jackson.module.kotlin.jacksonObjectMapper
import com.lagradost.cloudstream3.*
import com.lagradost.cloudstream3.plugins.BasePlugin
import com.lagradost.cloudstream3.plugins.Plugin
import android.content.Context
import com.lagradost.cloudstream3.utils.extractorApis
import kotlinx.coroutines.*
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import org.objectweb.asm.*
import java.io.File
import java.net.URLClassLoader
import java.util.concurrent.ConcurrentHashMap
import java.util.jar.JarFile

private val mapper = jacksonObjectMapper()
private data class Loaded(val plugin: BasePlugin, val loader: URLClassLoader)
private val loaded = mutableMapOf<String, Loaded>()
private val pluginStates = mutableMapOf<String, Map<String, Any?>>()

private val registryMutex = Mutex()
private var workerScope = CoroutineScope(Dispatchers.Default + SupervisorJob())
private val activeJobs = ConcurrentHashMap<String, Job>()
private val jobPluginMap = ConcurrentHashMap<String, String>()

private val providerMutexes = ConcurrentHashMap<String, Mutex>()
private val lastMainPageTimes = ConcurrentHashMap<String, Long>()

internal var rpcOut: java.io.PrintStream = System.out

fun sendFrame(msg: Map<String, Any?>) {
    val json = mapper.writeValueAsString(msg)
    synchronized(rpcOut) {
        rpcOut.println(json)
        rpcOut.flush()
    }
}

private fun provider(id: String): MainAPI {
    val split = id.indexOf(':')
    require(split > 0) { "Invalid provider identity" }
    val pluginId = id.substring(0, split)
    val name = id.substring(split + 1)
    return APIHolder.allProviders.firstOrNull { it.sourcePlugin == pluginId && it.name == name }
        ?: error("Provider is unavailable. Reload the extension: $name")
}

private fun providers() = APIHolder.allProviders.map { api -> mapProvider(api) }

private fun entryPoint(file: File): String = JarFile(file).use { jar ->
    jar.getJarEntry("manifest.json")?.let { entry ->
        mapper.readTree(jar.getInputStream(entry)).get("pluginClassName")?.asText()?.let { return@use it }
    }
    val found = mutableListOf<String>()
    jar.entries().asSequence().filter { it.name.endsWith(".class") && !it.name.startsWith("META-INF/") }.forEach { entry ->
        val reader = jar.getInputStream(entry).use { ClassReader(it) }
        reader.accept(object : ClassVisitor(Opcodes.ASM9) {
            override fun visitAnnotation(descriptor: String, visible: Boolean): AnnotationVisitor? {
                if (descriptor == "Lcom/lagradost/cloudstream3/plugins/CloudstreamPlugin;") found.add(reader.className.replace('/', '.'))
                return null
            }
        }, ClassReader.SKIP_CODE or ClassReader.SKIP_DEBUG or ClassReader.SKIP_FRAMES)
    }
    require(found.size == 1) { "Expected one @CloudstreamPlugin entry point, found ${found.size}" }
    found.single()
}

private fun unload(id: String) {
    // Cancel in-flight jobs for this plugin before closing classloaders
    val jobsToCancel = jobPluginMap.filterValues { it == id }.keys.toList()
    for (jobKey in jobsToCancel) {
        activeJobs[jobKey]?.cancel(CancellationException("Plugin $id was unloaded"))
    }
    val old = loaded.remove(id) ?: return
    try { old.plugin.beforeUnload() } finally {
        APIHolder.allProviders.filter { it.sourcePlugin == id }.forEach(APIHolder::removePluginMapping)
        APIHolder.allProviders.withLock { APIHolder.allProviders.removeAll { it.sourcePlugin == id } }
        extractorApis.withLock { extractorApis.removeAll { it.sourcePlugin == id } }
        old.loader.close()
        val existing = pluginStates[id]
        if (existing != null) {
            pluginStates[id] = existing + mapOf("loaded" to false)
        }
    }
}

private fun load(params: JsonNode): Any {
    val id = params.path("pluginId").asText()
    val file = File(params.path("path").asText()).canonicalFile
    require(file.isFile) { "Extension file is missing" }
    unload(id)
    var loader: URLClassLoader? = null
    var inspection: PackageInspection? = null
    try {
        val prepared = preparePackage(file)
        inspection = prepared.inspection
        val classLoader = URLClassLoader(arrayOf(prepared.file.toURI().toURL()), BasePlugin::class.java.classLoader)
        loader = classLoader
        val instance = classLoader.loadClass(prepared.entryClass ?: entryPoint(prepared.file)).getDeclaredConstructor().newInstance() as BasePlugin
        instance.filename = id
        loaded[id] = Loaded(instance, classLoader)
        if (instance is Plugin) instance.load(Context()) else instance.load()
        APIHolder.initAll()

        val registeredProviders = APIHolder.allProviders.filter { it.sourcePlugin == id }.map { mapProvider(it) }
        val registeredExtractors = extractorApis.filter { it.sourcePlugin == id }.map { it.name }

        val pluginType = when {
            registeredProviders.isNotEmpty() -> "provider"
            registeredExtractors.isNotEmpty() -> "extractor"
            else -> "empty"
        }

        pluginStates[id] = mapOf(
            "pluginId" to id,
            "loaded" to true,
            "runtime" to if (prepared.legacy) "dex_jvm" else "jvm",
            "format" to prepared.inspection.format,
            "apiVersion" to prepared.inspection.apiVersion,
            "pluginType" to pluginType,
            "providers" to registeredProviders.map { it["id"] },
            "extractors" to registeredExtractors,
            "requiredUnavailableApis" to emptyList<String>(),
            "actionableFailure" to null,
            "sha256" to prepared.inspection.sha256,
            "error" to null
        )
        return providers()
    } catch (e: Throwable) {
        // Rollback any partially registered providers or extractors
        APIHolder.allProviders.withLock {
            APIHolder.allProviders.removeAll { it.sourcePlugin == id }
        }
        extractorApis.withLock {
            extractorApis.removeAll { it.sourcePlugin == id }
        }
        unload(id)
        loader?.close()
        val cause = generateSequence(e) { it.cause }.last()
        val message = cause.message ?: cause.javaClass.simpleName

        val fallbackInspection = inspection ?: try { inspectPackage(file) } catch (_: Throwable) { null }

        pluginStates[id] = mapOf(
            "pluginId" to id,
            "loaded" to false,
            "runtime" to "unavailable",
            "format" to (fallbackInspection?.format ?: "unknown"),
            "apiVersion" to fallbackInspection?.apiVersion,
            "pluginType" to (fallbackInspection?.pluginType ?: "unknown"),
            "providers" to emptyList<String>(),
            "extractors" to emptyList<String>(),
            "requiredUnavailableApis" to (fallbackInspection?.unavailableApis ?: emptyList()),
            "actionableFailure" to message,
            "sha256" to (fallbackInspection?.sha256 ?: ""),
            "error" to message
        )
        throw IllegalStateException(message, e)
    }
}

private fun getCappedTimeout(desired: Long?, defaultMs: Long = 20_000L): Long {
    val t = desired ?: defaultMs
    return t.coerceIn(1_000L, 60_000L)
}

private suspend fun executeGetMainPage(api: MainAPI, page: Int, index: Int): Map<String, Any?> {
    val sections = if (api.mainPage.isNotEmpty()) api.mainPage else listOf(MainPageData(api.name, "", false))
    val rows = sections.drop(index).take(1).flatMap { section ->
        val request = MainPageRequest(section.name, section.data, section.horizontalImages)
        val result = withTimeout(getCappedTimeout(api.getMainPageTimeoutMs, 20_000L)) {
            api.getMainPage(page, request)
        }
        val items = result?.items ?: emptyList()
        if (items.isNotEmpty()) {
            items.mapIndexed { itemIdx, list ->
                mapOf(
                    "name" to list.name,
                    "horizontalImages" to list.isHorizontalImages,
                    "orientation" to if (list.isHorizontalImages) "horizontal" else "vertical",
                    "rowId" to "${section.name}::$itemIdx::${list.name}",
                    "sectionIndex" to index,
                    "listIndex" to itemIdx,
                    "page" to page,
                    "items" to list.list.map { mapSearchItem(api, it) },
                    "hasNext" to (result?.hasNext ?: false)
                )
            }
        } else {
            listOf(
                mapOf(
                    "name" to section.name,
                    "horizontalImages" to section.horizontalImages,
                    "orientation" to if (section.horizontalImages) "horizontal" else "vertical",
                    "rowId" to "${section.name}::0",
                    "sectionIndex" to index,
                    "listIndex" to 0,
                    "page" to page,
                    "items" to emptyList<Any>(),
                    "hasNext" to false
                )
            )
        }
    }
    return mapOf("rows" to rows, "nextSection" to index + 1, "hasMoreSections" to (index + 1 < sections.size))
}

private suspend fun handle(method: String, params: JsonNode, idNode: JsonNode?): Any? = when(method) {
    "health" -> mapOf(
        "protocolVersion" to 2,
        "runtime" to "jvm",
        "upstreamCommit" to "5024ace2922a148d38df2a816b483b529789cf58",
        "mediaTransport" to mapOf(
            "enabled" to true,
            "port" to dev.cloudstream.worker.transport.MediaTransportService.port,
            "address" to "127.0.0.1"
        ),
        "webViewSupported" to false,
        "webViewReport" to dev.cloudstream.worker.webview.DefaultWebViewResolverBridge.getUnsupportedReport(),
        "legacyDexSupported" to true,
        "capabilities" to listOf(
            "loadLinks",
            "getMainPage",
            "search",
            "load",
            "discriminatedStreams",
            "richDetails",
            "providerCapabilities",
            "cancellation",
            "progressEvents",
            "inspectPackage",
            "mediaTransport",
            "sessionHandles",
            "verifierLifecycle",
            "webViewDiagnostic"
        )
    )
    "createMediaSession" -> {
        val resourceHandle = params.path("resourceHandle").asText()
        dev.cloudstream.worker.transport.MediaTransportService.createSession(
            resourceHandle,
            workerScope
        ) { id ->
            try { provider(id) } catch (_: Exception) { null }
        }
    }
    "closeMediaSession" -> {
        val sessionToken = params.path("sessionToken").asText()
        val closed = dev.cloudstream.worker.transport.MediaTransportService.closeSession(sessionToken)
        mapOf("closed" to closed)
    }
    "getWebViewStatus" -> {
        dev.cloudstream.worker.webview.DefaultWebViewResolverBridge.getUnsupportedReport()
    }
    "inspectPackage" -> {
        val file = File(params.path("path").asText()).canonicalFile
        inspectPackage(file).toMap()
    }
    "loadPlugin" -> registryMutex.withLock { load(params) }
    "unloadPlugin" -> registryMutex.withLock {
        val pid = params.path("pluginId").asText()
        dev.cloudstream.worker.transport.MediaTransportService.unregisterPluginResources(pid)
        unload(pid)
        true
    }
    "listProviders" -> providers()
    "getPluginStates" -> pluginStates.values.toList()
    "search" -> {
        val api = provider(params.path("providerId").asText())
        val page = params.path("page").asInt(1)
        if (api.usesWebView) error("This source requires Android WebView and cannot be searched by the desktop worker yet")
        val timeout = getCappedTimeout(api.searchTimeoutMs, 20_000L)
        val result = withTimeout(timeout) { api.search(params.path("query").asText(), page) }
        mapOf("items" to result?.items?.map { mapSearchItem(api, it) }.orEmpty(), "hasNext" to (result?.hasNext ?: false))
    }
    "quickSearch" -> {
        val api = provider(params.path("providerId").asText())
        if (api.usesWebView) error("This source requires Android WebView and cannot be searched by the desktop worker yet")
        val timeout = getCappedTimeout(api.quickSearchTimeoutMs ?: api.searchTimeoutMs, 10_000L)
        val result = withTimeout(timeout) { api.quickSearch(params.path("query").asText()) }
        mapOf("items" to result?.map { mapSearchItem(api, it) }.orEmpty())
    }
    "getMainPage" -> {
        val providerId = params.path("providerId").asText()
        val api = provider(providerId)
        if (api.usesWebView) error("This provider requires WebView; the JVM browser resolver is not implemented yet")
        val page = params.path("page").asInt(1)
        val index = params.path("section").asInt(0).coerceAtLeast(0)
        if (api.sequentialMainPage) {
            val mutex = providerMutexes.computeIfAbsent(providerId) { Mutex() }
            mutex.withLock {
                val delayMs = api.sequentialMainPageDelay
                if (delayMs > 0) {
                    val last = lastMainPageTimes[providerId] ?: 0L
                    val elapsed = System.currentTimeMillis() - last
                    if (elapsed in 1 until delayMs) {
                        delay(delayMs - elapsed)
                    }
                }
                val res = executeGetMainPage(api, page, index)
                lastMainPageTimes[providerId] = System.currentTimeMillis()
                res
            }
        } else {
            executeGetMainPage(api, page, index)
        }
    }
    "load" -> {
        val api = provider(params.path("providerId").asText())
        val timeout = getCappedTimeout(api.loadTimeoutMs, 20_000L)
        val details = withTimeout(timeout) {
            api.load(params.path("url").asText())
        } ?: error("Provider returned no details")
        mapDetails(api, details)
    }
    "loadLinks" -> {
        val api = provider(params.path("providerId").asText())
        if (api.usesWebView) error("This provider requires WebView; use the Android runtime when available")
        val timeout = getCappedTimeout(api.loadLinksTimeoutMs, 30_000L)
        val streams = java.util.Collections.synchronizedList(mutableListOf<Map<String, Any?>>())
        val subtitles = java.util.Collections.synchronizedList(mutableListOf<Map<String, Any?>>())
        withTimeout(timeout) {
            api.loadLinks(params.path("data").asText(), false, { sub ->
                val mapped = mapSubtitle(sub, api, params.path("providerId").asText())
                subtitles.add(mapped)
                if (idNode != null && !idNode.isNull) {
                    sendFrame(mapOf(
                        "jsonrpc" to "2.0",
                        "method" to "progress",
                        "params" to mapOf(
                            "id" to idNode,
                            "type" to "subtitle",
                            "subtitle" to mapped
                        )
                    ))
                }
            }, { link ->
                val mapped = mapStream(link, api, params.path("providerId").asText())
                streams.add(mapped)
                if (idNode != null && !idNode.isNull) {
                    sendFrame(mapOf(
                        "jsonrpc" to "2.0",
                        "method" to "progress",
                        "params" to mapOf(
                            "id" to idNode,
                            "type" to "stream",
                            "stream" to mapped
                        )
                    ))
                }
            })
        }
        mapOf("streams" to streams, "subtitles" to subtitles)
    }
    else -> error("Unknown method: $method")
}

private fun dispatchMethod(method: String, params: JsonNode, idNode: JsonNode?) {
    val idKey = if (idNode != null && !idNode.isNull) idNode.asText() else null
    val job = workerScope.launch(start = CoroutineStart.LAZY) {
        try {
            if (idKey != null) {
                val providerId = params.path("providerId").takeIf { !it.isMissingNode && !it.isNull }?.asText()
                if (providerId != null) {
                    val split = providerId.indexOf(':')
                    if (split > 0) {
                        jobPluginMap[idKey] = providerId.substring(0, split)
                    }
                }
            }
            val result = handle(method, params, idNode)
            if (idNode != null && !idNode.isNull) {
                sendFrame(mapOf("jsonrpc" to "2.0", "id" to idNode, "result" to result))
            }
        } catch (e: CancellationException) {
            if (idNode != null && !idNode.isNull) {
                sendFrame(mapOf(
                    "jsonrpc" to "2.0",
                    "id" to idNode,
                    "error" to mapOf("code" to -32800, "message" to "Request cancelled")
                ))
            }
        } catch (e: TimeoutCancellationException) {
            if (idNode != null && !idNode.isNull) {
                sendFrame(mapOf(
                    "jsonrpc" to "2.0",
                    "id" to idNode,
                    "error" to mapOf("code" to -32000, "message" to "Provider action timed out")
                ))
            }
        } catch (e: Throwable) {
            val cause = generateSequence(e) { it.cause }.last()
            val message = cause.message ?: cause.javaClass.simpleName
            if (idNode != null && !idNode.isNull) {
                sendFrame(mapOf(
                    "jsonrpc" to "2.0",
                    "id" to idNode,
                    "error" to mapOf("code" to -32000, "message" to message)
                ))
            }
        } finally {
            if (idKey != null) {
                activeJobs.remove(idKey)
                jobPluginMap.remove(idKey)
            }
        }
    }
    if (idKey != null) {
        activeJobs[idKey] = job
    }
    job.start()
}

/**
 * Cloudstream Android sets these from its sync services at start-up. Without them every id an
 * extension adds with `addMalId`, `addAniListId`, `addKitsuId` or the Simkl helpers would be
 * stored under an empty key and could never be matched to a metadata service.
 */
internal fun configureMetadataIdPrefixes() {
    LoadResponse.malIdPrefix = "mal"
    LoadResponse.kitsuIdPrefix = "kitsu"
    LoadResponse.aniListIdPrefix = "anilist"
    LoadResponse.simklIdPrefix = "simkl"
}

fun main() {
    configureMetadataIdPrefixes()
    dev.cloudstream.worker.transport.MediaTransportService.start()
    Runtime.getRuntime().addShutdownHook(Thread {
        dev.cloudstream.worker.transport.MediaTransportService.stop()
    })
    // Upstream providers log to stdout. Reserve the original stream exclusively for framed RPC.
    rpcOut = System.out
    System.setOut(System.err)
    val reader = System.`in`.bufferedReader()
    while (true) {
        val line = reader.readLine() ?: break
        val trimmed = line.trim()
        if (trimmed.isEmpty()) continue
        try {
            val request = mapper.readTree(trimmed)
            val idNode = request["id"]
            val method = request.path("method").asText()
            val params = request.path("params")
            if (request.path("jsonrpc").asText() != "2.0" || method.isBlank()) {
                sendFrame(mapOf(
                    "jsonrpc" to "2.0",
                    "id" to idNode,
                    "error" to mapOf("code" to -32600, "message" to "Invalid Request")
                ))
                continue
            }
            if (method == "cancel") {
                val targetId = params.path("id").asText()
                val targetJob = activeJobs[targetId]
                val cancelled = if (targetJob != null && targetJob.isActive) {
                    targetJob.cancel(CancellationException("Cancelled by client"))
                    true
                } else {
                    false
                }
                if (idNode != null && !idNode.isNull) {
                    sendFrame(mapOf(
                        "jsonrpc" to "2.0",
                        "id" to idNode,
                        "result" to mapOf("cancelled" to cancelled, "targetId" to targetId)
                    ))
                }
                continue
            }
            dispatchMethod(method, params, idNode)
        } catch (e: Throwable) {
            sendFrame(mapOf(
                "jsonrpc" to "2.0",
                "id" to null,
                "error" to mapOf("code" to -32700, "message" to (e.message ?: "Parse error"))
            ))
        }
    }
    runBlocking {
        withTimeoutOrNull(5000) {
            activeJobs.values.toList().joinAll()
        }
        workerScope.cancel()
        workerScope = CoroutineScope(Dispatchers.Default + SupervisorJob())
        registryMutex.withLock {
            loaded.keys.toList().forEach(::unload)
        }
    }
}
