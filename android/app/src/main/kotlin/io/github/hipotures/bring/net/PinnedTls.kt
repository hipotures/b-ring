package io.github.hipotures.bring.net

import android.annotation.SuppressLint
import okhttp3.OkHttpClient
import java.security.MessageDigest
import java.security.cert.CertificateException
import java.security.cert.X509Certificate
import java.util.concurrent.ConcurrentHashMap
import javax.net.ssl.SSLContext
import javax.net.ssl.X509TrustManager

/**
 * Media3's OkHttp client for the LAN route (§3.1): trusts exactly the
 * certificate whose SHA-256 equals the pin from pairing. Like the engine's
 * verifier it ignores SAN/IP and the CA chain (a DHCP change must not break
 * playback); the TLS handshake still proves possession of the key.
 */
object PinnedTls {
    private val clients = ConcurrentHashMap<String, OkHttpClient>()
    private val plain by lazy { OkHttpClient() }

    /** The pinned client for [pinHex], or a standard WebPKI client for null (WAN). */
    fun client(pinHex: String?): OkHttpClient {
        if (pinHex == null) return plain
        return clients.getOrPut(pinHex.lowercase()) {
            val pin = hexToBytes(pinHex)
            val tm = PinTrustManager(pin)
            val ssl = SSLContext.getInstance("TLS").apply { init(null, arrayOf(tm), null) }
            plain.newBuilder()
                .sslSocketFactory(ssl.socketFactory, tm)
                .hostnameVerifier { _, session ->
                    val leaf = session.peerCertificates.firstOrNull() as? X509Certificate
                    leaf != null && matches(leaf, pin)
                }
                .build()
        }
    }

    fun sha256(der: ByteArray): ByteArray = MessageDigest.getInstance("SHA-256").digest(der)

    fun matches(cert: X509Certificate, pin: ByteArray): Boolean =
        MessageDigest.isEqual(sha256(cert.encoded), pin)

    fun hexToBytes(hex: String): ByteArray {
        require(hex.length == 64) { "pin must be 64 hex digits" }
        return ByteArray(32) { i -> hex.substring(2 * i, 2 * i + 2).toInt(16).toByte() }
    }

    /** Accepts only a chain whose leaf matches the pin. */
    @SuppressLint("CustomX509TrustManager")
    class PinTrustManager(private val pin: ByteArray) : X509TrustManager {
        override fun checkServerTrusted(chain: Array<out X509Certificate>?, authType: String?) {
            val leaf = chain?.firstOrNull() ?: throw CertificateException("no server certificate")
            if (!matches(leaf, pin)) throw CertificateException("certificate does not match the Ring's pin")
        }

        override fun checkClientTrusted(chain: Array<out X509Certificate>?, authType: String?) {
            throw CertificateException("client certificates are not used")
        }

        override fun getAcceptedIssuers(): Array<X509Certificate> = emptyArray()
    }
}
