@file:OptIn(com.lagradost.cloudstream3.InternalAPI::class, com.lagradost.cloudstream3.Prerelease::class)
package dev.cloudstream.worker

import com.lagradost.cloudstream3.*
import com.lagradost.cloudstream3.utils.DrmExtractorLink
import com.lagradost.cloudstream3.utils.ExtractorLink
import com.lagradost.cloudstream3.utils.ExtractorLinkPlayList
import com.lagradost.cloudstream3.utils.ExtractorLinkType

internal fun providerId(api: MainAPI): String = "${api.sourcePlugin}:${api.name}"

internal fun mapProvider(api: MainAPI): Map<String, Any?> {
    val capabilities = mapOf(
        "hasMainPage" to api.hasMainPage,
        "hasQuickSearch" to api.hasQuickSearch,
        "hasDownloadSupport" to api.hasDownloadSupport,
        "hasChromecastSupport" to api.hasChromecastSupport,
        "usesWebView" to api.usesWebView,
        "instantLinkLoading" to api.instantLinkLoading,
        "sequentialMainPage" to api.sequentialMainPage,
        "sequentialMainPageDelayMs" to api.sequentialMainPageDelay,
        "sequentialMainPageScrollDelayMs" to api.sequentialMainPageScrollDelay,
        "vpnStatus" to api.vpnStatus.name,
        "providerType" to api.providerType.name,
        "supportedSyncNames" to api.supportedSyncNames.map { it.name },
        "supportedTypes" to api.supportedTypes.map { it.name }
    )
    val timeouts = mapOf(
        "searchTimeoutMs" to api.searchTimeoutMs,
        "quickSearchTimeoutMs" to api.quickSearchTimeoutMs,
        "getMainPageTimeoutMs" to api.getMainPageTimeoutMs,
        "loadTimeoutMs" to api.loadTimeoutMs,
        "loadLinksTimeoutMs" to api.loadLinksTimeoutMs
    )
    return mapOf(
        "id" to providerId(api),
        "pluginId" to api.sourcePlugin,
        "name" to api.name,
        "lang" to api.lang,
        "mainUrl" to api.mainUrl,
        "hasMainPage" to api.hasMainPage,
        "usesWebView" to api.usesWebView,
        "supportedTypes" to api.supportedTypes.map { it.name },
        "capabilities" to capabilities,
        "timeouts" to timeouts
    )
}

internal fun mapSearchItem(api: MainAPI, value: SearchResponse): Map<String, Any?> {
    val year = when (value) {
        is AnimeSearchResponse -> value.year
        is MovieSearchResponse -> value.year
        is TvSeriesSearchResponse -> value.year
        else -> null
    }
    val otherName = when (value) {
        is AnimeSearchResponse -> value.otherName
        else -> null
    }
    val episodesCount = when (value) {
        is TvSeriesSearchResponse -> value.episodes
        else -> null
    }
    val dubEpisodes = when (value) {
        is AnimeSearchResponse -> value.episodes?.entries?.associate { (k, v) -> k.name.lowercase() to v }
        else -> null
    }
    val dubStatus = when (value) {
        is AnimeSearchResponse -> {
            val set = value.dubStatus
            if (!set.isNullOrEmpty()) {
                if (set.contains(DubStatus.Dubbed) && set.contains(DubStatus.Subbed)) "both"
                else set.first().name.lowercase()
            } else null
        }
        else -> null
    }
    val score = try { value.score?.toDouble() } catch (_: Throwable) { null }
    val quality = value.quality?.name
    val id = value.id

    return mapOf(
        "providerId" to providerId(api),
        "name" to value.name,
        "url" to value.url,
        "posterUrl" to value.posterUrl,
        "posterHeaders" to (value.posterHeaders ?: emptyMap<String, String>()),
        "mediaType" to value.type?.name,
        "year" to year,
        "score" to score,
        "rating" to score,
        "quality" to quality,
        "id" to id,
        "otherName" to otherName,
        "episodesCount" to episodesCount,
        "dubEpisodes" to dubEpisodes,
        "dubStatus" to dubStatus
    )
}

