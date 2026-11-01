package dev.cloudstream.sample

import com.lagradost.cloudstream3.*
import com.lagradost.cloudstream3.plugins.BasePlugin
import com.lagradost.cloudstream3.plugins.CloudstreamPlugin
import com.lagradost.cloudstream3.utils.ExtractorLink
import com.lagradost.cloudstream3.utils.ExtractorLinkType
import com.lagradost.cloudstream3.utils.newExtractorLink

@CloudstreamPlugin
class SamplePlugin : BasePlugin() {
    override fun load() { registerMainAPI(BlenderFilms()) }
}

class BlenderFilms : MainAPI() {
    override var name = "Blender Open Movies"
    override var mainUrl = "https://www.blender.org"
    override var lang = "en"
    override val hasMainPage = true
    override val supportedTypes = setOf(TvType.Movie)
    override val mainPage = listOf(MainPageData("Open movies", "films"))
    private val films = listOf(
        Film("Big Buck Bunny · Trailer", "https://download.blender.org/peach/trailer/trailer_480p.mov", "A gentle giant rabbit has a very bad day, then turns the tables on three mischievous woodland creatures. The official trailer for a Blender Foundation open movie, released under CC BY 3.0.", 2008),
        Film("Sintel", "https://download.blender.org/durian/trailer/sintel_trailer-480p.mp4", "A young woman searches for the dragon she befriended. The official trailer for the Blender Foundation open movie, released under CC BY 3.0.", 2010)
    )
    override suspend fun getMainPage(page: Int, request: MainPageRequest): HomePageResponse = newHomePageResponse("Open movies", if(page == 1) films.map { result(it) } else emptyList())
    override suspend fun search(query: String): List<SearchResponse> = films.filter { it.name.contains(query, true) }.map { result(it) }
    private suspend fun result(film: Film): SearchResponse = newMovieSearchResponse(film.name, film.url, TvType.Movie)
    override suspend fun load(url: String): LoadResponse {
        val film = films.first { it.url == url }
        return newMovieLoadResponse(film.name, film.url, TvType.Movie, film.url) {
            plot = film.plot; year = film.year; tags = listOf("Animation", "Open movie")
        }
    }
    override suspend fun loadLinks(data: String, isCasting: Boolean, subtitleCallback: (SubtitleFile) -> Unit, callback: (ExtractorLink) -> Unit): Boolean {
        require(films.any { it.url == data })
        callback(newExtractorLink(name, "Official Blender download", data, ExtractorLinkType.VIDEO))
        return true
    }
    private data class Film(val name: String, val url: String, val plot: String, val year: Int)
}
