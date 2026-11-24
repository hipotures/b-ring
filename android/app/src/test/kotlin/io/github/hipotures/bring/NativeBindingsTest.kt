package io.github.hipotures.bring

import io.github.hipotures.bring.ffi.Moon
import io.github.hipotures.bring.ffi.MoonException
import io.github.hipotures.bring.ffi.coreVersion
import io.github.hipotures.bring.ffi.isValidPairingCode
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder

/**
 * Calls the real engine (host build of libbring_uniffi.so) through the
 * generated bindings: proves they compile, load via JNA and round-trip
 * records and errors.
 */
class NativeBindingsTest {
    @get:Rule val tmp = TemporaryFolder()

    @Test fun coreVersionIsReported() {
        assertTrue(coreVersion().isNotBlank())
    }

    @Test fun malformedPairingCodeIsRejectedLocally() {
        assertFalse(isValidPairingCode("not a code"))
        assertFalse(isValidPairingCode(""))
    }

    @Test fun freshMoonIsUnpairedAndEmpty() {
        Moon.open(tmp.root.resolve("moon.db").path).use { moon ->
            assertNull(moon.ring())
            assertEquals(emptyList<Any>(), moon.entries(null))
            assertEquals(emptyList<Any>(), moon.watchedFolders())
        }
    }

    @Test fun syncWithoutPairingRaisesNotPaired() {
        Moon.open(tmp.root.resolve("moon.db").path).use { moon ->
            try {
                moon.syncIndex()
                fail("expected NotPaired")
            } catch (e: MoonException.NotPaired) {
                // expected
            }
        }
    }

    @Test fun invalidInputCarriesItsMessage() {
        Moon.open(tmp.root.resolve("moon.db").path).use { moon ->
            try {
                moon.thumbnail("xyz")
                fail("expected Invalid")
            } catch (e: MoonException.Invalid) {
                assertTrue(e.toString(), e.message.orEmpty().contains("thumbnail"))
            }
        }
    }
}
