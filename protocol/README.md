# Extension worker protocol, version 1

Transport: one JSON-RPC 2.0 object per UTF-8 line on stdin/stdout. Stdout is reserved for the protocol; upstream and extension logs are redirected to stderr. Rust launches the worker directly, without a shell or public network listener.

Request:

```json
{"jsonrpc":"2.0","id":1,"method":"health","params":{}}
```

Success:

```json
{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1,"runtime":"jvm","webViewSupported":false}}
```

Failure:

```json
{"jsonrpc":"2.0","id":1,"error":{"code":-32000,"message":"Provider failed"}}
```

| Method | Parameters | Result |
|---|---|---|
| `health` | None | Protocol version, runtime, upstream commit, WebView availability |
| `loadPlugin` | `pluginId`, absolute `path` | Registered providers |
| `unloadPlugin` | `pluginId` | Boolean |
| `listProviders` | None | Providers |
| `getPluginStates` | None | Per-extension loaded state, actual execution mode and failure message |
| `getMainPage` | `providerId`, `page`, optional `section` | `{ rows: [{ name, items, hasNext }], nextSection, hasMoreSections }` |
| `search` | `providerId`, `query`, `page` | `{ items, hasNext }` |
| `load` | `providerId`, provider-owned `url` | Item details and episodes |
| `loadLinks` | `providerId`, opaque episode `data` | Streams and subtitles |

Provider IDs combine the repository-scoped plugin ID and the upstream provider name. Episode `data` and provider-owned URLs are opaque strings: do not parse, normalize, concatenate, or reconstruct them. Stream headers and referers must survive the Rust boundary unchanged. Subtitle headers are represented in the protocol, but the first player implementation only automatically attaches header-free subtitle URLs.

The current worker is sequential. Calls have a coroutine deadline of 120 seconds; Rust has a hard 125-second deadline and discards a timed-out worker. A fresh worker attempts to restore all installed extensions, using the limited desktop DEX bridge for data-only CS3 packages and recording explicit failure reasons for unsupported Android integrations. Homepage categories are fetched one at a time rather than queueing every category before the user can search. A loader failure removes partial registrations before Rust attempts to restore the previous installed package. Independent cancellation, incremental link events, multiple API profiles, Android transport and per-extension operating-system isolation remain planned work.
