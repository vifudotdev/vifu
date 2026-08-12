package dev.vifu.android

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec
import org.json.JSONObject

internal data class VifuStoredIdentity(val privateKey: String, val deviceToken: String?)

internal class VifuCredentialStore(context: Context, scope: String) {
    private val preferences =
        context.getSharedPreferences("dev.vifu.android.$scope", Context.MODE_PRIVATE)

    fun load(): VifuStoredIdentity? {
        val encoded = preferences.getString(PAYLOAD_KEY, null) ?: return null
        val payload = JSONObject(String(decrypt(Base64.decode(encoded, Base64.NO_WRAP))))
        return VifuStoredIdentity(
            privateKey = payload.getString("privateKey"),
            deviceToken = payload.optString("deviceToken").takeIf(String::isNotEmpty),
        )
    }

    fun save(identity: VifuStoredIdentity) {
        val payload = JSONObject()
            .put("privateKey", identity.privateKey)
            .put("deviceToken", identity.deviceToken.orEmpty())
            .toString()
            .toByteArray()
        preferences.edit()
            .putString(PAYLOAD_KEY, Base64.encodeToString(encrypt(payload), Base64.NO_WRAP))
            .apply()
    }

    private fun encrypt(plaintext: ByteArray): ByteArray {
        val cipher = Cipher.getInstance(TRANSFORMATION)
        cipher.init(Cipher.ENCRYPT_MODE, key())
        return cipher.iv + cipher.doFinal(plaintext)
    }

    private fun decrypt(ciphertext: ByteArray): ByteArray {
        require(ciphertext.size > IV_SIZE) { "Stored Vifu credentials are invalid" }
        val cipher = Cipher.getInstance(TRANSFORMATION)
        cipher.init(
            Cipher.DECRYPT_MODE,
            key(),
            GCMParameterSpec(128, ciphertext.copyOfRange(0, IV_SIZE)),
        )
        return cipher.doFinal(ciphertext.copyOfRange(IV_SIZE, ciphertext.size))
    }

    private fun key(): SecretKey {
        val keyStore = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        (keyStore.getKey(KEY_ALIAS, null) as? SecretKey)?.let { return it }
        return KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore").run {
            init(
                KeyGenParameterSpec.Builder(
                    KEY_ALIAS,
                    KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT,
                )
                    .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                    .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                    .build(),
            )
            generateKey()
        }
    }

    private companion object {
        const val PAYLOAD_KEY = "gateway_identity"
        const val KEY_ALIAS = "dev.vifu.android.gateway.credentials.v1"
        const val TRANSFORMATION = "AES/GCM/NoPadding"
        const val IV_SIZE = 12
    }
}
