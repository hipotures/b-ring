package io.github.hipotures.bring

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import androidx.core.content.edit
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/**
 * Per-Ring device tokens, encrypted with an AES-GCM key that never leaves the
 * Android Keystore. The engine never writes tokens to SQLite (§4.1).
 */
class TokenStore(context: Context) {
    private val prefs = context.getSharedPreferences("device_token", Context.MODE_PRIVATE)

    fun save(ringId: String, token: String) {
        val cipher = Cipher.getInstance(TRANSFORMATION)
        cipher.init(Cipher.ENCRYPT_MODE, key())
        val ct = cipher.doFinal(token.toByteArray(Charsets.UTF_8))
        prefs.edit(commit = true) {
            putString(ivKey(ringId), Base64.encodeToString(cipher.iv, Base64.NO_WRAP))
            putString(ctKey(ringId), Base64.encodeToString(ct, Base64.NO_WRAP))
        }
    }

    /** The stored token for [ringId], or null if missing or undecryptable. */
    fun load(ringId: String): String? = decrypt(ivKey(ringId), ctKey(ringId))

    /**
     * Migrates a legacy single-Ring token (`iv`/`ct`) to [ringId] once, so an
     * existing paired device stays paired seamlessly after upgrading to multi-Ring.
     */
    fun migrateLegacy(ringId: String): String? {
        load(ringId)?.let { return it }
        val legacy = decrypt(LEGACY_IV, LEGACY_CT) ?: return null
        save(ringId, legacy)
        prefs.edit(commit = true) {
            remove(LEGACY_IV)
            remove(LEGACY_CT)
        }
        return legacy
    }

    fun remove(ringId: String) {
        prefs.edit(commit = true) {
            remove(ivKey(ringId))
            remove(ctKey(ringId))
        }
    }

    fun hasAny(): Boolean = prefs.all.keys.any { it == LEGACY_CT || it.startsWith(CT_PREFIX) }

    fun clear() {
        prefs.edit(commit = true) { clear() }
        runCatching { keyStore().deleteEntry(ALIAS) }
    }

    private fun decrypt(ivKey: String, ctKey: String): String? {
        val iv = prefs.getString(ivKey, null) ?: return null
        val ct = prefs.getString(ctKey, null) ?: return null
        return runCatching {
            val cipher = Cipher.getInstance(TRANSFORMATION)
            cipher.init(
                Cipher.DECRYPT_MODE,
                key(),
                GCMParameterSpec(128, Base64.decode(iv, Base64.NO_WRAP)),
            )
            String(cipher.doFinal(Base64.decode(ct, Base64.NO_WRAP)), Charsets.UTF_8)
        }.getOrNull()
    }

    private fun ivKey(ringId: String) = "$IV_PREFIX$ringId"
    private fun ctKey(ringId: String) = "$CT_PREFIX$ringId"

    private fun keyStore() = KeyStore.getInstance(KEYSTORE).apply { load(null) }

    private fun key(): SecretKey {
        (keyStore().getKey(ALIAS, null) as? SecretKey)?.let { return it }
        val spec = KeyGenParameterSpec.Builder(
            ALIAS,
            KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT,
        )
            .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
            .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
            .setKeySize(256)
            .build()
        return KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, KEYSTORE)
            .apply { init(spec) }
            .generateKey()
    }

    private companion object {
        const val KEYSTORE = "AndroidKeyStore"
        const val ALIAS = "bring-device-token"
        const val TRANSFORMATION = "AES/GCM/NoPadding"
        const val LEGACY_IV = "iv"
        const val LEGACY_CT = "ct"
        const val IV_PREFIX = "iv_"
        const val CT_PREFIX = "ct_"
    }
}