internal fun mapEpisode(ep: Episode, dub: DubStatus? = null, prefix: String = ""): Map<String, Any?> {
    val epName = ep.name ?: if (ep.episode != null) "Episode ${ep.episode}" else "Episode"
    val score = try { ep.score?.toDouble() } catch (_: Throwable) { null }
    return mapOf(
        "name" to "$prefix$epName",
        "data" to ep.data,
        "season" to ep.season,
        "episode" to ep.episode,
        "posterUrl" to ep.posterUrl,
        "score" to score,
        "rating" to score,
        "description" to ep.description,
        "date" to ep.date,
        "runTime" to ep.runTime,
        "dubStatus" to dub?.name?.lowercase()
    )
}

internal fun mapDetails(api: MainAPI, details: LoadResponse): Map<String, Any?> {
    val score = try { details.score?.toDouble() } catch (_: Throwable) { null }
    val ratingValue = score

    val episodes: List<Map<String, Any?>> = when (details) {
        is MovieLoadResponse -> listOf(mapOf(
            "name" to details.name,
            "data" to details.dataUrl,
            "season" to null,
            "episode" to null,
            "posterUrl" to details.posterUrl,
            "score" to score,
            "rating" to ratingValue,
            "description" to details.plot,
            "date" to null,
            "runTime" to details.duration,
            "dubStatus" to "none",
            "index" to 0
        ))
        is LiveStreamLoadResponse -> listOf(mapOf(
            "name" to details.name,
            "data" to details.dataUrl,
            "season" to null,
            "episode" to null,
            "posterUrl" to details.posterUrl,
            "score" to score,
            "rating" to ratingValue,
            "description" to details.plot,
            "date" to null,
            "runTime" to details.duration,
            "dubStatus" to "none",
            "index" to 0
        ))
        is TorrentLoadResponse -> listOf(mapOf(
            "name" to details.name,
            "data" to (details.torrent ?: details.magnet ?: details.url),
            "season" to null,
            "episode" to null,
            "posterUrl" to details.posterUrl,
            "score" to score,
            "rating" to ratingValue,
            "description" to details.plot,
            "date" to null,
            "runTime" to details.duration,
            "dubStatus" to "none",
            "index" to 0
        ))
        is TvSeriesLoadResponse -> details.episodes.mapIndexed { idx, ep ->
            mapEpisode(ep, null) + mapOf("index" to idx)
        }
        is AnimeLoadResponse -> details.episodes.entries.flatMap { (dub, eps) ->
            eps.mapIndexed { idx, ep ->
                mapEpisode(ep, dub) + mapOf("index" to idx)
            }
        }
        else -> emptyList()
    }

    val dubGroups: Map<String, List<Map<String, Any?>>> = when (details) {
        is AnimeLoadResponse -> details.episodes.mapKeys { it.key.name.lowercase() }.mapValues { (dubKey, eps) ->
            eps.mapIndexed { idx, ep ->
                val dubStatus = when (dubKey) {
                    "dubbed" -> DubStatus.Dubbed
                    "subbed" -> DubStatus.Subbed
                    else -> DubStatus.None
                }
                mapEpisode(ep, dubStatus) + mapOf("index" to idx)
            }
        }
        else -> emptyMap()
    }

    val seasons: List<Map<String, Any?>> = episodes
        .groupBy { it["season"] as? Int }
        .map { (seasonNum, eps) ->
            mapOf(
                "season" to seasonNum,
                "name" to if (seasonNum != null) "Season $seasonNum" else null,
                "episodes" to eps
            )
        }

    val trailers = details.trailers.map {
        mapOf(
            "extractorUrl" to it.extractorUrl,
            "referer" to it.referer,
            "raw" to it.raw,
            "headers" to (it.headers ?: emptyMap<String, String>())
        )
    }

    val actors = details.actors?.map { actorData ->
        mapOf(
            "actor" to mapOf("name" to actorData.actor.name, "image" to actorData.actor.image),
            "role" to actorData.role?.name,
            "roleString" to actorData.roleString,
            "voiceActor" to actorData.voiceActor?.let { mapOf("name" to it.name, "image" to it.image) }
        )
    }.orEmpty()

    val recommendations = details.recommendations?.map { mapSearchItem(api, it) }.orEmpty()

    return mapOf(
        "item" to mapOf(
            "providerId" to providerId(api),
            "name" to details.name,
            "url" to details.url,
            "posterUrl" to details.posterUrl,
            "posterHeaders" to (details.posterHeaders ?: emptyMap<String, String>()),
            "mediaType" to details.type.name,
            "year" to details.year,
            "score" to score,
            "rating" to ratingValue,
            "uniqueUrl" to details.uniqueUrl
        ),
        "plot" to details.plot,
        "tags" to (details.tags ?: emptyList<String>()),
        "episodes" to episodes,
        "dubGroups" to dubGroups,
        "seasons" to seasons,
        "duration" to details.duration,
        "uniqueUrl" to details.uniqueUrl,
        "backgroundPosterUrl" to details.backgroundPosterUrl,
        "logoUrl" to details.logoUrl,
        "contentRating" to details.contentRating,
        "comingSoon" to details.comingSoon,
        "syncData" to details.syncData,
        "trailers" to trailers,
        "actors" to actors,
        "recommendations" to recommendations
    )
}

