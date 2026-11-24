package io.github.hipotures.bring.ui

import io.github.hipotures.bring.ffi.EntryState
import org.junit.Assert.assertEquals
import org.junit.Test

class BadgesTest {
    @Test fun stateBadgeComesFirst() {
        assertEquals(listOf(Badge.LOCAL), badgesFor(EntryState.LOCAL_ONLY, false, null, false))
        assertEquals(listOf(Badge.SYNCED), badgesFor(EntryState.SYNCED, false, null, false))
        assertEquals(listOf(Badge.REMOTE), badgesFor(EntryState.REMOTE_ONLY, false, null, false))
    }

    @Test fun flagsFollowInFixedOrder() {
        assertEquals(
            listOf(Badge.LOCAL, Badge.UPLOAD, Badge.ERROR, Badge.HELD),
            badgesFor(EntryState.LOCAL_ONLY, pending = true, error = "boom", suppressed = true),
        )
    }

    @Test fun labelsAreDistinct() {
        assertEquals(Badge.entries.size, Badge.entries.map { it.label }.toSet().size)
    }
}
