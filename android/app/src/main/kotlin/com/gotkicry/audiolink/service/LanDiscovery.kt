package com.gotkicry.audiolink.service

import android.content.Context
import android.net.wifi.WifiManager
import android.util.Log
import android.os.SystemClock
import com.gotkicry.audiolink.core.DiscoveryBrowser
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.flow
import kotlinx.coroutines.flow.flowOn
import kotlinx.coroutines.isActive

data class LanHost(val id: String, val name: String, val addr: String, val compatible: Boolean)
data class LanDiscoveryState(
    val hosts: List<LanHost> = emptyList(),
    val failed: Boolean = false,
    val scanning: Boolean = true,
)

/** Cold flow: a foreground receiver collects it; cancellation releases Wi-Fi and UDP resources. */
fun discoverLanHosts(context: Context): Flow<LanDiscoveryState> = flow {
    val wifi = context.applicationContext.getSystemService(WifiManager::class.java)
    while (currentCoroutineContext().isActive) {
        emit(LanDiscoveryState())
        var browser: DiscoveryBrowser? = null
        var lock: WifiManager.MulticastLock? = null
        try {
            lock = wifi?.createMulticastLock("AudioLink discovery")?.apply {
                setReferenceCounted(false)
                acquire()
            }
            browser = DiscoveryBrowser()
            val started = SystemClock.elapsedRealtime()
            while (currentCoroutineContext().isActive) {
                val hosts = browser.hosts().map {
                    LanHost(it.idShort, it.name, it.addr, it.compatible)
                }
                emit(LanDiscoveryState(hosts = hosts, scanning = hosts.isEmpty() && SystemClock.elapsedRealtime() - started < 3500))
                delay(1000)
            }
        } catch (cancelled: CancellationException) {
            throw cancelled
        } catch (error: Exception) {
            Log.w("LanDiscovery", "Could not browse LAN hosts", error)
            emit(LanDiscoveryState(failed = true, scanning = false))
        } catch (error: LinkageError) {
            Log.e("LanDiscovery", "Native discovery is unavailable", error)
            emit(LanDiscoveryState(failed = true, scanning = false))
        } finally {
            // Rust stop joins its worker; this flow runs on IO, never the UI thread.
            try { browser?.stop() } finally {
                try { browser?.close() } finally {
                    if (lock?.isHeld == true) lock.release()
                }
            }
        }
        delay(3000)
    }
}.flowOn(Dispatchers.IO)
