@file:OptIn(com.lagradost.cloudstream3.InternalAPI::class, com.lagradost.cloudstream3.Prerelease::class)
package dev.cloudstream.worker

import com.lagradost.cloudstream3.*
import com.lagradost.cloudstream3.LoadResponse.Companion.addAniListId
import com.lagradost.cloudstream3.LoadResponse.Companion.addImdbId
import com.lagradost.cloudstream3.LoadResponse.Companion.addKitsuId
import com.lagradost.cloudstream3.LoadResponse.Companion.addMalId
import com.lagradost.cloudstream3.LoadResponse.Companion.addTMDbId
import kotlinx.coroutines.runBlocking
import kotlin.test.*

class MetadataIdsTest {
    private class Provider : MainAPI() {
        override var name = "Ids"
        override var mainUrl = "https://example.com"
        override val supportedTypes = setOf(TvType.Anime, TvType.Movie)
    }

    @Test
    fun metadataIdsAreStoredUnderTheirServiceNameAndReachTheDesktop() {
        configureMetadataIdPrefixes()
        val api = Provider()
        val details = runBlocking {
            api.newAnimeLoadResponse("Title", "https://example.com/t", TvType.Anime) {
                addMalId(21)
                addAniListId(21)
                addKitsuId(12)
                addImdbId("tt0388629")
                addTMDbId("37854")
            }
        }
        // Each service has its own key. An empty key (the library default) would make them useless.
        assertEquals("21", details.syncData["mal"])
        assertEquals("21", details.syncData["anilist"])
        assertEquals("12", details.syncData["kitsu"])
        assertFalse(details.syncData.containsKey(""), "no id may be stored under an empty key")
        // IMDb and TMDb are packed into Simkl's single id string as JSON, as Android does.
        val simkl = details.syncData["simkl"]
        assertNotNull(simkl)
        assertTrue(simkl.contains("tt0388629") && simkl.contains("37854"), simkl)

        @Suppress("UNCHECKED_CAST")
        val mapped = mapDetails(api, details)["syncData"] as Map<String, String>
        assertEquals("21", mapped["mal"])
        assertEquals(setOf("mal", "anilist", "kitsu", "simkl"), mapped.keys)
    }
}
