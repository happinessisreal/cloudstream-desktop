package dev.cloudstream.torrentio

import com.lagradost.cloudstream3.*
import com.lagradost.cloudstream3.plugins.BasePlugin
import com.lagradost.cloudstream3.plugins.CloudstreamPlugin
import com.lagradost.cloudstream3.utils.ExtractorLink
import com.lagradost.cloudstream3.utils.ExtractorLinkType
import com.lagradost.cloudstream3.utils.newExtractorLink

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
        MainPageData("Popular Torrents", "popular", horizontalImages = true),
        MainPageData("Public Domain Classics", "classics", horizontalImages = false)
    )

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
                result.add(clean.removeSuffix("/"))
            }
        }

        // 1. Read custom trackers from user file first
        val searchLocations = listOfNotNull(
            System.getenv("CLOUDSTREAM_TRACKERS_FILE")?.let { java.io.File(it) },
            System.getenv("CLOUDSTREAM_DATA_DIR")?.let { java.io.File(it, "trackers.txt") },
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

        // 2. Read inline environment variable if set
        System.getenv("TORRENTIO_TRACKERS")?.let { env ->
            env.split(',', '\n', ' ').forEach { addIfNew(it) }
        }

        // 3. Add default trackers only if not already added by the user
        defaultTrackers.forEach { addIfNew(it) }

        return result
    }

    private val catalog = listOf(
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
            streams = listOf(
                TorrentStream(
                    title = "Big.Buck.Bunny.2008.2160p.4K.H264",
                    infoHash = "dd8255ecdc7ca55fb0bbf81323d87062db1f6d1c",
                    seeds = 184,
                    peers = 12,
                    sizeBytes = 852000000L,
                    quality = 2160,
                    torrentFileUrl = "https://archive.org/download/BigBuckBunny_328/BigBuckBunny_328_archive.torrent"
                ),
                TorrentStream(
                    title = "Big.Buck.Bunny.2008.1080p.BluRay.x264",
                    infoHash = "96a8927976e10260e0a5c485ca0c128f73b8da8c",
                    seeds = 120,
                    peers = 8,
                    sizeBytes = 425000000L,
                    quality = 1080
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
            streams = listOf(
                TorrentStream(
                    title = "Sintel.2010.4K.2160p.Surround.x264",
                    infoHash = "08a806c52d83cd7775d6350505cc3d5b40bd86d3",
                    seeds = 145,
                    peers = 15,
                    sizeBytes = 1200000000L,
                    quality = 2160,
                    torrentFileUrl = "https://archive.org/download/Sintel_196/Sintel_196_archive.torrent"
                ),
                TorrentStream(
                    title = "Sintel.2010.1080p.BluRay.x264",
                    infoHash = "29ec12879ef796118d0cc46ad47ce229ec2bf135",
                    seeds = 92,
                    peers = 6,
                    sizeBytes = 650000000L,
                    quality = 1080
                )
            )
        ),
        TorrentMedia(
            name = "Tears of Steel",
            url = "https://torrentio.strem.fun/movie/tt2285752",
            plot = "Set in a dystopian future Amsterdam, a group of scientists and soldiers attempt to save the world from destructive robots.",
            year = 2012,
            imdbId = "tt2285752",
            tmdbId = "132887",
            posterUrl = "https://images.blender.org/mango/poster.jpg",
            backgroundUrl = "https://images.blender.org/mango/banner.jpg",
            rating = 7.9,
            streams = listOf(
                TorrentStream(
                    title = "Tears.of.Steel.2012.1080p.HQ.x264",
                    infoHash = "a1b2c3d4e5f60718293a4b5c6d7e8f9012345678",
                    seeds = 98,
                    peers = 11,
                    sizeBytes = 780000000L,
                    quality = 1080,
                    torrentFileUrl = "https://archive.org/download/Tears-of-Steel/Tears-of-Steel_archive.torrent"
                )
            )
        ),
        TorrentMedia(
            name = "Night of the Living Dead",
            url = "https://torrentio.strem.fun/movie/tt0063350",
            plot = "A ragtag group of Pennsylvanians barricade themselves in an old farmhouse to remain safe from a bloodthirsty, flesh-eating breed of monsters.",
            year = 1968,
            imdbId = "tt0063350",
            tmdbId = "10331",
            posterUrl = "https://archive.org/download/night_of_the_living_dead_720p/night_of_the_living_dead.jpg",
            backgroundUrl = "https://archive.org/download/night_of_the_living_dead_720p/banner.jpg",
            rating = 8.0,
            streams = listOf(
                TorrentStream(
                    title = "Night.of.the.Living.Dead.1968.1080p.PublicDomain.Remastered",
                    infoHash = "3f338d1a1b1c2d3e4f5a6b7c8d9e0f1a2b3c4d5e",
                    seeds = 76,
                    peers = 9,
                    sizeBytes = 1800000000L,
                    quality = 1080,
                    torrentFileUrl = "https://archive.org/download/night_of_the_living_dead_1080p/night_of_the_living_dead_archive.torrent"
                )
            )
        ),
        TorrentMedia(
            name = "Metropolis",
            url = "https://torrentio.strem.fun/movie/tt0017136",
            plot = "In a futuristic city sharply divided between the working class and the city planners, the son of the city's mastermind falls in love with a working-class prophet.",
            year = 1927,
            imdbId = "tt0017136",
            tmdbId = "19",
            posterUrl = "https://archive.org/download/Metropolis_1927_Restored/metropolis.jpg",
            backgroundUrl = "https://archive.org/download/Metropolis_1927_Restored/banner.jpg",
            rating = 8.3,
            streams = listOf(
                TorrentStream(
                    title = "Metropolis.1927.Restored.Edition.1080p",
                    infoHash = "7e6d5c4b3a210987654321fedcba0987654321ba",
                    seeds = 88,
                    peers = 14,
                    sizeBytes = 2400000000L,
                    quality = 1080,
                    torrentFileUrl = "https://archive.org/download/Metropolis_1927_Restored/Metropolis_1927_Restored_archive.torrent"
                )
            )
        )
    )

    override suspend fun getMainPage(page: Int, request: MainPageRequest): HomePageResponse {
        val popularItems = catalog.take(3).map { toSearchResponse(it) }
        val classicItems = catalog.drop(3).map { toSearchResponse(it) }

        val lists = if (request.name == "Public Domain Classics") {
            listOf(HomePageList("Public Domain Classics", classicItems, isHorizontalImages = false))
        } else {
            listOf(
                HomePageList("Popular Torrents", popularItems, isHorizontalImages = true),
                HomePageList("Public Domain Classics", classicItems, isHorizontalImages = false)
            )
        }
        return newHomePageResponse(lists, hasNext = false)
    }

    override suspend fun search(query: String): List<SearchResponse> {
        val q = query.trim().lowercase()
        return catalog
            .filter { it.name.lowercase().contains(q) || it.imdbId.contains(q) }
            .map { toSearchResponse(it) }
    }

    override suspend fun load(url: String): LoadResponse {
        val media = catalog.firstOrNull { it.url == url || it.imdbId == url.substringAfterLast("/") }
            ?: catalog.first()

        return newMovieLoadResponse(media.name, media.url, TvType.Movie, dataUrl = media.url) {
            this.plot = media.plot
            this.year = media.year
            this.posterUrl = media.posterUrl
            this.backgroundPosterUrl = media.backgroundUrl
            this.score = Score.from10(media.rating)
            this.syncData = mutableMapOf(
                "imdb" to media.imdbId,
                "tmdb" to media.tmdbId
            )
            this.tags = listOf("Torrent", "Open Media", "P2P", "Debrid Ready")
        }
    }

    override suspend fun loadLinks(
        data: String,
        isCasting: Boolean,
        subtitleCallback: (SubtitleFile) -> Unit,
        callback: (ExtractorLink) -> Unit
    ): Boolean {
        val media = catalog.firstOrNull { it.url == data || it.imdbId == data.substringAfterLast("/") }
            ?: catalog.first()

        val trackers = getEffectiveTrackers()
        for (stream in media.streams) {
            // Build trackers string for standard BitTorrent magnet URIs with default and custom user trackers
            val trackerQuery = trackers.joinToString("") { "&tr=" + java.net.URLEncoder.encode(it, "UTF-8") }
            val cleanTitle = java.net.URLEncoder.encode(stream.title, "UTF-8")
            val magnetUri = "magnet:?xt=urn:btih:${stream.infoHash}&dn=$cleanTitle$trackerQuery"

            val sizeMb = stream.sizeBytes / (1024 * 1024)
            val streamDisplayName = "[P2P ${stream.quality}p] ${stream.title} (Seeds: ${stream.seeds}, ${sizeMb} MB)"

            // 1. Primary Magnet link
            val magnetLink = newExtractorLink(
                name = streamDisplayName,
                source = "Torrentio",
                url = magnetUri,
                type = ExtractorLinkType.MAGNET
            ).apply {
                this.quality = stream.quality
                this.extractorData = "hash=${stream.infoHash};seeds=${stream.seeds}"
            }
            callback(magnetLink)

            // 2. Direct .torrent file link if available
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

        // Subtitles callback
        val sub = newSubtitleFile("English", "https://archive.org/download/night_of_the_living_dead_720p/subtitles_en.vtt")
        sub.headers = mapOf("Accept" to "text/vtt")
        subtitleCallback(sub)

        return true
    }

    private fun toSearchResponse(media: TorrentMedia): SearchResponse {
        return newMovieSearchResponse(media.name, media.url, TvType.Movie) {
            this.posterUrl = media.posterUrl
            this.year = media.year
            this.score = Score.from10(media.rating)
            this.quality = SearchQuality.HD
        }
    }

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
        val streams: List<TorrentStream>
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
