package dev.vifu.android

import java.net.URI

data class VifuConnectionConfig(
    val serverUrl: String,
    val appId: String,
    val serverCertificateDer: ByteArray? = null,
    val captureTraceContent: Boolean = false,
) {
    init {
        require(APP_ID.matches(appId)) { "appId must use the vifu_app_<64 hex characters> format" }
        val uri = runCatching { URI(serverUrl) }.getOrNull()
        require(uri?.host != null && (uri.scheme == "https" || isLoopbackHttp(uri))) {
            "serverUrl must use HTTPS; HTTP is allowed only for a loopback address"
        }
    }

    private fun isLoopbackHttp(uri: URI): Boolean =
        uri.scheme == "http" && uri.host in setOf("127.0.0.1", "localhost", "::1")

    private companion object {
        val APP_ID = Regex("^vifu_app_[0-9a-fA-F]{64}$")
    }
}

sealed interface VifuConnectionState {
    data object Stopped : VifuConnectionState
    data object Connecting : VifuConnectionState
    data object Connected : VifuConnectionState
    data object Reconnecting : VifuConnectionState
    data object AuthorizationRequired : VifuConnectionState
    data class Degraded(val message: String?) : VifuConnectionState
    data class Failed(val message: String?) : VifuConnectionState
}
