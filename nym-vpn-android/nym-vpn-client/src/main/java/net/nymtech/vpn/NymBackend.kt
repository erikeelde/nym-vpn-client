package net.nymtech.vpn

import android.content.Context
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import net.nymtech.vpn.model.BackendMessage
import net.nymtech.vpn.model.Statistics
import net.nymtech.vpn.util.Constants
import net.nymtech.vpn.util.InvalidCredentialException
import net.nymtech.vpn.util.ServiceManager
import net.nymtech.vpn.util.SingletonHolder
import nym_vpn_lib.BandwidthStatus
import nym_vpn_lib.ConnectionStatus
import nym_vpn_lib.ExitStatus
import nym_vpn_lib.FfiException
import nym_vpn_lib.NymVpnStatus
import nym_vpn_lib.TunStatus
import nym_vpn_lib.TunnelStatusListener
import nym_vpn_lib.VpnConfig
import nym_vpn_lib.checkCredential
import nym_vpn_lib.runVpn
import timber.log.Timber
import java.time.Instant

class NymBackend private constructor(val context: Context) : Backend, TunnelStatusListener {

	companion object : SingletonHolder<NymBackend, Context>(::NymBackend)
	private val ioDispatcher = Dispatchers.IO

	private var statsJob: Job? = null
	private var tunnel: Tunnel? = null
	private var state: Tunnel.State = Tunnel.State.Down

	override suspend fun validateCredential(credential: String): Instant? {
		return try {
			withContext(ioDispatcher) {
				checkCredential(credential)
			}
		} catch (e: FfiException) {
			Timber.e(e)
			throw InvalidCredentialException("Credential invalid or expired")
		}
	}

	override suspend fun importCredential(credential: String): Instant? {
		return try {
			nym_vpn_lib.importCredential(credential, Constants.NATIVE_STORAGE_PATH)
		} catch (e: FfiException) {
			Timber.e(e)
			throw InvalidCredentialException("Credential invalid or expired")
		}
	}

	override fun start(tunnel: Tunnel): Tunnel.State {
		this.tunnel = tunnel
		tunnel.environment.setup()
		// reset any error state
		tunnel.onBackendMessage(BackendMessage.None)
		ServiceManager.startVpnServiceForeground(context)
		return Tunnel.State.Connecting.InitializingClient
	}

	override fun stop(): Tunnel.State {
		ServiceManager.stopVpnServiceForeground(context)
		return Tunnel.State.Disconnecting
	}

	private fun onDisconnect() {
		statsJob?.cancel()
		tunnel?.onStatisticChange(Statistics())
	}

	private fun onConnect() = CoroutineScope(ioDispatcher).launch {
		startConnectionTimer()
	}

	override fun getState(): Tunnel.State {
		return state
	}

	private fun isTwoHop(mode: Tunnel.Mode): Boolean = when (mode) {
		Tunnel.Mode.TWO_HOP_MIXNET -> true
		else -> false
	}

	internal suspend fun connect() {
		withContext(ioDispatcher) {
			tunnel?.let {
				runCatching {
					runVpn(
						VpnConfig(
							it.environment.apiUrl,
							it.environment.nymVpnApiUrl,
							it.entryPoint,
							it.exitPoint,
							isTwoHop(it.mode),
							Constants.NATIVE_STORAGE_PATH,
							this@NymBackend,
						),
					)
				}.onFailure {
					// temp for now until we setup error/message callback
					tunnel?.onBackendMessage(BackendMessage.Error.StartFailed)
				}
			}
		}
	}

	private suspend fun startConnectionTimer() {
		withContext(ioDispatcher) {
			var seconds = 0L
			do {
				if (state == Tunnel.State.Up) {
					tunnel?.onStatisticChange(Statistics(seconds))
					seconds++
				}
				delay(Constants.STATISTICS_INTERVAL_MILLI)
			} while (true)
		}
	}

	override fun onTunStatusChange(status: TunStatus) {
		val state = when (status) {
			TunStatus.INITIALIZING_CLIENT -> Tunnel.State.Connecting.InitializingClient
			TunStatus.ESTABLISHING_CONNECTION -> Tunnel.State.Connecting.EstablishingConnection
			TunStatus.DOWN -> {
				Tunnel.State.Down
			}
			TunStatus.UP -> {
				statsJob = onConnect()
				Tunnel.State.Up
			}
			TunStatus.DISCONNECTING -> {
				onDisconnect()
				Tunnel.State.Disconnecting
			}
		}
		this.state = state
		tunnel?.onStateChange(state)
	}

	override fun onBandwidthStatusChange(status: BandwidthStatus) {
		Timber.d("Bandwidth status: $status")
	}

	override fun onConnectionStatusChange(status: ConnectionStatus) {
		Timber.d("Connection status: $status")
	}

	override fun onNymVpnStatusChange(status: NymVpnStatus) {
		Timber.d("VPN status: $status")
	}

	override fun onExitStatusChange(status: ExitStatus) {
		when (status) {
			ExitStatus.Stopped -> Timber.d("Tunnel stopped")
			is ExitStatus.Failed -> {
				Timber.e(status.error)
				// need to stop the vpn service even though vpn never started from lib perspective
				stop()
				tunnel?.onBackendMessage(BackendMessage.Error.StartFailed)
				// Need to set state down because this likely never happened in lib
				tunnel?.onStateChange(Tunnel.State.Down)
			}
		}
	}
}