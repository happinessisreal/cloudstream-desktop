package dev.cloudstream.worker

import com.fasterxml.jackson.module.kotlin.jacksonObjectMapper
import com.googlecode.d2j.dex.Dex2jar
import org.objectweb.asm.*
import java.io.File
import java.nio.file.Files
import java.nio.file.StandardCopyOption
import java.security.MessageDigest
import java.util.jar.JarFile
import java.util.jar.JarEntry
import java.util.jar.JarOutputStream

data class PackageInspection(
    val file: File,
    val sha256: String,
    val format: String, // "dex" or "jar"
    val apiVersion: Int?,
    val entryClass: String?,
    val legacy: Boolean,
    val requiresResources: Boolean,
    val unavailableApis: List<String>,
    val actionableFailure: String?,
    val pluginType: String // "importer", "provider", "extractor", "empty", "unknown"
) {
    fun toMap(): Map<String, Any?> = mapOf(
        "sha256" to sha256,
        "format" to format,
        "apiVersion" to apiVersion,
        "entryClass" to entryClass,
        "legacy" to legacy,
        "requiresResources" to requiresResources,
        "unavailableApis" to unavailableApis,
        "actionableFailure" to actionableFailure,
        "pluginType" to pluginType
    )
}

data class PreparedPackage(
    val file: File,
    val entryClass: String?,
    val legacy: Boolean,
    val inspection: PackageInspection
)

fun inspectPackage(original: File): PackageInspection {
    require(original.isFile) { "Extension file does not exist: ${original.path}" }
    val sha256 = MessageDigest.getInstance("SHA-256")
        .digest(original.readBytes())
        .joinToString("") { "%02x".format(it) }

    var format = "jar"
    var legacy = false
    var entryClass: String? = null
    var apiVersion: Int? = null
    var requiresResources = false
    val unavailableApis = mutableListOf<String>()
    var actionableFailure: String? = null
    var pluginType = "provider"

    JarFile(original).use { archive ->
        val hasDex = archive.getJarEntry("classes.dex") != null
        val hasClasses = archive.entries().asSequence().any { it.name.endsWith(".class") }
        if (!hasDex && !hasClasses) {
            error("Package contains neither JVM classes nor Android DEX")
        }

        if (hasDex) {
            format = "dex"
            legacy = true
            val manifestEntry = archive.getJarEntry("manifest.json") ?: error("CS3 package has no manifest")
            require(manifestEntry.size in 0..65536) { "CS3 manifest is too large" }
            val manifest = archive.getInputStream(manifestEntry).use { jacksonObjectMapper().readTree(it) }
            requiresResources = manifest.path("requiresResources").asBoolean(false)
            if (requiresResources) {
                unavailableApis.add("android.content.res.Resources")
                actionableFailure = "This extension requires Android resources; the full Android runtime is not available yet"
            }
            apiVersion = manifest.path("apiVersion").takeIf { !it.isMissingNode && !it.isNull }?.asInt()
            entryClass = manifest.path("pluginClassName").asText().takeIf { it.isNotBlank() }
                ?: error("CS3 package has no plugin entry point")
        } else {
            archive.getJarEntry("manifest.json")?.let { manifestEntry ->
                if (manifestEntry.size in 0..65536) {
                    val manifest = archive.getInputStream(manifestEntry).use { jacksonObjectMapper().readTree(it) }
                    apiVersion = manifest.path("apiVersion").takeIf { !it.isMissingNode && !it.isNull }?.asInt()
                    entryClass = manifest.path("pluginClassName").asText().takeIf { it.isNotBlank() }
                }
            }
        }
    }

    // Determine the bytecode archive to scan (converted jar for DEX or original for JAR)
    val bytecodeArchive = if (legacy) {
        if (actionableFailure != null) {
            null
        } else {
            val converted = File(original.parentFile, "$sha256.desktop-dex-v2.jar")
            if (!converted.isFile) {
                val temporary = Files.createTempFile(original.parentFile.toPath(), "desktop-dex-", ".jar")
                try {
                    Files.delete(temporary)
                    Dex2jar.from(original).skipDebug(true).to(temporary)
                    restoreJvmMethodNames(temporary.toFile())
                    Files.move(temporary, converted.toPath(), StandardCopyOption.REPLACE_EXISTING)
                } finally {
                    Files.deleteIfExists(temporary)
                }
            }
            converted
        }
    } else {
        original
    }

    if (bytecodeArchive != null) {
        // Scan dependencies of the bytecode archive
        val (scannedApis, failureAndType) = scanArchiveDependencies(bytecodeArchive)
        unavailableApis.addAll(scannedApis)
        if (actionableFailure == null && failureAndType.first != null) {
            actionableFailure = failureAndType.first
        }
        if (failureAndType.second != "provider") {
            pluginType = failureAndType.second
        }
    }

    return PackageInspection(
        file = bytecodeArchive ?: original,
        sha256 = sha256,
        format = format,
        apiVersion = apiVersion,
        entryClass = entryClass,
        legacy = legacy,
        requiresResources = requiresResources,
        unavailableApis = unavailableApis.distinct(),
        actionableFailure = actionableFailure,
        pluginType = pluginType
    )
}

/** Translate original data-only CS3 bytecode without requiring extension authors
 * to rebuild. This deliberately does not claim general Android compatibility. */
