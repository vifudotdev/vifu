package dev.vifu.android

import android.content.Context
import android.util.Base64
import androidx.annotation.RestrictTo

@RestrictTo(RestrictTo.Scope.LIBRARY_GROUP)
data class VifuStoredGatewayBinding(
    val serverUrl: String,
    val serverCertificateDer: ByteArray?,
) {
    fun connectionConfig(captureTraceContent: Boolean) = VifuConnectionConfig(
        serverUrl = serverUrl,
        serverCertificateDer = serverCertificateDer?.copyOf(),
        captureTraceContent = captureTraceContent,
    )

    companion object {
        fun from(pairing: VifuGatewayPairingCode) = VifuStoredGatewayBinding(
            serverUrl = pairing.serverUrl,
            serverCertificateDer = pairing.serverCertificateDer,
        )
    }
}

@RestrictTo(RestrictTo.Scope.LIBRARY_GROUP)
class VifuGatewayBindingStore(context: Context, scope: String) {
    private val preferences = context.getSharedPreferences(
        "dev.vifu.android.gateway.$scope",
        Context.MODE_PRIVATE,
    )

    fun load(): VifuStoredGatewayBinding? {
        val serverUrl = preferences.getString(SERVER_URL, null) ?: return null
        return runCatching {
            val certificate = preferences.getString(SERVER_CERTIFICATE, null)
                ?.let { Base64.decode(it, Base64.NO_WRAP) }
            VifuStoredGatewayBinding(serverUrl, certificate).also {
                it.connectionConfig(captureTraceContent = true)
            }
        }.getOrElse {
            clear()
            null
        }
    }

    fun save(binding: VifuStoredGatewayBinding) {
        preferences.edit()
            .putString(SERVER_URL, binding.serverUrl)
            .putString(
                SERVER_CERTIFICATE,
                binding.serverCertificateDer?.let { Base64.encodeToString(it, Base64.NO_WRAP) },
            )
            .apply()
    }

    fun clear() {
        preferences.edit().clear().apply()
    }

    private companion object {
        const val SERVER_URL = "server_url"
        const val SERVER_CERTIFICATE = "server_certificate"
    }
}
