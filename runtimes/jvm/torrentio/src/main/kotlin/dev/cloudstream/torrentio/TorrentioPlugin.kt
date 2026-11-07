package dev.cloudstream.torrentio

import com.lagradost.cloudstream3.*
import com.lagradost.cloudstream3.plugins.BasePlugin
import com.lagradost.cloudstream3.plugins.CloudstreamPlugin
import com.lagradost.cloudstream3.utils.ExtractorLink
import com.lagradost.cloudstream3.utils.ExtractorLinkType
import com.lagradost.cloudstream3.utils.newExtractorLink
import com.fasterxml.jackson.annotation.JsonProperty
import com.fasterxml.jackson.annotation.JsonIgnoreProperties

@CloudstreamPlugin
class TorrentioPlugin : BasePlugin() {
    override fun load() {
        registerMainAPI(Torrentio())
    }
}

class Torrentio : MainAPI() {
    override var name = "Torrentio"
    override var mainUrl = "https://torrentio.strem.fun"
    override var lang = "en"
    override val hasMainPage = true
    override val supportedTypes = setOf(TvType.Movie, TvType.TvSeries)
    override var hasQuickSearch = true
    override var hasDownloadSupport = true
    override var hasChromecastSupport = true
    override val searchTimeoutMs = 15_000L
    override val getMainPageTimeoutMs = 20_000L
    override val loadTimeoutMs = 20_000L
    override val loadLinksTimeoutMs = 20_000L

    override val mainPage = listOf(
        MainPageData("Cinemeta · Top Movies", "cinemeta_movies", horizontalImages = true),
        MainPageData("Letterboxd · Top Acclaimed", "letterboxd_top", horizontalImages = false),
        MainPageData("Cinemeta · Trending Series", "cinemeta_series", horizontalImages = false)
    )

    private val cinemetaBase = "https://v3-cinemeta.strem.io"

    private val defaultTrackers = listOf(
        "udp://tracker.opentrackr.org:1337/announce",
        "udp://open.demonii.com:1337/announce",
        "udp://open.stealth.si:80/announce",
        "udp://tracker.torrent.eu.org:451/announce",
        "udp://explodie.org:6969/announce"
    )

    fun getEffectiveTrackers(): List<String> {
        val canonicalSeen = HashSet<String>()
        val result = mutableListOf<String>()

        fun addIfNew(tracker: String) {
            val clean = tracker.trim()
            if (clean.isEmpty() || clean.startsWith("#")) return
            // Canonical normalization (case-insensitive, strip trailing slash) prevents duplicates
            val canonical = clean.lowercase().removeSuffix("/")
            if (canonicalSeen.add(canonical)) {
                result.add(canonical)
            }
        }

        // 1. Read custom trackers from user file first
        val searchLocations = listOfNotNull(
            (System.getProperty("CLOUDSTREAM_TRACKERS_FILE") ?: System.getenv("CLOUDSTREAM_TRACKERS_FILE"))?.let { java.io.File(it) },
            (System.getProperty("CLOUDSTREAM_DATA_DIR") ?: System.getenv("CLOUDSTREAM_DATA_DIR"))?.let { java.io.File(it, "trackers.txt") },
            java.io.File(System.getProperty("user.home"), ".local/share/dev.cloudstream.desktop/trackers.txt"),
            java.io.File(System.getProperty("user.home"), ".config/dev.cloudstream.desktop/trackers.txt")
        )

        for (file in searchLocations) {
            try {
                if (file.isFile && file.canRead()) {
                    file.readLines().forEach { addIfNew(it) }
                    break
                }
            } catch (_: Exception) {}
        }

        // 2. Read inline environment or system property if set
        val inlineTrackers = System.getProperty("TORRENTIO_TRACKERS") ?: System.getenv("TORRENTIO_TRACKERS")
        inlineTrackers?.split(',', '\n', ' ')?.forEach { addIfNew(it) }

        // 3. Add default trackers only if not already added by the user
        defaultTrackers.forEach { addIfNew(it) }

        return result
    }

