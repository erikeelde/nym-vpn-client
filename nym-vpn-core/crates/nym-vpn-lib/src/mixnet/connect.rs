// Copyright 2023 - Nym Technologies SA <contact@nymtech.net>
// SPDX-License-Identifier: GPL-3.0-only

use std::{path::PathBuf, result::Result};

use nym_config::defaults::NymNetworkDetails;
use nym_sdk::mixnet::{MixnetClientBuilder, NodeIdentity, StoragePaths};
use tracing::{debug, info};

use super::{MixnetError, SharedMixnetClient};
use crate::vpn::MixnetClientConfig;

fn true_to_enabled(val: bool) -> &'static str {
    if val {
        "enabled"
    } else {
        "disabled"
    }
}

fn true_to_disabled(val: bool) -> &'static str {
    if val {
        "disabled"
    } else {
        "enabled"
    }
}

fn apply_mixnet_client_config(
    mixnet_client_config: &MixnetClientConfig,
    debug_config: &mut nym_client_core::config::DebugConfig,
) {
    let MixnetClientConfig {
        enable_poisson_rate,
        disable_background_cover_traffic,
        enable_credentials_mode: _enable_credentials_mode,
        min_mixnode_performance,
        min_gateway_performance,
    } = mixnet_client_config;

    // Disable Poisson rate limiter by default
    info!(
        "mixnet client poisson rate limiting: {}",
        true_to_enabled(*enable_poisson_rate)
    );
    debug_config
        .traffic
        .disable_main_poisson_packet_distribution = !enable_poisson_rate;

    info!(
        "mixnet client background loop cover traffic stream: {}",
        true_to_disabled(*disable_background_cover_traffic)
    );
    debug_config.cover_traffic.disable_loop_cover_traffic_stream =
        *disable_background_cover_traffic;

    if let Some(min_mixnode_performance) = min_mixnode_performance {
        debug_config.topology.minimum_mixnode_performance = *min_mixnode_performance;
    }
    info!(
        "mixnet client minimum mixnode performance: {}",
        debug_config.topology.minimum_mixnode_performance,
    );

    if let Some(min_gateway_performance) = min_gateway_performance {
        debug_config.topology.minimum_gateway_performance = *min_gateway_performance;
    }
    info!(
        "mixnet client minimum gateway performance: {}",
        debug_config.topology.minimum_gateway_performance,
    );
}

pub(crate) async fn setup_mixnet_client(
    mixnet_entry_gateway: &NodeIdentity,
    mixnet_client_key_storage_path: &Option<PathBuf>,
    mut task_client: nym_task::TaskClient,
    mixnet_client_config: MixnetClientConfig,
) -> Result<SharedMixnetClient, MixnetError> {
    let mut debug_config = nym_client_core::config::DebugConfig::default();
    apply_mixnet_client_config(&mixnet_client_config, &mut debug_config);

    let user_agent = nym_bin_common::bin_info_owned!().into();

    let mixnet_client = if let Some(path) = mixnet_client_key_storage_path {
        debug!("Using custom key storage path: {:?}", path);

        let gateway_id = mixnet_entry_gateway.to_base58_string();
        if let Err(err) =
            crate::credentials::check_imported_credential(path.to_path_buf(), &gateway_id).await
        {
            // UGLY: flow needs to restructured to sort this out, but I don't want to refactor all
            // that just before release.
            task_client.disarm();
            return Err(MixnetError::InvalidCredential {
                reason: err,
                path: path.to_path_buf(),
                gateway_id,
            });
        };

        let key_storage_path = StoragePaths::new_from_dir(path)
            .map_err(MixnetError::FailedToSetupMixnetStoragePaths)?;
        MixnetClientBuilder::new_with_default_storage(key_storage_path)
            .await
            .map_err(MixnetError::FailedToCreateMixnetClientWithDefaultStorage)?
            .with_user_agent(user_agent)
            .request_gateway(mixnet_entry_gateway.to_string())
            .network_details(NymNetworkDetails::new_from_env())
            .debug_config(debug_config)
            .custom_shutdown(task_client)
            .credentials_mode(mixnet_client_config.enable_credentials_mode)
            .build()
            .map_err(MixnetError::FailedToBuildMixnetClient)?
            .connect_to_mixnet()
            .await
            .map_err(map_mixnet_connect_error)?
    } else {
        debug!("Using ephemeral key storage");
        MixnetClientBuilder::new_ephemeral()
            .with_user_agent(user_agent)
            .request_gateway(mixnet_entry_gateway.to_string())
            .network_details(NymNetworkDetails::new_from_env())
            .debug_config(debug_config)
            .custom_shutdown(task_client)
            .credentials_mode(mixnet_client_config.enable_credentials_mode)
            .build()
            .map_err(MixnetError::FailedToBuildMixnetClient)?
            .connect_to_mixnet()
            .await
            .map_err(map_mixnet_connect_error)?
    };

    Ok(SharedMixnetClient::new(mixnet_client))
}

// Map some specific mixnet errors to more specific ones
fn map_mixnet_connect_error(err: nym_sdk::Error) -> MixnetError {
    match err {
        nym_sdk::Error::ClientCoreError(
            nym_client_core::error::ClientCoreError::GatewayClientError { gateway_id, source },
        ) => MixnetError::EntryGateway {
            gateway_id: gateway_id.to_string(),
            source: Box::new(source),
        },
        _ => MixnetError::FailedToConnectToMixnet(err),
    }
}
