package io.github.hipotures.bring.net

import org.junit.Assert.assertSame
import org.junit.Assert.assertNotSame
import org.junit.Test
import java.security.cert.CertificateException
import java.security.cert.CertificateFactory
import java.security.cert.X509Certificate

class PinnedTlsTest {
    private val cert = javaClass.classLoader!!.getResourceAsStream("pin-test-cert.pem").use {
        CertificateFactory.getInstance("X.509").generateCertificate(it) as X509Certificate
    }

    /** `openssl x509 -outform DER | sha256sum` of the fixture. */
    private val pinHex = "bf6622e801dae4048c1fdd5d612521b76f2682fc14f08bf4c2e625673d745b24"

    @Test fun matchingPinIsTrusted() {
        PinnedTls.PinTrustManager(PinnedTls.hexToBytes(pinHex)).checkServerTrusted(arrayOf(cert), "ECDHE_ECDSA")
    }

    @Test(expected = CertificateException::class)
    fun otherPinIsRejected() {
        val wrong = pinHex.replaceRange(0, 2, "00")
        PinnedTls.PinTrustManager(PinnedTls.hexToBytes(wrong)).checkServerTrusted(arrayOf(cert), "ECDHE_ECDSA")
    }

    @Test(expected = CertificateException::class)
    fun emptyChainIsRejected() {
        PinnedTls.PinTrustManager(PinnedTls.hexToBytes(pinHex)).checkServerTrusted(emptyArray(), "RSA")
    }

    @Test(expected = IllegalArgumentException::class)
    fun malformedPinIsRefused() {
        PinnedTls.hexToBytes("abcd")
    }

    @Test fun clientsAreCachedPerPinAndCaseInsensitive() {
        val a = PinnedTls.client(pinHex)
        assertSame(a, PinnedTls.client(pinHex.uppercase()))
        assertNotSame(a, PinnedTls.client(null))
    }
}