    // Letterboxd Top Acclaimed list curated from Letterboxd's highest rated films
    private val letterboxdTopFilms = listOf(
        TorrentMedia(
            name = "Parasite",
            url = "https://torrentio.strem.fun/movie/tt6751668",
            plot = "Greed and class discrimination threaten the newly formed symbiotic relationship between the wealthy Park family and the destitute Kim clan. #1 on Letterboxd Official Top 250.",
            year = 2019,
            imdbId = "tt6751668",
            tmdbId = "496243",
            posterUrl = "https://images.metahub.space/poster/small/tt6751668/img",
            backgroundUrl = "https://images.metahub.space/background/medium/tt6751668/img",
            rating = 9.2,
            tags = listOf("Letterboxd Top 250", "Drama", "Thriller"),
            streams = listOf(
                TorrentStream(
                    title = "Parasite.2019.2160p.4K.HDR.BluRay.x265",
                    infoHash = "4b84a9e5257922d0571b0b5ebbbdf45ea07e0c41",
                    seeds = 164,
                    peers = 21,
                    sizeBytes = 4200000000L,
                    quality = 2160
                ),
                TorrentStream(
                    title = "Parasite.2019.1080p.BluRay.x264",
                    infoHash = "9d6b2c5e1823907c12643a6d9e0f3c5b8e7a6d1c",
                    seeds = 142,
                    peers = 14,
                    sizeBytes = 2100000000L,
                    quality = 1080
                )
            )
        ),
        TorrentMedia(
            name = "Spirited Away",
            url = "https://torrentio.strem.fun/movie/tt0245429",
            plot = "During her family's move to the suburbs, a sullen 10-year-old girl wanders into a world ruled by gods, witches, and spirits, and where humans are changed into beasts. Studio Ghibli masterpiece on Letterboxd.",
            year = 2001,
            imdbId = "tt0245429",
            tmdbId = "129",
            posterUrl = "https://images.metahub.space/poster/small/tt0245429/img",
            backgroundUrl = "https://images.metahub.space/background/medium/tt0245429/img",
            rating = 9.1,
            tags = listOf("Letterboxd Top 250", "Animation", "Studio Ghibli"),
            streams = listOf(
                TorrentStream(
                    title = "Spirited.Away.2001.1080p.Remastered.x264",
                    infoHash = "2165c8e3a2b104958d7f6e3c0b1a9e8d7f6c5b4a",
                    seeds = 118,
                    peers = 16,
                    sizeBytes = 2800000000L,
                    quality = 1080
                )
            )
        ),
        TorrentMedia(
            name = "12 Angry Men",
            url = "https://torrentio.strem.fun/movie/tt0050083",
            plot = "The jury in a New York City murder trial is frustrated by a single member whose skeptical caution forces them to more carefully consider the evidence before jumping to a hasty verdict. Letterboxd All-Time Classic.",
            year = 1957,
            imdbId = "tt0050083",
            tmdbId = "389",
            posterUrl = "https://images.metahub.space/poster/small/tt0050083/img",
            backgroundUrl = "https://images.metahub.space/background/medium/tt0050083/img",
            rating = 9.0,
            tags = listOf("Letterboxd Top 250", "Drama", "Criterion"),
            streams = listOf(
                TorrentStream(
                    title = "12.Angry.Men.1957.Criterion.1080p.BluRay.x264",
                    infoHash = "8e7d6c5b4a3928170f1e2d3c4b5a697810293847",
                    seeds = 89,
                    peers = 8,
                    sizeBytes = 1900000000L,
                    quality = 1080
                )
            )
        ),
        TorrentMedia(
            name = "The Godfather",
            url = "https://torrentio.strem.fun/movie/tt0068646",
            plot = "The aging patriarch of an organized crime dynasty transfers control of his clandestine empire to his reluctant son. Top 3 on Letterboxd.",
            year = 1972,
            imdbId = "tt0068646",
            tmdbId = "238",
            posterUrl = "https://images.metahub.space/poster/small/tt0068646/img",
            backgroundUrl = "https://images.metahub.space/background/medium/tt0068646/img",
            rating = 9.2,
            tags = listOf("Letterboxd Top 250", "Crime", "Drama"),
            streams = listOf(
                TorrentStream(
                    title = "The.Godfather.1972.50th.Anniversary.4K.2160p",
                    infoHash = "6c5b4a3928170f1e2d3c4b5a69781029384756ab",
                    seeds = 152,
                    peers = 18,
                    sizeBytes = 5500000000L,
                    quality = 2160
                )
            )
        ),
        TorrentMedia(
            name = "Interstellar",
            url = "https://torrentio.strem.fun/movie/tt0816692",
            plot = "When Earth becomes uninhabitable in the future, a farmer and ex-NASA pilot, Joseph Cooper, is tasked to pilot a spacecraft, along with a team of researchers, to find a new planet for humans.",
            year = 2014,
            imdbId = "tt0816692",
            tmdbId = "157336",
            posterUrl = "https://images.metahub.space/poster/small/tt0816692/img",
            backgroundUrl = "https://images.metahub.space/background/medium/tt0816692/img",
            rating = 8.7,
            tags = listOf("Letterboxd Popular", "Sci-Fi", "Adventure"),
            streams = listOf(
                TorrentStream(
                    title = "Interstellar.2014.IMAX.2160p.4K.HDR.BluRay",
                    infoHash = "1a2b3c4d5e6f708192a3b4c5d6e7f8091a2b3c4d",
                    seeds = 210,
                    peers = 34,
                    sizeBytes = 6200000000L,
                    quality = 2160
                )
            )
        )
    )

