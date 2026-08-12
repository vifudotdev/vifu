package dev.vifu.android

import java.net.URI
import java.net.URLDecoder
import java.nio.charset.StandardCharsets
import java.security.MessageDigest
import java.util.Base64

/** A one-time Agent Gateway enrollment bound to one Vifu Server URL. */
class VifuGatewayPairingCode(code: String) {
    val serverUrl: String
    val enrollmentToken: String
    private val certificateDer: ByteArray?
    val serverCertificateDer: ByteArray?
        get() = certificateDer?.copyOf()
    val serverCertificateSha256: String?

    init {
        val uri = parseUri(code)
        require(uri.scheme == "vifu" && uri.host == "gateway" && uri.path == "/enroll") {
            INVALID_CODE
        }
        val query = parseQuery(requireNotNull(uri.rawQuery) { INVALID_CODE })
        val server = requireNotNull(query["server"]) { INVALID_CODE }
        require(isHttpsOrigin(server)) { INVALID_CODE }
        val token = requireNotNull(query["token"]) { INVALID_CODE }
        require(ENROLLMENT_TOKEN.matches(token)) { INVALID_CODE }

        val encodedCertificate = query["certificate"]
        val fingerprint = query["fingerprint"]
        val certificate = when {
            encodedCertificate == null && fingerprint == null -> null
            encodedCertificate != null && fingerprint != null -> {
                val decoded = runCatching { Base64.getDecoder().decode(encodedCertificate) }
                    .getOrNull()
                require(decoded != null && decoded.isNotEmpty() && fingerprint == fingerprint(decoded)) {
                    INVALID_CODE
                }
                decoded
            }
            else -> throw IllegalArgumentException(INVALID_CODE)
        }

        serverUrl = server
        enrollmentToken = token
        certificateDer = certificate
        serverCertificateSha256 = fingerprint
    }

    fun connectionConfig(captureTraceContent: Boolean = false) = VifuConnectionConfig(
        serverUrl = serverUrl,
        serverCertificateDer = certificateDer?.copyOf(),
        captureTraceContent = captureTraceContent,
        enrollmentToken = enrollmentToken,
    )

    private companion object {
        const val INVALID_CODE = "This Vifu pairing code is invalid or has expired."
        val ENROLLMENT_TOKEN = Regex("^vifu_ge_[0-9a-fA-F]{64}$")

        fun parseUri(code: String): URI = runCatching { URI(code.trim()) }
            .getOrElse { throw IllegalArgumentException(INVALID_CODE, it) }

        fun parseQuery(rawQuery: String): Map<String, String> = rawQuery
            .split('&')
            .map { item ->
                val parts = item.split('=', limit = 2)
                require(parts.size == 2) { INVALID_CODE }
                decode(parts[0]) to decode(parts[1])
            }
            .also { pairs ->
                require(pairs.map { it.first }.distinct().size == pairs.size) { INVALID_CODE }
            }
            .toMap()

        fun decode(value: String): String = runCatching {
            URLDecoder.decode(value, StandardCharsets.UTF_8)
        }.getOrElse { throw IllegalArgumentException(INVALID_CODE, it) }

        fun isHttpsOrigin(value: String): Boolean {
            val uri = runCatching { URI(value) }.getOrNull() ?: return false
            return uri.scheme?.lowercase() == "https" &&
                !uri.host.isNullOrEmpty() &&
                uri.userInfo == null &&
                uri.query == null &&
                uri.fragment == null &&
                (uri.path.isNullOrEmpty() || uri.path == "/")
        }

        fun fingerprint(certificate: ByteArray): String = MessageDigest.getInstance("SHA-256")
            .digest(certificate)
            .joinToString(prefix = "sha256:", separator = "") { "%02x".format(it) }
    }
}
