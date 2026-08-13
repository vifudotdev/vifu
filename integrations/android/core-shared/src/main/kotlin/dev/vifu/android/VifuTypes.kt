package dev.vifu.android

import java.net.URI

data class VifuConnectionConfig(
    val serverUrl: String,
    val appId: String? = null,
    val serverCertificateDer: ByteArray? = null,
    val captureTraceContent: Boolean = false,
    val enrollmentToken: String? = null,
    val gatewayName: String? = null,
    val gatewayKind: String? = null,
    val gatewayAttributes: Map<String, String> = emptyMap(),
) {
    init {
        require(appId == null || APP_ID.matches(appId)) {
            "appId must use the vifu_app_<64 hex characters> format"
        }
        require(enrollmentToken == null || ENROLLMENT_TOKEN.matches(enrollmentToken)) {
            "enrollmentToken must use the vifu_ge_<64 hex characters> format"
        }
        require(appId == null || enrollmentToken == null) {
            "appId and enrollmentToken cannot both be set"
        }
        require(gatewayName == null || gatewayName.trim().length in 1..128) {
            "gatewayName must contain between 1 and 128 characters"
        }
        require(gatewayKind == null || GATEWAY_KIND.matches(gatewayKind)) {
            "gatewayKind must use lowercase letters, numbers, dots, underscores, or hyphens"
        }
        require(gatewayAttributes.size <= 32) {
            "gatewayAttributes cannot contain more than 32 entries"
        }
        require(gatewayAttributes.all { (key, value) ->
            ATTRIBUTE_KEY.matches(key) && value.length <= 256
        }) {
            "gatewayAttributes must use bounded identifier keys and values"
        }
        val uri = runCatching { URI(serverUrl) }.getOrNull()
        require(uri?.host != null && (uri.scheme == "https" || isLoopbackHttp(uri))) {
            "serverUrl must use HTTPS; HTTP is allowed only for a loopback address"
        }
    }

    private fun isLoopbackHttp(uri: URI): Boolean =
        uri.scheme == "http" && uri.host in setOf("127.0.0.1", "localhost", "::1")

    private companion object {
        val APP_ID = Regex("^vifu_app_[0-9a-fA-F]{64}$")
        val ENROLLMENT_TOKEN = Regex("^vifu_ge_[0-9a-fA-F]{64}$")
        val GATEWAY_KIND = Regex("^[a-z0-9][a-z0-9._-]{0,63}$")
        val ATTRIBUTE_KEY = Regex("^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$")
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
