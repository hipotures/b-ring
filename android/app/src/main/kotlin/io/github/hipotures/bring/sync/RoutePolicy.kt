package io.github.hipotures.bring.sync

import io.github.hipotures.bring.ffi.Route

/** Who started a sync. */
enum class Trigger { AUTO, MANUAL }

/** What the UI shows for the current route. */
enum class RouteStatus(val label: String) {
    UNKNOWN("····"),
    LAN("LAN"),
    WAN("WAN"),
    OFFLINE("OFFLINE"),
    ;

    companion object {
        fun of(route: Route?): RouteStatus = when (route) {
            Route.LAN -> LAN
            Route.WAN -> WAN
            null -> OFFLINE
        }
    }
}

object RoutePolicy {
    /**
     * §4: background sync runs only when the LAN check passed; over WAN
     * (cellular, foreign Wi-Fi) it runs only when the user asked for it.
     */
    fun mayRun(trigger: Trigger, route: Route?): Boolean = when (route) {
        null -> false
        Route.LAN -> true
        Route.WAN -> trigger == Trigger.MANUAL
    }
}
