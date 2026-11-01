@file:OptIn(com.lagradost.cloudstream3.InternalAPI::class, com.lagradost.cloudstream3.Prerelease::class)
package dev.cloudstream.worker

import com.fasterxml.jackson.module.kotlin.jacksonObjectMapper
import kotlinx.coroutines.*
import java.io.*
import kotlin.test.*

class WorkerProtocolTest {
    private val mapper = jacksonObjectMapper()

    @Test
    fun healthReportsProtocolAndCapabilities() {
        // Run main process for a quick stdin/stdout round-trip
        val input = """
            {"jsonrpc":"2.0","id":1,"method":"health","params":{}}
        """.trimIndent() + "\n"

        val inStream = ByteArrayInputStream(input.toByteArray(Charsets.UTF_8))
        val outBytes = ByteArrayOutputStream()
        val origIn = System.`in`
        val origOut = System.out

        try {
            System.setIn(inStream)
            // Redirect out
            val printOut = PrintStream(outBytes)
            System.setOut(printOut)

            main()

            val lines = outBytes.toString(Charsets.UTF_8).trim().lines().filter { it.isNotBlank() }
            assertTrue(lines.isNotEmpty(), "Expected at least one response line")

            val json = mapper.readTree(lines.first())
            assertEquals("2.0", json.path("jsonrpc").asText())
            assertEquals(1, json.path("id").asInt())
            val result = json.path("result")
            assertEquals(2, result.path("protocolVersion").asInt())
            assertEquals("jvm", result.path("runtime").asText())

            val capabilities = result.path("capabilities").map { it.asText() }
            assertTrue(capabilities.contains("cancellation"))
            assertTrue(capabilities.contains("progressEvents"))
        } finally {
            System.setIn(origIn)
            System.setOut(origOut)
        }
    }

    @Test
    fun cancellationStopsTargetRequest() {
        val input = """
            {"jsonrpc":"2.0","id":100,"method":"cancel","params":{"id":999}}
        """.trimIndent() + "\n"

        val inStream = ByteArrayInputStream(input.toByteArray(Charsets.UTF_8))
        val outBytes = ByteArrayOutputStream()
        val origIn = System.`in`
        val origOut = System.out

        try {
            System.setIn(inStream)
            val printOut = PrintStream(outBytes)
            System.setOut(printOut)

            main()

            val lines = outBytes.toString(Charsets.UTF_8).trim().lines().filter { it.isNotBlank() }
            assertTrue(lines.isNotEmpty())
            val json = mapper.readTree(lines.first())
            assertEquals(100, json.path("id").asInt())
            assertEquals(false, json.path("result").path("cancelled").asBoolean())
            assertEquals("999", json.path("result").path("targetId").asText())
        } finally {
            System.setIn(origIn)
            System.setOut(origOut)
        }
    }

    @Test
    fun malformedJsonEmitsParseErrorWithoutCrashing() {
        val input = "THIS IS NOT VALID JSON\n{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"health\",\"params\":{}}\n"

        val inStream = ByteArrayInputStream(input.toByteArray(Charsets.UTF_8))
        val outBytes = ByteArrayOutputStream()
        val origIn = System.`in`
        val origOut = System.out

        try {
            System.setIn(inStream)
            val printOut = PrintStream(outBytes)
            System.setOut(printOut)

            main()

            val lines = outBytes.toString(Charsets.UTF_8).trim().lines().filter { it.isNotBlank() }
            assertEquals(2, lines.size)

            val errJson = mapper.readTree(lines[0])
            assertEquals(-32700, errJson.path("error").path("code").asInt())

            val healthJson = mapper.readTree(lines[1])
            assertEquals(2, healthJson.path("id").asInt())
            assertEquals(2, healthJson.path("result").path("protocolVersion").asInt())
        } finally {
            System.setIn(origIn)
            System.setOut(origOut)
        }
    }
}
