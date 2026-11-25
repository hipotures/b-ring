package io.github.hipotures.bring.pairing

import android.Manifest
import android.annotation.SuppressLint
import android.bluetooth.BluetoothDevice
import android.bluetooth.BluetoothGatt
import android.bluetooth.BluetoothGattCallback
import android.bluetooth.BluetoothGattCharacteristic
import android.bluetooth.BluetoothManager
import android.bluetooth.BluetoothProfile
import android.bluetooth.le.ScanCallback
import android.bluetooth.le.ScanFilter
import android.bluetooth.le.ScanResult
import android.bluetooth.le.ScanSettings
import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import android.os.ParcelUuid
import io.github.hipotures.bring.ffi.blePairingPayloadUuid
import io.github.hipotures.bring.ffi.blePairingServiceUuid
import io.github.hipotures.bring.ffi.pairingCodeFromBytes
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.withTimeout
import java.util.UUID
import kotlin.coroutines.resume

/**
 * Reads the pairing code from a Ring running `bring pair --ble`
 * (docs/ARCHITECTURE.md §3.6): scan for its GATT service, connect, read one
 * characteristic, disconnect. No Bluetooth bonding, so nothing shows up in the
 * system's paired-devices list.
 */
object BlePairingReader {
    private val serviceUuid: UUID by lazy { UUID.fromString(blePairingServiceUuid()) }
    private val payloadUuid: UUID by lazy { UUID.fromString(blePairingPayloadUuid()) }
    private const val SCAN_TIMEOUT_MS = 30_000L
    private const val READ_TIMEOUT_MS = 15_000L

    fun available(context: Context): Boolean =
        context.packageManager.hasSystemFeature(PackageManager.FEATURE_BLUETOOTH_LE)

    /** Runtime permissions for scanning and connecting. */
    val permissions: Array<String> =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            arrayOf(Manifest.permission.BLUETOOTH_SCAN, Manifest.permission.BLUETOOTH_CONNECT)
        } else {
            // Android 11 only scans BLE with location access (maxSdkVersion 30).
            arrayOf(Manifest.permission.ACCESS_FINE_LOCATION)
        }

    class BleException(message: String) : Exception(message)

    /** Suspends until the code is read; [onStatus] narrates progress. */
    @SuppressLint("MissingPermission") // The caller requests [permissions] first.
    suspend fun read(context: Context, onStatus: (String) -> Unit): String {
        val manager = context.getSystemService(BluetoothManager::class.java)
        val adapter = manager?.adapter ?: throw BleException("This device has no Bluetooth")
        if (!adapter.isEnabled) throw BleException("Bluetooth is off. Turn it on and try again.")
        val scanner = adapter.bluetoothLeScanner ?: throw BleException("Bluetooth is off. Turn it on and try again.")

        onStatus("Looking for a Ring…")
        val device = try {
            withTimeout(SCAN_TIMEOUT_MS) { scan(scanner) }
        } catch (_: kotlinx.coroutines.TimeoutCancellationException) {
            throw BleException("No Ring found. Is `bring pair --ble` running nearby?")
        }
        onStatus("Reading the code from ${device.address}…")
        val bytes = try {
            withTimeout(READ_TIMEOUT_MS) { readPayload(context, device) }
        } catch (_: kotlinx.coroutines.TimeoutCancellationException) {
            throw BleException("The Ring did not answer over Bluetooth")
        }
        return pairingCodeFromBytes(bytes) ?: throw BleException("The Ring sent an invalid code")
    }

    @SuppressLint("MissingPermission")
    private suspend fun scan(scanner: android.bluetooth.le.BluetoothLeScanner): BluetoothDevice =
        suspendCancellableCoroutine { cont ->
            val callback = object : ScanCallback() {
                override fun onScanResult(callbackType: Int, result: ScanResult) {
                    if (cont.isActive) {
                        runCatching { scanner.stopScan(this) }
                        cont.resume(result.device)
                    }
                }

                override fun onScanFailed(errorCode: Int) {
                    if (cont.isActive) {
                        cont.resumeWith(Result.failure(BleException("Bluetooth scan failed ($errorCode)")))
                    }
                }
            }
            val filter = ScanFilter.Builder().setServiceUuid(ParcelUuid(serviceUuid)).build()
            val settings = ScanSettings.Builder().setScanMode(ScanSettings.SCAN_MODE_LOW_LATENCY).build()
            scanner.startScan(listOf(filter), settings, callback)
            cont.invokeOnCancellation { runCatching { scanner.stopScan(callback) } }
        }

    @SuppressLint("MissingPermission")
    private suspend fun readPayload(context: Context, device: BluetoothDevice): ByteArray {
        val result = CompletableDeferred<ByteArray>()
        val callback = object : BluetoothGattCallback() {
            override fun onConnectionStateChange(gatt: BluetoothGatt, status: Int, newState: Int) {
                when {
                    status != BluetoothGatt.GATT_SUCCESS ->
                        result.completeExceptionally(BleException("Bluetooth connection failed ($status)"))
                    newState == BluetoothProfile.STATE_CONNECTED -> {
                        // 31 bytes fit one read once the MTU is above the 23-byte default.
                        if (!gatt.requestMtu(64)) gatt.discoverServices()
                    }
                    newState == BluetoothProfile.STATE_DISCONNECTED ->
                        result.completeExceptionally(BleException("The Ring closed the Bluetooth connection"))
                }
            }

            override fun onMtuChanged(gatt: BluetoothGatt, mtu: Int, status: Int) {
                gatt.discoverServices()
            }

            override fun onServicesDiscovered(gatt: BluetoothGatt, status: Int) {
                val ch = gatt.getService(serviceUuid)?.getCharacteristic(payloadUuid)
                if (ch == null || !gatt.readCharacteristic(ch)) {
                    result.completeExceptionally(BleException("This device is not a Ring waiting for pairing"))
                }
            }

            override fun onCharacteristicRead(
                gatt: BluetoothGatt,
                characteristic: BluetoothGattCharacteristic,
                value: ByteArray,
                status: Int,
            ) {
                if (status == BluetoothGatt.GATT_SUCCESS) {
                    result.complete(value)
                } else {
                    result.completeExceptionally(BleException("Reading the code failed ($status)"))
                }
            }

            @Deprecated("Called instead of the ByteArray overload before Android 13")
            override fun onCharacteristicRead(
                gatt: BluetoothGatt,
                characteristic: BluetoothGattCharacteristic,
                status: Int,
            ) {
                @Suppress("DEPRECATION")
                onCharacteristicRead(gatt, characteristic, characteristic.value ?: ByteArray(0), status)
            }
        }
        val gatt = device.connectGatt(context, false, callback, BluetoothDevice.TRANSPORT_LE)
            ?: throw BleException("Bluetooth connection failed")
        try {
            return result.await()
        } finally {
            runCatching { gatt.disconnect() }
            gatt.close()
        }
    }
}
