package com.lagradost.cloudstream3.plugins

import android.content.Context

/** Compatibility entry point for plugins whose Android dependency is limited
 * to an unused load(Context) argument. Providers/extractors use upstream APIs. */
abstract class Plugin : BasePlugin() {
    open fun load(context: Context) { load() }
}
