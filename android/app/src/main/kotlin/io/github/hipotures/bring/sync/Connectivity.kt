package io.github.hipotures.bring.sync

import android.content.Context
import android.net.ConnectivityManager
import android.net.NetworkCapabilities

/**
 * First half of LAN detection (§4), without location permissions: the
 * active network is unmetered Wi-Fi. The engine then probes `/health`.
 */
fun Context.onUnmeteredWifi(): Boolean {
    val cm = getSystemService(ConnectivityManager::class.java) ?: return false
    val caps = cm.getNetworkCapabilities(cm.activeNetwork) ?: return false
    return caps.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) &&
        caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_NOT_METERED)
}