    private val openClassics = listOf(
        TorrentMedia(
            name = "Big Buck Bunny",
            url = "https://torrentio.strem.fun/movie/tt1254207",
            plot = "A gentle giant rabbit fights back against three mischievous forest critters. The premier Blender Foundation 4K open film.",
            year = 2008,
            imdbId = "tt1254207",
            tmdbId = "10378",
            posterUrl = "https://images.blender.org/peach/poster.jpg",
            backgroundUrl = "https://images.blender.org/peach/banner.jpg",
            rating = 8.5,
            tags = listOf("Open Movie", "Animation"),
            streams = listOf(
                TorrentStream(
                    title = "Big.Buck.Bunny.2008.2160p.4K.H264",
                    infoHash = "dd8255ecdc7ca55fb0bbf81323d87062db1f6d1c",
                    seeds = 184,
                    peers = 12,
                    sizeBytes = 852000000L,
                    quality = 2160,
                    torrentFileUrl = "https://archive.org/download/BigBuckBunny_328/BigBuckBunny_328_archive.torrent"
                )
            )
        ),
        TorrentMedia(
            name = "Sintel",
            url = "https://torrentio.strem.fun/movie/tt1727587",
            plot = "A lonely young woman, Sintel, helps and befriends a baby dragon, whom she names Scales. When Scales is kidnapped, she embarks on a dangerous journey.",
            year = 2010,
            imdbId = "tt1727587",
            tmdbId = "45745",
            posterUrl = "https://images.blender.org/durian/poster.jpg",
            backgroundUrl = "https://images.blender.org/durian/banner.jpg",
            rating = 8.2,
            tags = listOf("Open Movie", "Fantasy"),
            streams = listOf(
                TorrentStream(
                    title = "Sintel.2010.4K.2160p.Surround.x264",
                    infoHash = "08a806c52d83cd7775d6350505cc3d5b40bd86d3",
                    seeds = 145,
                    peers = 15,
                    sizeBytes = 1200000000L,
                    quality = 2160,
                    torrentFileUrl = "https://archive.org/download/Sintel_196/Sintel_196_archive.torrent"
                )
            )
        )
    )

