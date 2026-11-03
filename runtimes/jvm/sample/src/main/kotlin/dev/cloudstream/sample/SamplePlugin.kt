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
    override var hasQuickSearch = true
    override var hasDownloadSupport = true
    override var hasChromecastSupport = false
    override val searchTimeoutMs = 15_000L
    override val getMainPageTimeoutMs = 20_000L
    override val loadTimeoutMs = 20_000L
    override val loadLinksTimeoutMs = 20_000L

    override val mainPage = listOf(
        MainPageData("Open movies", "films", horizontalImages = true),
        MainPageData("Community picks", "community", horizontalImages = false)
    )
    private val films = listOf(
        Film(
            "Big Buck Bunny · Trailer",
            "https://download.blender.org/peach/trailer/trailer_480p.mov",
            "A gentle giant rabbit has a very bad day, then turns the tables on three mischievous woodland creatures. The official trailer for a Blender Foundation open movie, released under CC BY 3.0.",
            2008,
            "https://www.blender.org/films/big-buck-bunny"
        ),
        Film(
            "Sintel",
            "https://download.blender.org/durian/trailer/sintel_trailer-480p.mp4",
            "A young woman searches for the dragon she befriended. The official trailer for the Blender Foundation open movie, released under CC BY 3.0.",
            2010,
            "https://www.blender.org/films/sintel"
        )
    )

    override suspend fun getMainPage(page: Int, request: MainPageRequest): HomePageResponse {
        return if (request.name == "Community picks") {
            val list1 = HomePageList("Popular open titles", listOf(result(films[0])), isHorizontalImages = false)
            val list2 = HomePageList("Classic animations", listOf(result(films[1])), isHorizontalImages = false)
            newHomePageResponse(listOf(list1, list2), hasNext = page < 2)
        } else {
            val items = if (page == 1) films.map { result(it) } else emptyList()
            val list = HomePageList(request.name, items, isHorizontalImages = request.horizontalImages)
            newHomePageResponse(listOf(list), hasNext = false)
        }
    }

    override suspend fun search(query: String): List<SearchResponse> =
        films.filter { it.name.contains(query, true) }.map { result(it) }

    private fun result(film: Film): SearchResponse {
        val res = newMovieSearchResponse(film.name, film.url, TvType.Movie)
        res.year = film.year
        res.score = Score.from10(8.5)
        res.quality = SearchQuality.HD
        return res
    }

    override suspend fun load(url: String): LoadResponse {
        val film = films.first { it.url == url }
        return newMovieLoadResponse(film.name, film.url, TvType.Movie, film.url) {
            plot = film.plot
            year = film.year
            tags = listOf("Animation", "Open movie")
            duration = 596
            score = Score.from10(8.5)
            uniqueUrl = film.uniqueUrl
            syncData = mutableMapOf("tmdb" to "10378", "imdb" to "tt1254207")
            trailers = mutableListOf(
                TrailerData(
                    "https://download.blender.org/peach/trailer/trailer_480p.mov",
                    "https://www.blender.org",
                    false,
                    mapOf("User-Agent" to "BlenderSample/1.0")
                )
            )
            actors = listOf(
                ActorData(
                    Actor("Big Buck Bunny", "https://images.blender.org/bunny.png"),
                    ActorRole.Main,
                    "Main Character",
                    null
                )
            )
            recommendations = listOf(result(films[1]))
            backgroundPosterUrl = "https://images.blender.org/peach/banner.jpg"
            logoUrl = "https://images.blender.org/peach/logo.png"
            contentRating = "G"
            comingSoon = false
        }
    }

    override suspend fun loadLinks(
        data: String,
        isCasting: Boolean,
        subtitleCallback: (SubtitleFile) -> Unit,
        callback: (ExtractorLink) -> Unit
    ): Boolean {
        require(films.any { it.url == data })
        val link = newExtractorLink(name, "Official Blender download", data, ExtractorLinkType.VIDEO)
        link.referer = "https://www.blender.org"
        link.quality = 480
        link.extractorData = "sample-extractor-data"
        link.audioTracks = listOf(newAudioFile("https://download.blender.org/peach/audio_en.aac"))
        callback(link)

        val sub = SubtitleFile("English", "https://download.blender.org/peach/subtitles_en.vtt")
        sub.headers = mapOf("Accept" to "text/vtt")
        subtitleCallback(sub)
        return true
    }

    private data class Film(
        val name: String,
        val url: String,
        val plot: String,
        val year: Int,
        val uniqueUrl: String
    )
}