fun preparePackage(original: File): PreparedPackage {
    val inspection = inspectPackage(original)
    if (inspection.actionableFailure != null) {
        throw IllegalStateException(inspection.actionableFailure)
    }
    return PreparedPackage(inspection.file, inspection.entryClass, inspection.legacy, inspection)
}

/** Android stub generators sanitize Kotlin value-class symbols (e.g.
 * constructor-impl becomes constructor_impl). Resolve each external reference
 * against the actual host JVM ABI, keeping exact names/descriptors when present. */
internal fun restoreJvmMethodNames(file: File) {
    val output = File(file.parentFile, "${file.name}.abi")
    val mapping = mutableMapOf<Triple<String, String, String>, String>()
    fun jvmName(owner: String, name: String, descriptor: String): String = mapping.getOrPut(Triple(owner, name, descriptor)) {
        try {
            val type = Class.forName(owner.replace('/', '.'), false, BasePluginClassLoader)
            val methods = (type.methods.asSequence() + type.declaredMethods.asSequence()).filter { Type.getMethodDescriptor(it) == descriptor }.toList()
            if (methods.any { it.name == name }) name
            else methods.firstOrNull { it.name.replace('-', '_') == name }?.name ?: name
        } catch (_: Throwable) { name }
    }
    try {
        JarFile(file).use { archive ->
            JarOutputStream(output.outputStream()).use { target ->
                archive.entries().asSequence().forEach { entry ->
                    val bytes = archive.getInputStream(entry).use { it.readBytes() }
                    val rewritten = if (entry.name.endsWith(".class")) {
                        val reader = ClassReader(bytes)
                        val writer = ClassWriter(reader, 0)
                        reader.accept(object : ClassVisitor(Opcodes.ASM9, writer) {
                            override fun visitMethod(access: Int, name: String, descriptor: String, signature: String?, exceptions: Array<out String>?): MethodVisitor = object : MethodVisitor(Opcodes.ASM9, super.visitMethod(access, name, descriptor, signature, exceptions)) {
                                override fun visitMethodInsn(opcode: Int, owner: String, name: String, descriptor: String, isInterface: Boolean) {
                                    super.visitMethodInsn(opcode, owner, jvmName(owner, name, descriptor), descriptor, isInterface)
                                }
                            }
                        }, 0)
                        writer.toByteArray()
                    } else bytes
                    target.putNextEntry(JarEntry(entry.name)); target.write(rewritten); target.closeEntry()
                }
            }
        }
        Files.move(output.toPath(), file.toPath(), StandardCopyOption.REPLACE_EXISTING)
    } finally { output.delete() }
}
private val BasePluginClassLoader = com.lagradost.cloudstream3.plugins.BasePlugin::class.java.classLoader

internal fun scanArchiveDependencies(file: File): Pair<List<String>, Pair<String?, String>> {
    val unavailable = mutableListOf<String>()
    var failure: String? = null
    var detectedType = "provider"

    fun dependency(owner: String, member: String? = null) {
        if (owner == "com/lagradost/cloudstream3/plugins/RepositoryManager") {
            unavailable.add("com.lagradost.cloudstream3.plugins.RepositoryManager")
            if (failure == null) {
                failure = "This is a repository-import extension, not a media source. Add its repository URLs directly; Android repository-import plugins are not supported yet"
            }
            detectedType = "importer"
        }
        if (owner.startsWith("android/") && owner != "android/util/Log") {
            val apiName = owner.replace('/', '.')
            unavailable.add(apiName)
            if (failure == null) {
                failure = "This extension needs the Android API $apiName${member?.let { ".$it" }.orEmpty()}; full Android features are not supported by the desktop bridge"
            }
        }
        if (owner.startsWith("com/lagradost/cloudstream3/ui/") || owner.startsWith("com/lagradost/cloudstream3/actions/")) {
            val apiName = owner.replace('/', '.')
            unavailable.add(apiName)
            if (failure == null) {
                failure = "This extension requires Android app UI integration, which is not supported by the desktop bridge"
            }
            if (detectedType != "importer") detectedType = "importer"
        }
    }

    JarFile(file).use { archive ->
        archive.entries().asSequence().filter { it.name.endsWith(".class") }.forEach { entry ->
            archive.getInputStream(entry).use { input ->
                ClassReader(input).accept(object : ClassVisitor(Opcodes.ASM9) {
                    override fun visitMethod(access: Int, name: String?, descriptor: String?, signature: String?, exceptions: Array<out String>?): MethodVisitor = object : MethodVisitor(Opcodes.ASM9) {
                        override fun visitMethodInsn(opcode: Int, owner: String, name: String, descriptor: String, isInterface: Boolean) { dependency(owner, name) }
                        override fun visitFieldInsn(opcode: Int, owner: String, name: String, descriptor: String) { dependency(owner, name) }
                        override fun visitTypeInsn(opcode: Int, type: String) { dependency(type) }
                    }
                }, ClassReader.SKIP_DEBUG or ClassReader.SKIP_FRAMES)
            }
        }
    }
    return Pair(unavailable, Pair(failure, detectedType))
}

internal fun checkDesktopDependencies(file: File) {
    val (unavailable, failureAndType) = scanArchiveDependencies(file)
    if (failureAndType.first != null) {
        throw IllegalStateException(failureAndType.first)
    }
}
