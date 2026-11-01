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

data class PreparedPackage(val file: File, val entryClass: String?, val legacy: Boolean)

/** Translate original data-only CS3 bytecode without requiring extension authors
 * to rebuild. This deliberately does not claim general Android compatibility. */
fun preparePackage(original: File): PreparedPackage {
    val entryClass: String?
    JarFile(original).use { archive ->
        if (archive.getJarEntry("classes.dex") == null) return PreparedPackage(original,null,false)
        val manifestEntry = archive.getJarEntry("manifest.json") ?: error("CS3 package has no manifest")
        require(manifestEntry.size in 0..65536) { "CS3 manifest is too large" }
        val manifest = archive.getInputStream(manifestEntry).use { jacksonObjectMapper().readTree(it) }
        require(!manifest.path("requiresResources").asBoolean(false)) { "This extension requires Android resources; the full Android runtime is not available yet" }
        entryClass = manifest.path("pluginClassName").asText().takeIf { it.isNotBlank() }
            ?: error("CS3 package has no plugin entry point")
    }
    val hash = MessageDigest.getInstance("SHA-256").digest(original.readBytes()).joinToString("") { "%02x".format(it) }
    val converted = File(original.parentFile,"$hash.desktop-dex-v2.jar")
    if (!converted.isFile) {
        val temporary = Files.createTempFile(original.parentFile.toPath(),"desktop-dex-",".jar")
        try {
            Files.delete(temporary) // The translator owns creation of the output archive.
            Dex2jar.from(original).skipDebug(true).to(temporary)
            checkDesktopDependencies(temporary.toFile())
            restoreJvmMethodNames(temporary.toFile())
            Files.move(temporary,converted.toPath(),StandardCopyOption.REPLACE_EXISTING)
        } finally { Files.deleteIfExists(temporary) }
    } else checkDesktopDependencies(converted)
    return PreparedPackage(converted,entryClass,true)
}

/** Android stub generators sanitize Kotlin value-class symbols (e.g.
 * constructor-impl becomes constructor_impl). Resolve each external reference
 * against the actual host JVM ABI, keeping exact names/descriptors when present. */
internal fun restoreJvmMethodNames(file: File) {
    val output = File(file.parentFile,"${file.name}.abi")
    val mapping = mutableMapOf<Triple<String,String,String>,String>()
    fun jvmName(owner: String, name: String, descriptor: String): String = mapping.getOrPut(Triple(owner,name,descriptor)) {
        try {
            val type = Class.forName(owner.replace('/','.'),false,BasePluginClassLoader)
            val methods = (type.methods.asSequence() + type.declaredMethods.asSequence()).filter { Type.getMethodDescriptor(it) == descriptor }.toList()
            if (methods.any { it.name == name }) name
            else methods.firstOrNull { it.name.replace('-','_') == name }?.name ?: name
        } catch (_: Throwable) { name }
    }
    try {
        JarFile(file).use { archive ->
            JarOutputStream(output.outputStream()).use { target ->
                archive.entries().asSequence().forEach { entry ->
                    val bytes = archive.getInputStream(entry).use { it.readBytes() }
                    val rewritten = if (entry.name.endsWith(".class")) {
                        val reader = ClassReader(bytes)
                        val writer = ClassWriter(reader,0)
                        reader.accept(object : ClassVisitor(Opcodes.ASM9,writer) {
                            override fun visitMethod(access: Int,name: String,descriptor: String,signature: String?,exceptions: Array<out String>?): MethodVisitor = object : MethodVisitor(Opcodes.ASM9,super.visitMethod(access,name,descriptor,signature,exceptions)) {
                                override fun visitMethodInsn(opcode: Int,owner: String,name: String,descriptor: String,isInterface: Boolean) {
                                    super.visitMethodInsn(opcode,owner,jvmName(owner,name,descriptor),descriptor,isInterface)
                                }
                            }
                        },0)
                        writer.toByteArray()
                    } else bytes
                    target.putNextEntry(JarEntry(entry.name));target.write(rewritten);target.closeEntry()
                }
            }
        }
        Files.move(output.toPath(),file.toPath(),StandardCopyOption.REPLACE_EXISTING)
    } finally { output.delete() }
}
private val BasePluginClassLoader = com.lagradost.cloudstream3.plugins.BasePlugin::class.java.classLoader

internal fun checkDesktopDependencies(file: File) {
    fun dependency(owner: String, member: String? = null) {
        if (owner == "com/lagradost/cloudstream3/plugins/RepositoryManager")
            error("This is a repository-import extension, not a media source. Add its repository URLs directly; Android repository-import plugins are not supported yet")
        if (owner.startsWith("android/") && owner != "android/util/Log")
            error("This extension needs the Android API ${owner.replace('/','.')}${member?.let { ".$it" }.orEmpty()}; full Android features are not supported by the desktop bridge")
        if (owner.startsWith("com/lagradost/cloudstream3/ui/") || owner.startsWith("com/lagradost/cloudstream3/actions/"))
            error("This extension requires Android app UI integration, which is not supported by the desktop bridge")
    }
    JarFile(file).use { archive ->
        archive.entries().asSequence().filter { it.name.endsWith(".class") }.forEach { entry ->
            archive.getInputStream(entry).use { input ->
                ClassReader(input).accept(object : ClassVisitor(Opcodes.ASM9) {
                    override fun visitMethod(access: Int, name: String?, descriptor: String?, signature: String?, exceptions: Array<out String>?): MethodVisitor = object : MethodVisitor(Opcodes.ASM9) {
                        override fun visitMethodInsn(opcode: Int, owner: String, name: String, descriptor: String, isInterface: Boolean) { dependency(owner,name) }
                        override fun visitFieldInsn(opcode: Int, owner: String, name: String, descriptor: String) { dependency(owner,name) }
                        override fun visitTypeInsn(opcode: Int, type: String) { dependency(type) }
                    }
                },ClassReader.SKIP_DEBUG or ClassReader.SKIP_FRAMES)
            }
        }
    }
}