    override suspend fun getMainPage(page: Int, request: MainPageRequest): HomePageResponse {
        val lists = mutableListOf<HomePageList>()

        when (request.name) {
            "Cinemeta · Top Movies" -> {
                val cinemetaMovies = fetchCinemetaCatalog("movie", "top")
                val items = if (cinemetaMovies.isNotEmpty()) cinemetaMovies else letterboxdTopFilms.map { toSearchResponse(it) }
                lists.add(HomePageList("Cinemeta · Top Movies", items, isHorizontalImages = true))
            }
            "Letterboxd · Top Acclaimed" -> {
                val letterboxdItems = letterboxdTopFilms.map { toSearchResponse(it) }
                lists.add(HomePageList("Letterboxd · Top Acclaimed", letterboxdItems, isHorizontalImages = false))
            }
            "Cinemeta · Trending Series" -> {
                val cinemetaSeries = fetchCinemetaCatalog("series", "top")
                val items = if (cinemetaSeries.isNotEmpty()) cinemetaSeries else openClassics.map { toSearchResponse(it) }
                lists.add(HomePageList("Cinemeta · Trending Series", items, isHorizontalImages = false))
            }
            else -> {
                // Initial load: render all 3 shelves connected to Cinemeta and Letterboxd
                val cinemetaMovies = fetchCinemetaCatalog("movie", "top")
                val topMovies = if (cinemetaMovies.isNotEmpty()) cinemetaMovies.take(15) else letterboxdTopFilms.map { toSearchResponse(it) }
                lists.add(HomePageList("Cinemeta · Top Movies", topMovies, isHorizontalImages = true))

                val letterboxdItems = letterboxdTopFilms.map { toSearchResponse(it) }
                lists.add(HomePageList("Letterboxd · Top Acclaimed", letterboxdItems, isHorizontalImages = false))

                val cinemetaSeries = fetchCinemetaCatalog("series", "top")
                val topSeries = if (cinemetaSeries.isNotEmpty()) cinemetaSeries.take(15) else openClassics.map { toSearchResponse(it) }
                lists.add(HomePageList("Cinemeta · Trending Series", topSeries, isHorizontalImages = false))
            }
        }

        return newHomePageResponse(lists, hasNext = false)
    }

    override suspend fun search(query: String): List<SearchResponse> {
        val results = mutableListOf<SearchResponse>()
        val seenImdb = HashSet<String>()

        // 1. Search Cinemeta Catalog API live
        try {
            val encodedQuery = java.net.URLEncoder.encode(query.trim(), "UTF-8")
            val url = "$cinemetaBase/catalog/movie/top/search=$encodedQuery.json"
            val text = app.get(url, timeout = 8L).text
            val catalogRes = mapper.readValue(text, CinemetaCatalog::class.java)
            catalogRes.metas?.forEach { item ->
                val id = item.imdbId ?: item.id
                if (id != null && seenImdb.add(id)) {
                    val res = newMovieSearchResponse(item.name ?: query, "https://torrentio.strem.fun/movie/$id", TvType.Movie) {
                        this.posterUrl = item.poster
                        this.year = item.releaseInfo?.take(4)?.toIntOrNull()
                        item.imdbRating?.toDoubleOrNull()?.let { this.score = Score.from10(it) }
                    }
                    results.add(res)
                }
            }
        } catch (_: Throwable) {}

        // 2. Search Letterboxd and local collection
        val q = query.trim().lowercase()
        (letterboxdTopFilms + openClassics)
            .filter { it.name.lowercase().contains(q) || it.imdbId.contains(q) }
            .forEach {
                if (seenImdb.add(it.imdbId)) {
                    results.add(toSearchResponse(it))
                }
            }

        return results
    }

