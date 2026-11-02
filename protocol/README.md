# Extension worker protocol, version 2

Transport: one JSON-RPC 2.0 object per UTF-8 line on stdin/stdout. Stdout is reserved exclusively for framed protocol messages; upstream and extension logs are redirected to stderr. Rust launches the worker directly, without a shell or public network listener.

Negotiation: Rust inspects `protocolVersion` on startup (`health` call). Protocol version 2 is the primary negotiated version, with backward compatibility for version 1 payloads.

## Health Request

Request:

```json
{"jsonrpc":"2.0","id":1,"method":"health","params":{}}
```

Success:

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "protocolVersion": 2,
    "runtime": "jvm",
    "upstreamCommit": "5024ace2922a148d38df2a816b483b529789cf58",
    "webViewSupported": false,
    "legacyDexSupported": true,
    "capabilities": [
      "loadLinks",
      "getMainPage",
      "search",
      "load",
      "discriminatedStreams",
      "richDetails",
      "providerCapabilities",
      "cancellation",
      "progressEvents"
    ]
  }
}
```

Failure:

```json
{"jsonrpc":"2.0","id":1,"error":{"code":-32000,"message":"Provider failed"}}
```

## Methods Overview

| Method | Parameters | Result |
|---|---|---|
| `health` | None | Protocol version, runtime, upstream commit, WebView availability, capabilities list |
| `loadPlugin` | `pluginId`, absolute `path` | Registered providers |
| `unloadPlugin` | `pluginId` | Boolean (cancels active jobs for the plugin before unloading) |
| `listProviders` | None | Enriched providers with capabilities and timeouts |
| `getPluginStates` | None | Per-extension loaded state, execution mode (`jvm` vs `dex`), capability report, unavailable APIs, and failure diagnostics |
| `inspectPackage` | `path` | Pre-flight package inspection without loading: format, sha256, apiVersion, entryClass, requiresResources, unavailableApis, actionableFailure, pluginType |
| `getMainPage` | `providerId`, `page`, optional `section` | `{ rows: [{ rowId, name, horizontalImages, orientation, items, hasNext }], nextSection, hasMoreSections }` |
| `search` | `providerId`, `query`, `page` | `{ items: [MediaItem with score, quality, year, dubStatus, otherName, etc.], hasNext }` |
| `quickSearch` | `providerId`, `query` | `{ items: [MediaItem] }` (Fast search query for providers advertising `hasQuickSearch`) |
| `load` | `providerId`, provider-owned `url` | Enriched details (syncData, trailers, actors, recommendations, dubGroups, seasons, uniqueUrl, backgroundPosterUrl, logoUrl, contentRating) |
| `loadLinks` | `providerId`, opaque episode `data` | Discriminated streams (direct, HLS, DASH, playlist, torrent, DRM) and subtitles; streams incremental progress events |
| `cancel` | `id` (target request ID) | `{ "cancelled": boolean, "targetId": string }` |

## Asynchronous Execution & Cancellation

Transport execution is asynchronous and non-blocking:
- Requests run concurrently in independent coroutines without blocking reading from stdin or writing to stdout.
- Stdout responses may arrive out-of-order and are demultiplexed by their `id`.
- The `cancel` method can be sent at any time to cancel an in-flight operation. When cancelled, the running job stops and responds with error code `-32800` (`"Request cancelled"`).
- In-flight requests are bounded by a concurrency semaphore to prevent runtime resource exhaustion.
- Extension `loadPlugin` and `unloadPlugin` calls are mutex-protected; unloading a plugin automatically cancels in-flight jobs for its providers before closing its classloader.
- Sources with `sequentialMainPage = true` enforce per-provider serialization and `sequentialMainPageDelay` throttling without blocking other providers.

## Progress Notifications

During long-running streaming operations such as `loadLinks`, progress notifications are framed as standard JSON-RPC 2.0 notifications (messages with a `method` and no `id`):

```json
{"jsonrpc":"2.0","method":"progress","params":{"id":12,"type":"stream","stream":{"name":"1080p Stream","url":"https://...","kind":"hls"}}}
{"jsonrpc":"2.0","method":"progress","params":{"id":12,"type":"subtitle","subtitle":{"name":"English","url":"https://...","langTag":"en"}}}
```

When all links and subtitles are extracted, the worker sends the final single response containing the complete lists of streams and subtitles.

## Data Contract Details

### Provider
- `id`: combines repository-scoped plugin ID and provider name (`${pluginId}:${name}`).
- `capabilities`:
  - `hasMainPage`: boolean
  - `hasQuickSearch`: boolean
  - `hasDownloadSupport`: boolean
  - `hasChromecastSupport`: boolean
  - `usesWebView`: boolean
  - `instantLinkLoading`: boolean
  - `sequentialMainPage`: boolean
  - `sequentialMainPageDelayMs` / `sequentialMainPageScrollDelayMs`: timing intervals
  - `vpnStatus`: e.g. `"None"`, `"MightBeNeeded"`, `"Torrent"`
  - `providerType`: e.g. `"DirectProvider"`, `"MetaProvider"`
  - `supportedSyncNames`: array of supported sync identifiers (e.g. `["Trakt", "Mal", "Anilist"]`)
  - `supportedTypes`: media taxonomy types (e.g. `["Movie", "TvSeries", "Anime"]`)
- `timeouts`: per-action deadlines in milliseconds (`searchTimeoutMs`, `getMainPageTimeoutMs`, `loadTimeoutMs`, `loadLinksTimeoutMs`, `quickSearchTimeoutMs`).

### MediaItem & Search Response
- `name`: media display title
- `url`: provider-owned URL (opaque string, round-tripped byte-for-byte)
- `mediaType`: upstream `TvType` (`Movie`, `TvSeries`, `Anime`, `Live`, `Torrent`)
- `year`: release year (integer or null)
- `score`: normalized rating score (float or null)
- `quality`: upstream quality tag (e.g. `"HD"`, `"FourK"`, `"Cam"`)
- `id`: provider-assigned integer identifier if present
- `otherName`: alternative or localized title (e.g. Japanese name)
- `episodesCount`: total episodes count if available in search
- `dubStatus`: `"dubbed"`, `"subbed"`, `"both"`, or `"none"`
- `dubEpisodes`: map of dub status to available episode counts (`{"dubbed": 26, "subbed": 26}`)

### Homepage Rows
- Upstream `HomePageList` rows are mapped individually rather than flattened into a single list.
- `rowId`: stable identity for the shelf (`${sectionName}::${index}::${rowName}`).
- `name`: shelf title.
- `horizontalImages`: boolean orientation indicator.
- `orientation`: `"horizontal"` or `"vertical"`.
- `items`: array of `MediaItem`.
- `hasNext`: pagination status for the row.

### Rich Media Details
- `plot`: textual summary/synopsis
- `tags`: genres and category labels
- `duration`: runtime duration
- `uniqueUrl`: provider unique URL or canonical identity
- `backgroundPosterUrl` & `logoUrl`: visual assets
- `contentRating`: age classification (e.g. `"PG-13"`, `"TV-MA"`)
- `comingSoon`: release status
- `syncData`: external tracking IDs (`{"imdb": "...", "tmdb": "...", "mal": "..."}`)
- `trailers`: list of trailer resources with `extractorUrl`, `referer`, `raw`, `headers`
- `actors`: list of actor credits with `name`, `image`, `role`, `roleString`, `voiceActor`
- `recommendations`: related `MediaItem` entries
- `episodes`: flat legacy-compatible episode list
- `dubGroups`: structured grouping of episodes by dub variant (`"dubbed"`, `"subbed"`, `"none"`) without prefixing names with translated labels
- `seasons`: structured season groupings

### Streams and Subtitles
- Streams are discriminated by `kind`:
  - `"direct"`: direct HTTP video stream
  - `"hls"`: HLS M3U8 manifest
  - `"dash"`: MPEG-DASH manifest
  - `"playlist"`: segment playlist with duration metadata
  - `"torrent"` / `"magnet"`: torrent or magnet URI
  - `"drm"`: encrypted media with `uuid`, `kid`, `key`, `kty`, `licenseUrl`, `keyRequestParameters`
- `audioTracks`: separate audio file resources with headers
- `extractorData`: opaque string preserved byte-for-byte for verifier/interceptor jobs
- Subtitles include `lang`, `url`, `headers`, and `langTag` (ISO language tag)

## Constraints & Compatibility
- Provider URLs and episode `data` are completely opaque: never parse, construct, strip, lowercase, or guess them.
- If consumers or extensions send older protocol v1 payloads, absent fields deserialize to default values without error.
- All unknown future fields at any layer are preserved across round-trips via open map containers (`extra`).
