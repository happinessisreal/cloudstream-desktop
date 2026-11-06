package dev.cloudstream.worker.webview

interface WebViewResolverBridge {
    fun isSupported(): Boolean
    fun getUnsupportedReport(): Map<String, Any?>
}

object DefaultWebViewResolverBridge : WebViewResolverBridge {
    override fun isSupported(): Boolean = false
    override fun getUnsupportedReport(): Map<String, Any?> = mapOf(
        "supported" to false,
        "reason" to "webview_unsupported",
        "message" to "Desktop JVM worker does not support full Android WebView engine; planned for T33 milestone",
        "missingFeatures" to listOf(
            "request_interception",
            "custom_headers_on_all_requests",
            "isolated_cookie_manager",
            "post_body_inspection"
        )
    )
}