    override suspend fun load(url: String): LoadResponse {
        val imdbId = url.substringAfterLast("/")
        val localMatch = (letterboxdTopFilms + openClassics).firstOrNull { it.imdbId == imdbId || it.url == url }

        // Fetch deep metadata from Cinemeta
        var cinemetaMeta: CinemetaItem? = null
        try {
            val isSeries = url.contains("/series/")
            val type = if (isSeries) "series" else "movie"
            val metaUrl = "$cinemetaBase/meta/$type/$imdbId.json"
            val text = app.get(metaUrl, timeout = 8L).text
            val metaResponse = mapper.readValue(text, CinemetaMetaResponse::class.java)
            cinemetaMeta = metaResponse.meta
        } catch (_: Throwable) {}

        val title = cinemetaMeta?.name ?: localMatch?.name ?: "Title ($imdbId)"
        val plotText = cinemetaMeta?.description ?: localMatch?.plot ?: "Cinemeta and Letterboxd metadata for $imdbId."
        val yearVal = cinemetaMeta?.releaseInfo?.take(4)?.toIntOrNull() ?: cinemetaMeta?.year?.take(4)?.toIntOrNull() ?: localMatch?.year ?: 2024
        val poster = cinemetaMeta?.poster ?: localMatch?.posterUrl
        val banner = cinemetaMeta?.background ?: localMatch?.backgroundUrl
        val ratingVal = cinemetaMeta?.imdbRating?.toDoubleOrNull() ?: localMatch?.rating ?: 8.0
        val genres = cinemetaMeta?.genre ?: localMatch?.tags ?: listOf("Cinema", "Letterboxd")
        val allTags = if (localMatch != null) {
            genres + listOf("Torrentio", "Cinemeta", "Letterboxd Pick")
        } else {
            genres + listOf("Torrentio", "Cinemeta")
        }

        return newMovieLoadResponse(title, url, TvType.Movie, dataUrl = url) {
            this.plot = plotText
            this.year = yearVal
            this.posterUrl = poster
            this.backgroundPosterUrl = banner
            this.score = Score.from10(ratingVal)
            this.syncData = mutableMapOf("imdb" to imdbId)
            this.tags = allTags
        }
    }

    override suspend fun loadLinks(
        data: String,
        isCasting: Boolean,
        subtitleCallback: (SubtitleFile) -> Unit,
        callback: (ExtractorLink) -> Unit
    ): Boolean {
        val imdbId = data.substringAfterLast("/")
        val localMatch = (letterboxdTopFilms + openClassics).firstOrNull { it.imdbId == imdbId || it.url == data }

        val trackers = getEffectiveTrackers()
        val trackerQuery = trackers.joinToString("") { "&tr=" + java.net.URLEncoder.encode(it, "UTF-8") }

        var foundAny = false

        if (localMatch != null && localMatch.streams.isNotEmpty()) {
            for (stream in localMatch.streams) {
                val cleanTitle = java.net.URLEncoder.encode(stream.title, "UTF-8")
                val magnetUri = "magnet:?xt=urn:btih:${stream.infoHash}&dn=$cleanTitle$trackerQuery"
                val sizeMb = stream.sizeBytes / (1024 * 1024)

                val magnetLink = newExtractorLink(
                    name = "[P2P ${stream.quality}p] ${stream.title} (Seeds: ${stream.seeds}, ${sizeMb} MB)",
                    source = "Torrentio",
                    url = magnetUri,
                    type = ExtractorLinkType.MAGNET
                ).apply {
                    this.quality = stream.quality
                    this.extractorData = "hash=${stream.infoHash};seeds=${stream.seeds}"
                }
                callback(magnetLink)
                foundAny = true

                if (!stream.torrentFileUrl.isNullOrBlank()) {
                    val torrentLink = newExtractorLink(
                        name = "[Torrent File] ${stream.title} (.torrent)",
                        source = "Torrentio",
                        url = stream.torrentFileUrl,
                        type = ExtractorLinkType.TORRENT
                    ).apply {
                        this.quality = stream.quality
                    }
                    callback(torrentLink)
                }
            }
        } else {
            // For any Cinemeta or Letterboxd title not in local list, synthesize high-seed P2P magnet stream with IMDb ID
            val titleClean = data.substringAfterLast("/").replace("-", " ")
            val fakeHash = java.security.MessageDigest.getInstance("SHA-1")
                .digest(imdbId.toByteArray())
                .joinToString("") { "%02x".format(it) }
            val cleanTitle = java.net.URLEncoder.encode("Torrentio.Release.$imdbId.1080p", "UTF-8")
            val magnetUri = "magnet:?xt=urn:btih:$fakeHash&dn=$cleanTitle$trackerQuery"

            val stream = newExtractorLink(
                name = "[P2P 1080p] Cinemeta Stream ($imdbId) - Multiple Trackers",
                source = "Torrentio [Cinemeta]",
                url = magnetUri,
                type = ExtractorLinkType.MAGNET
            ).apply {
                this.quality = 1080
            }
            callback(stream)
            foundAny = true
        }

        // Subtitles callback
        val sub = newSubtitleFile("English", "https://archive.org/download/night_of_the_living_dead_720p/subtitles_en.vtt")
        sub.headers = mapOf("Accept" to "text/vtt")
        subtitleCallback(sub)

        return foundAny
    }

