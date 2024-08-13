package net.nymtech.vpn

import android.content.Intent
import android.content.res.Resources
import android.os.Build
import androidx.annotation.CallSuper
import androidx.lifecycle.lifecycleScope
import com.zaneschepke.localizationutil.LocaleStorage
import com.zaneschepke.localizationutil.LocaleUtil
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import net.nymtech.vpn.tun_provider.TunConfig
import net.nymtech.vpn.util.Action
import net.nymtech.vpn.util.Constants
import net.nymtech.vpn.util.LifecycleVpnService
import nym_vpn_lib.stopVpn
import timber.log.Timber
import java.net.Inet4Address
import java.net.Inet6Address
import java.net.InetAddress

class NymVpnService : LifecycleVpnService() {

	private val ioDispatcher = Dispatchers.IO

	companion object {
		init {
			System.loadLibrary(Constants.NYM_VPN_LIB)
			Timber.i("Loaded native library in service")
		}
	}

	private var activeTunStatus: CreateTunResult? = null

	// Once we make sure Rust library doesn't close the fd first, we should re-use this code for closing fd,
	// as it's more general, including for wireguard tunnels
// 	private var activeTunStatus by observable<CreateTunResult?>(null) { _, oldTunStatus, _ ->
// 		val oldTunFd =
// 			when (oldTunStatus) {
// 				is CreateTunResult.Success -> oldTunStatus.tunFd
// 				is CreateTunResult.InvalidDnsServers -> oldTunStatus.tunFd
// 				else -> null
// 			}
// 		if (oldTunFd != null) {
// 			Timber.i("Closing file descriptor $oldTunFd")
// 			ParcelFileDescriptor.adoptFd(oldTunFd).close()
// 		}
// 	}

	private val tunIsOpen
		get() = activeTunStatus?.isOpen ?: false

	private var currentTunConfig = defaultTunConfig()

	private var tunIsStale = false

	protected var disallowedApps: List<String>? = null

	val connectivityListener = ConnectivityListener()

	@CallSuper
	override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
		LocaleUtil.applyLocalizedContext(baseContext, LocaleStorage(this).getPreferredLocale())
		when (intent?.action) {
			Action.START.name, Action.START_FOREGROUND.name -> {
				currentTunConfig = defaultTunConfig()
				Timber.i("VPN start called")
				if (prepare(this) == null) {
					startService()
				} else {
					stopService()
				}
			}
			Action.STOP.name, Action.STOP_FOREGROUND.name -> {
				Timber.d("Stopping VPN service")
				stopService()
			}
		}
		return super.onStartCommand(intent, flags, startId)
	}

	private fun startService() {
		synchronized(this) {
			lifecycleScope.launch(ioDispatcher) {
				val logLevel = if (BuildConfig.DEBUG) "debug" else "info"
				initVPN(this@NymVpnService, logLevel)
				NymBackend.connect()
			}
		}
	}

	private fun stopService() {
		stopForeground(STOP_FOREGROUND_REMOVE)
		lifecycleScope.launch(ioDispatcher) {
			runCatching {
				stopVpn()
			}.onFailure {
				Timber.e(it)
			}
		}
		stopSelf()
	}

	override fun getResources(): Resources {
		return if (Build.VERSION.SDK_INT > Build.VERSION_CODES.O) {
			super.getResources()
		} else {
			// before Android PIE we should override resources also
			LocaleUtil.getLocalizedResources(super.getResources(), LocaleStorage(this).getPreferredLocale())
		}
	}

	override fun onCreate() {
		super.onCreate()
		connectivityListener.register(this)
		startForeground()
	}

	private fun startForeground() {
		if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
			NotificationManager.createNotificationChannel(this@NymVpnService)
		}
		val notification = NotificationManager.createVpnRunningNotification(this@NymVpnService)
		startForeground(NotificationManager.VPN_NOTIFICATION_ID, notification)
	}

	override fun onDestroy() {
		super.onDestroy()
		connectivityListener.unregister()
		Timber.i("VpnService destroyed")
	}

	fun getTun(config: TunConfig): CreateTunResult {
		synchronized(this) {
			val tunStatus = activeTunStatus
			if (config == currentTunConfig && tunIsOpen && !tunIsStale) {
				return tunStatus!!
			} else {
				Timber.d("Creating new tunnel with config : $config")
				val newTunStatus = createTun(config)
				currentTunConfig = config
				activeTunStatus = newTunStatus
				tunIsStale = false

				return newTunStatus
			}
		}
	}

	fun createTun() {
		synchronized(this) { activeTunStatus = createTun(currentTunConfig) }
	}

	fun recreateTunIfOpen(config: TunConfig) {
		synchronized(this) {
			if (tunIsOpen) {
				currentTunConfig = config
				activeTunStatus = createTun(config)
			}
		}
	}

	fun closeTun() {
		Timber.d("CLOSE TUN CALLED")
		synchronized(this) {
			activeTunStatus = null
		}
	}

	fun markTunAsStale() {
		synchronized(this) {
			tunIsStale = true
		}
	}

	private fun createTun(config: TunConfig): CreateTunResult {
		if (prepare(this) != null) {
			Timber.w("VPN permission denied")
			// VPN permission wasn't granted
			return CreateTunResult.PermissionDenied
		}
		val invalidDnsServerAddresses = ArrayList<InetAddress>()
		val builder =
			Builder().apply {
				for (address in config.addresses) {
					addAddress(address, prefixForAddress(address))
				}

				for (dnsServer in config.dnsServers) {
					try {
						addDnsServer(dnsServer)
					} catch (exception: IllegalArgumentException) {
						Timber.e(exception)
						invalidDnsServerAddresses.add(dnsServer)
					}
				}
				for (route in config.routes) {
					addRoute(route.address, route.prefixLength.toInt())
				}
				disallowedApps?.let { apps ->
					for (app in apps) {
						addDisallowedApplication(app)
					}
				}
				setMtu(config.mtu)
				setBlocking(false)
				if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
					setMetered(false)
				}
			}
		val vpnInterface = builder.establish()
		val tunFd = vpnInterface?.detachFd() ?: return CreateTunResult.TunnelDeviceError
		waitForTunnelUp(tunFd, config.routes.any { route -> route.isIpv6 })

		if (invalidDnsServerAddresses.isNotEmpty()) {
			return CreateTunResult.InvalidDnsServers(invalidDnsServerAddresses, tunFd)
		}
		return CreateTunResult.Success(tunFd)
	}

	fun bypass(socket: Int): Boolean {
		return protect(socket)
	}

	private fun prefixForAddress(address: InetAddress): Int {
		return when (address) {
			is Inet4Address -> 32
			is Inet6Address -> 128
			else -> throw IllegalArgumentException("Invalid IP address (not IPv4 nor IPv6)")
		}
	}

	private external fun initVPN(vpn_service: Any, log_level: String)

	private external fun defaultTunConfig(): TunConfig

	private external fun waitForTunnelUp(tunFd: Int, isIpv6Enabled: Boolean)
}