internal fun mapStream(link: ExtractorLink, api: MainAPI? = null, providerId: String? = null): Map<String, Any?> {
    val audioTracks = link.audioTracks?.map { track ->
        mapOf("url" to track.url, "headers" to (track.headers ?: emptyMap<String, String>()))
    }.orEmpty()

    val playlist = if (link is ExtractorLinkPlayList) {
        link.playlist.map { item ->
            mapOf("url" to item.url, "durationUs" to item.durationUs)
        }
    } else null

    val drm = if (link is DrmExtractorLink) {
        mapOf(
            "uuid" to link.uuid?.toString(),
            "kid" to link.kid,
            "key" to link.key,
            "kty" to link.kty,
            "keyRequestParameters" to link.keyRequestParameters,
            "licenseUrl" to link.licenseUrl,
            "headers" to (link.headers ?: emptyMap<String, String>())
        )
    } else null

    val kind = when {
        link is DrmExtractorLink -> "drm"
        link is ExtractorLinkPlayList -> "playlist"
        link.type == ExtractorLinkType.M3U8 -> "hls"
        link.type == ExtractorLinkType.DASH -> "dash"
        link.type == ExtractorLinkType.TORRENT -> "torrent"
        link.type == ExtractorLinkType.MAGNET -> "magnet"
        else -> "direct"
    }

    val resourceHandle = if (api != null) {
        val pid = providerId ?: api.name
        dev.cloudstream.worker.transport.MediaTransportService.registerResource(pid, link, api)
    } else null

    val map = mutableMapOf<String, Any?>(
        "name" to link.name,
        "url" to link.url,
        "referer" to link.referer,
        "quality" to link.quality,
        "headers" to link.getAllHeaders(),
        "streamType" to (if (link is DrmExtractorLink) "DRM" else if (link is ExtractorLinkPlayList) "PLAYLIST" else link.type.name),
        "kind" to kind,
        "source" to link.source,
        "extractorData" to link.extractorData,
        "audioTracks" to audioTracks,
        "playlist" to playlist,
        "drm" to drm
    )
    if (resourceHandle != null) {
        map["resourceHandle"] = resourceHandle
    }
    return map
}

internal fun mapSubtitle(sub: SubtitleFile): Map<String, Any?> {
    return mapOf(
        "lang" to sub.lang,
        "url" to sub.url,
        "headers" to (sub.headers ?: emptyMap<String, String>()),
        "langTag" to sub.langTag
    )
}