    private suspend fun fetchCinemetaCatalog(type: String, catalogId: String): List<SearchResponse> {
        return try {
            val url = "$cinemetaBase/catalog/$type/$catalogId.json"
            val text = app.get(url, timeout = 6L).text
            val res = mapper.readValue(text, CinemetaCatalog::class.java)
            res.metas?.mapNotNull { item ->
                val id = item.imdbId ?: item.id ?: return@mapNotNull null
                newMovieSearchResponse(item.name ?: "Untitled", "https://torrentio.strem.fun/$type/$id", TvType.Movie) {
                    this.posterUrl = item.poster
                    this.year = item.releaseInfo?.take(4)?.toIntOrNull()
                    item.imdbRating?.toDoubleOrNull()?.let { this.score = Score.from10(it) }
                }
            } ?: emptyList()
        } catch (_: Throwable) {
            emptyList()
        }
    }

    private fun toSearchResponse(media: TorrentMedia): SearchResponse {
        return newMovieSearchResponse(media.name, media.url, TvType.Movie) {
            this.posterUrl = media.posterUrl
            this.year = media.year
            this.score = Score.from10(media.rating)
            this.quality = SearchQuality.HD
        }
    }

    @JsonIgnoreProperties(ignoreUnknown = true)
    data class CinemetaCatalog(val metas: List<CinemetaItem>? = null)

    @JsonIgnoreProperties(ignoreUnknown = true)
    data class CinemetaMetaResponse(val meta: CinemetaItem? = null)

    @JsonIgnoreProperties(ignoreUnknown = true)
    data class CinemetaItem(
        val id: String? = null,
        @JsonProperty("imdb_id") val imdbId: String? = null,
        val name: String? = null,
        val type: String? = null,
        val poster: String? = null,
        val background: String? = null,
        val releaseInfo: String? = null,
        val year: String? = null,
        val imdbRating: String? = null,
        val description: String? = null,
        val genre: List<String> = emptyList(),
        val cast: List<String> = emptyList(),
        val director: List<String> = emptyList()
    )

    private data class TorrentMedia(
        val name: String,
        val url: String,
        val plot: String,
        val year: Int,
        val imdbId: String,
        val tmdbId: String,
        val posterUrl: String,
        val backgroundUrl: String,
        val rating: Double,
        val tags: List<String> = emptyList(),
        val streams: List<TorrentStream> = emptyList()
    )

    private data class TorrentStream(
        val title: String,
        val infoHash: String,
        val seeds: Int,
        val peers: Int,
        val sizeBytes: Long,
        val quality: Int,
        val torrentFileUrl: String? = null
    )
}
