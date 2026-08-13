package dev.vifu.android

import java.net.URLEncoder
import java.nio.charset.StandardCharsets
import java.security.MessageDigest
import java.util.Base64
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertThrows
import org.junit.Test

class VifuGatewayPairingCodeTest {
    @Test
    fun acceptsSystemTrustedHttps() {
        val token = "vifu_ge_" + "a".repeat(64)
        val pairing = VifuGatewayPairingCode(
            "vifu://gateway/enroll?server=https%3A%2F%2Fapi.vifu.ai&token=$token",
        )

        assertEquals("https://api.vifu.ai", pairing.serverUrl)
        assertEquals(token, pairing.enrollmentToken)
        assertNull(pairing.serverCertificateDer)
        assertNull(pairing.serverCertificateSha256)
    }

    @Test
    fun validatesLocalCertificatePin() {
        val token = "vifu_ge_" + "b".repeat(64)
        val certificate = "local-certificate".toByteArray()
        val fingerprint = MessageDigest.getInstance("SHA-256")
            .digest(certificate)
            .joinToString(prefix = "sha256:", separator = "") { "%02x".format(it) }
        val pairing = VifuGatewayPairingCode(
            "vifu://gateway/enroll" +
                "?server=${encode("https://macbook.local:6790")}" +
                "&token=$token" +
                "&certificate=${encode(Base64.getEncoder().encodeToString(certificate))}" +
                "&fingerprint=$fingerprint",
        )

        assertArrayEquals(certificate, pairing.serverCertificateDer)
        assertEquals(fingerprint, pairing.serverCertificateSha256)
        assertEquals(token, pairing.connectionConfig().enrollmentToken)
    }

    @Test
    fun storedBindingKeepsTrustDataButNotTheOneTimeToken() {
        val token = "vifu_ge_" + "e".repeat(64)
        val certificate = "local-certificate".toByteArray()
        val fingerprint = MessageDigest.getInstance("SHA-256")
            .digest(certificate)
            .joinToString(prefix = "sha256:", separator = "") { "%02x".format(it) }
        val pairing = VifuGatewayPairingCode(
            "vifu://gateway/enroll" +
                "?server=${encode("https://macbook.local:6790")}" +
                "&token=$token" +
                "&certificate=${encode(Base64.getEncoder().encodeToString(certificate))}" +
                "&fingerprint=$fingerprint",
        )

        val resumed = VifuStoredGatewayBinding.from(pairing)
            .connectionConfig(captureTraceContent = true)

        assertEquals("https://macbook.local:6790", resumed.serverUrl)
        assertArrayEquals(certificate, resumed.serverCertificateDer)
        assertNull(resumed.enrollmentToken)
        assertEquals(true, resumed.captureTraceContent)
    }

    @Test
    fun rejectsInvalidToken() {
        assertThrows(IllegalArgumentException::class.java) {
            VifuGatewayPairingCode(
                "vifu://gateway/enroll?server=https%3A%2F%2Fapi.vifu.ai&token=vifu_ge_${"z".repeat(64)}",
            )
        }
    }

    @Test
    fun resumeConfigDoesNotRequireAnEnrollmentToken() {
        val config = VifuConnectionConfig(serverUrl = "https://api.vifu.ai")

        assertNull(config.appId)
        assertNull(config.enrollmentToken)
    }

    @Test
    fun oneTimeEnrollmentCanReassignAnAuthorizedDeviceAndIsThenCleared() {
        val token = "vifu_ge_" + "c".repeat(64)
        val config = VifuConnectionConfig(
            serverUrl = "https://api.vifu.ai",
            enrollmentToken = token,
        )

        assertEquals(token, gatewayEnrollmentToken(config, pendingEnrollmentToken = token))
        assertNull(gatewayEnrollmentToken(config, pendingEnrollmentToken = null))
    }

    @Test
    fun appIdRemainsTheManagedBuildSelectorAfterAuthorization() {
        val appId = "vifu_app_" + "d".repeat(64)
        val config = VifuConnectionConfig(
            serverUrl = "https://api.vifu.ai",
            appId = appId,
        )

        assertEquals(appId, gatewayEnrollmentToken(config, pendingEnrollmentToken = null))
    }

    private fun encode(value: String): String = URLEncoder.encode(value, StandardCharsets.UTF_8)
}
