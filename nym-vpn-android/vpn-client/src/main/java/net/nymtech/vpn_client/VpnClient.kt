package net.nymtech.vpn_client

import android.content.Context
import android.content.Intent
import android.net.VpnService
import kotlinx.coroutines.flow.Flow
import net.nymtech.NymVpnService

interface VpnClient {
    fun prepare(context : Context) : Intent?
    fun connect(entryIso: String, exitIso: String, vpnService: NymVpnService, interfaceFd : Int)
    fun disconnect()
    val statistics : Flow<VpnStatistics>
}