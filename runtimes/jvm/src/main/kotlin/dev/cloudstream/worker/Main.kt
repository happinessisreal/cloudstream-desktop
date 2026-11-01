@file:OptIn(com.lagradost.cloudstream3.InternalAPI::class, com.lagradost.cloudstream3.Prerelease::class)
package dev.cloudstream.worker

import com.fasterxml.jackson.databind.JsonNode
import com.fasterxml.jackson.module.kotlin.jacksonObjectMapper
import com.lagradost.cloudstream3.*
import com.lagradost.cloudstream3.plugins.BasePlugin
import com.lagradost.cloudstream3.plugins.Plugin
import android.content.Context
import com.lagradost.cloudstream3.utils.extractorApis
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.objectweb.asm.*
import java.io.File
import java.net.URLClassLoader
import java.util.jar.JarFile

private val mapper = jacksonObjectMapper()
private data class Loaded(val plugin: BasePlugin, val loader: URLClassLoader)
private val loaded = mutableMapOf<String, Loaded>()
private val pluginStates = mutableMapOf<String, Map<String,Any?>>()

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
    val old = loaded.remove(id) ?: return
    try { old.plugin.beforeUnload() } finally {
        APIHolder.allProviders.filter { it.sourcePlugin == id }.forEach(APIHolder::removePluginMapping)
        APIHolder.allProviders.withLock { APIHolder.allProviders.removeAll { it.sourcePlugin == id } }
        extractorApis.withLock { extractorApis.removeAll { it.sourcePlugin == id } }
        old.loader.close()
    }
}

private fun load(params: JsonNode): Any {
    val id = params.path("pluginId").asText()
    val file = File(params.path("path").asText()).canonicalFile
    require(file.isFile) { "Extension file is missing" }
    unload(id)
    var loader: URLClassLoader? = null
    try {
        val prepared = preparePackage(file)
        val classLoader = URLClassLoader(arrayOf(prepared.file.toURI().toURL()), BasePlugin::class.java.classLoader)
        loader = classLoader
        val instance = classLoader.loadClass(prepared.entryClass ?: entryPoint(prepared.file)).getDeclaredConstructor().newInstance() as BasePlugin
        instance.filename = id
        loaded[id] = Loaded(instance, classLoader)
        if (instance is Plugin) instance.load(Context()) else instance.load()
        APIHolder.initAll()
        pluginStates[id] = mapOf("pluginId" to id, "loaded" to true, "runtime" to if (prepared.legacy) "dex_jvm" else "jvm", "error" to null)
        return providers()
    } catch (e: Throwable) {
        unload(id); loader?.close()
        val cause = generateSequence(e) { it.cause }.last()
        val message = cause.message ?: cause.javaClass.simpleName
        pluginStates[id] = mapOf("pluginId" to id, "loaded" to false, "runtime" to "unavailable", "error" to message)
        throw IllegalStateException(message, e)
    }
}

private suspend fun handle(method: String, params: JsonNode): Any? = when(method) {
    "health" -> mapOf(
        "protocolVersion" to 2,
        "runtime" to "jvm",
        "upstreamCommit" to "5024ace2922a148d38df2a816b483b529789cf58",
        "webViewSupported" to false,
        "legacyDexSupported" to true,
        "capabilities" to listOf(
            "loadLinks",
            "getMainPage",
            "search",
            "load",
            "discriminatedStreams",
            "richDetails",
            "providerCapabilities"
        )
    )
    "loadPlugin" -> load(params)
    "unloadPlugin" -> { unload(params.path("pluginId").asText()); true }
    "listProviders" -> providers()
    "getPluginStates" -> pluginStates.values.toList()
    "search" -> {
        val api = provider(params.path("providerId").asText())
        val page = params.path("page").asInt(1)
        if (api.usesWebView) error("This source requires Android WebView and cannot be searched by the desktop worker yet")
        val result = withTimeout(api.searchTimeoutMs ?: 20_000L) { api.search(params.path("query").asText(), page) }
        mapOf("items" to result?.items?.map { mapSearchItem(api, it) }.orEmpty(), "hasNext" to (result?.hasNext ?: false))
    }
    "getMainPage" -> {
        val api = provider(params.path("providerId").asText())
        if (api.usesWebView) error("This provider requires WebView; the JVM browser resolver is not implemented yet")
        val page = params.path("page").asInt(1)
        val index = params.path("section").asInt(0).coerceAtLeast(0)
        val rows = api.mainPage.drop(index).take(1).flatMap { section ->
            val request = MainPageRequest(section.name, section.data, section.horizontalImages)
            val result = withTimeout(api.getMainPageTimeoutMs ?: 20_000L) { api.getMainPage(page, request) }
            val items = result?.items ?: emptyList()
            if (items.isNotEmpty()) {
                items.mapIndexed { itemIdx, list ->
                    mapOf(
                        "name" to list.name,
                        "horizontalImages" to list.isHorizontalImages,
                        "orientation" to if (list.isHorizontalImages) "horizontal" else "vertical",
                        "rowId" to "${section.name}::$itemIdx::${list.name}",
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
                        "items" to emptyList<Any>(),
                        "hasNext" to false
                    )
                )
            }
        }
        mapOf("rows" to rows, "nextSection" to index + 1, "hasMoreSections" to (index + 1 < api.mainPage.size))
    }
    "load" -> {
        val api = provider(params.path("providerId").asText())
        val details = api.load(params.path("url").asText()) ?: error("Provider returned no details")
        mapDetails(api, details)
    }
    "loadLinks" -> {
        val api = provider(params.path("providerId").asText())
        if (api.usesWebView) error("This provider requires WebView; use the Android runtime when available")
        val streams = java.util.Collections.synchronizedList(mutableListOf<Map<String, Any?>>())
        val subtitles = java.util.Collections.synchronizedList(mutableListOf<Map<String, Any?>>())
        api.loadLinks(params.path("data").asText(), false, { sub ->
            subtitles.add(mapSubtitle(sub))
        }, { link ->
            streams.add(mapStream(link))
        })
        mapOf("streams" to streams, "subtitles" to subtitles)
    }
    else -> error("Unknown method: $method")
}

fun main() {
    // Upstream providers log to stdout. Reserve the original stream exclusively for framed RPC.
    val rpcOut = System.out
    System.setOut(System.err)
    generateSequence(::readlnOrNull).forEach { line ->
        var id: JsonNode? = null
        val response = try {
            val request = mapper.readTree(line)
            id = request["id"]
            require(request.path("jsonrpc").asText() == "2.0") { "Expected JSON-RPC 2.0" }
            val result = runBlocking { withTimeout(120_000) { handle(request.path("method").asText(), request.path("params")) } }
            mapOf("jsonrpc" to "2.0", "id" to id, "result" to result)
        } catch (e: Throwable) {
            e.printStackTrace(System.err)
            mapOf("jsonrpc" to "2.0", "id" to id, "error" to mapOf("code" to -32000, "message" to (e.cause?.message ?: e.message ?: e.javaClass.simpleName)))
        }
        rpcOut.println(mapper.writeValueAsString(response)); rpcOut.flush()
    }
    loaded.keys.toList().forEach(::unload)
}
