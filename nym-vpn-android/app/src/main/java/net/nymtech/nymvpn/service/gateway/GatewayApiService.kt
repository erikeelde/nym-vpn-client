package net.nymtech.nymvpn.service.gateway

import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.withContext
import net.nymtech.nymvpn.module.IoDispatcher
import net.nymtech.vpn.model.Country
import timber.log.Timber
import javax.inject.Inject

class GatewayApiService @Inject constructor(
	private val gatewayApi: GatewayApi,
	private val gatewayLibService: GatewayLibService,
	@IoDispatcher private val ioDispatcher: CoroutineDispatcher,
) : GatewayService {

	override suspend fun getLowLatencyCountry(): Result<Country> {
		return withContext(ioDispatcher) {
			gatewayLibService.getLowLatencyCountry()
		}
	}

	override suspend fun getCountries(exitOnly: Boolean): Result<Set<Country>> {
		Timber.d("Getting countries from nym api")
		return safeApiCall {
			withContext(ioDispatcher) {
				val countries = if (exitOnly) {
					gatewayApi.getAllExitGatewayTwoCharacterCountryCodes()
				} else {
					gatewayApi.getAllEntryGatewayTwoCharacterCountryCodes()
				}
				countries.map { Country(it) }.toSet()
			}
		}
	}
}
