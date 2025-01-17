// Copyright 2024 - Nym Technologies SA <contact@nymtech.net>
// SPDX-License-Identifier: GPL-3.0-only

use tokio::sync::mpsc::UnboundedReceiver;
use tracing::{error, info};

use super::{vpn_service::NymVpnService, VpnServiceCommand, VpnServiceStateChange};

pub(crate) fn start_vpn_service(
    vpn_state_changes_tx: tokio::sync::broadcast::Sender<VpnServiceStateChange>,
    vpn_command_rx: UnboundedReceiver<VpnServiceCommand>,
    mut task_client: nym_task::TaskClient,
) -> std::thread::JoinHandle<()> {
    info!("Starting VPN service");

    // TODO: join up the task handling in vpn library with the daemon
    task_client.disarm();

    std::thread::spawn(move || {
        let vpn_rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        vpn_rt.block_on(async {
            let service = NymVpnService::new(vpn_state_changes_tx, vpn_command_rx);
            match service.init_storage().await {
                Ok(()) => {
                    info!("VPN service initialized successfully");
                    service.run().await.ok();
                }
                Err(err) => {
                    error!("Failed to initialize VPN service: {:?}", err);
                }
            }

            // The task handling of the vpn libary is not yet integrated with the daemon task
            // handling, so sleep here to give the mixnet client a sporting chance to flush data to
            // storage.
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        });
    })
}
