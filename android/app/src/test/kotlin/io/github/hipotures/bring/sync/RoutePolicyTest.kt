package io.github.hipotures.bring.sync

import io.github.hipotures.bring.ffi.Route
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class RoutePolicyTest {
    @Test fun lanRunsForEveryTrigger() {
        assertTrue(RoutePolicy.mayRun(Trigger.AUTO, Route.LAN))
        assertTrue(RoutePolicy.mayRun(Trigger.MANUAL, Route.LAN))
    }

    @Test fun wanOnlyWhenTheUserAsked() {
        assertFalse(RoutePolicy.mayRun(Trigger.AUTO, Route.WAN))
        assertTrue(RoutePolicy.mayRun(Trigger.MANUAL, Route.WAN))
    }

    @Test fun unreachableRingNeverRuns() {
        assertFalse(RoutePolicy.mayRun(Trigger.AUTO, null))
        assertFalse(RoutePolicy.mayRun(Trigger.MANUAL, null))
    }

    @Test fun statusMirrorsTheRoute() {
        assertEquals(RouteStatus.LAN, RouteStatus.of(Route.LAN))
        assertEquals(RouteStatus.WAN, RouteStatus.of(Route.WAN))
        assertEquals(RouteStatus.OFFLINE, RouteStatus.of(null))
    }
}
