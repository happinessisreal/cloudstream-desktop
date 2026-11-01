package dev.cloudstream.worker

import org.objectweb.asm.*
import java.io.File
import java.net.URLClassLoader
import java.nio.file.Files
import java.util.jar.JarEntry
import java.util.jar.JarOutputStream
import kotlin.test.*

class LegacyDexTest {
    private fun fixture(methodName: String, androidType: String? = null, block: (File) -> Unit) {
        val directory = Files.createTempDirectory("cloudstream-abi-test").toFile()
        try {
            val file = File(directory, "fixture.jar")
            val writer = ClassWriter(ClassWriter.COMPUTE_MAXS)
            writer.visit(Opcodes.V1_8, Opcodes.ACC_PUBLIC, "LegacyFixture", null, "java/lang/Object", null)
            val method = writer.visitMethod(Opcodes.ACC_PUBLIC or Opcodes.ACC_STATIC, "roundTrip", "(Ljava/lang/Object;)Ljava/lang/Object;", null, null)
            method.visitCode()
            if (androidType != null) {
                method.visitTypeInsn(Opcodes.NEW, androidType)
                method.visitInsn(Opcodes.POP)
            }
            method.visitVarInsn(Opcodes.ALOAD, 0)
            method.visitMethodInsn(Opcodes.INVOKESTATIC, "kotlin/Result", methodName, "(Ljava/lang/Object;)Ljava/lang/Object;", false)
            method.visitInsn(Opcodes.ARETURN)
            method.visitMaxs(0, 0)
            method.visitEnd()
            writer.visitEnd()
            JarOutputStream(file.outputStream()).use { jar ->
                jar.putNextEntry(JarEntry("LegacyFixture.class"))
                jar.write(writer.toByteArray())
                jar.closeEntry()
            }
            block(file)
        } finally {
            directory.deleteRecursively()
        }
    }

    @Test
    fun restoresAndroidSanitizedKotlinValueClassMethodNames() {
        fixture("constructor_impl") { file ->
            restoreJvmMethodNames(file)
            URLClassLoader(arrayOf(file.toURI().toURL()), javaClass.classLoader).use { loader ->
                assertEquals("episode data", loader.loadClass("LegacyFixture").getMethod("roundTrip", Any::class.java).invoke(null, "episode data"))
            }
        }
    }

    @Test
    fun preservesExactJvmNames() {
        fixture("constructor-impl") { file ->
            restoreJvmMethodNames(file)
            URLClassLoader(arrayOf(file.toURI().toURL()), javaClass.classLoader).use { loader ->
                assertEquals("unchanged", loader.loadClass("LegacyFixture").getMethod("roundTrip", Any::class.java).invoke(null, "unchanged"))
            }
        }
    }

    @Test
    fun rejectsAndroidWebViewBeforeLoadingCode() {
        fixture("constructor_impl", "android/webkit/WebView") { file ->
            assertContains(assertFailsWith<IllegalStateException> { checkDesktopDependencies(file) }.message.orEmpty(), "android.webkit.WebView")
        }
    }

    @Test
    fun inspectsPackageCapabilityReportAndIdentifiesImporterPlugins() {
        fixture("constructor_impl", "com/lagradost/cloudstream3/plugins/RepositoryManager") { file ->
            val inspection = inspectPackage(file)
            assertEquals("jar", inspection.format)
            assertEquals("importer", inspection.pluginType)
            assertTrue(inspection.unavailableApis.contains("com.lagradost.cloudstream3.plugins.RepositoryManager"))
            assertNotNull(inspection.actionableFailure)
            assertTrue(inspection.actionableFailure.contains("repository-import extension"))
        }
    }

    @Test
    fun inspectsRequiresResourcesManifestFailure() {
        val directory = Files.createTempDirectory("cloudstream-res-test").toFile()
        try {
            val file = File(directory, "res_fixture.cs3")
            JarOutputStream(file.outputStream()).use { jar ->
                jar.putNextEntry(JarEntry("classes.dex"))
                jar.write("dex\n035\u0000".toByteArray())
                jar.closeEntry()

                jar.putNextEntry(JarEntry("manifest.json"))
                jar.write("""{"pluginClassName":"example.Plugin","requiresResources":true,"apiVersion":2}""".toByteArray())
                jar.closeEntry()
            }

            val inspection = inspectPackage(file)
            assertEquals("dex", inspection.format)
            assertTrue(inspection.legacy)
            assertEquals(2, inspection.apiVersion)
            assertTrue(inspection.requiresResources)
            assertTrue(inspection.unavailableApis.contains("android.content.res.Resources"))
            assertNotNull(inspection.actionableFailure)
            assertTrue(inspection.actionableFailure.contains("requires Android resources"))
        } finally {
            directory.deleteRecursively()
        }
    }
}
