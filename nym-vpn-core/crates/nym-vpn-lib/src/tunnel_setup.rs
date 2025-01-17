// Copyright 2024 - Nym Technologies SA <contact@nymtech.net>
// SPDX-License-Identifier: GPL-3.0-only

use std::{net::IpAddr, time::Duration};

use futures::{
    channel::{mpsc, oneshot},
    StreamExt,
};
use ipnetwork::IpNetwork;
use log::*;
use nym_authenticator_client::AuthClient;
use nym_gateway_directory::{AuthAddresses, GatewayClient, IpPacketRouterAddress};
use nym_task::TaskManager;
use nym_wg_gateway_client::WgGatewayClient;
use talpid_core::dns::DnsMonitor;
use talpid_routing::{Node, RequiredRoute, RouteManager};
use talpid_tunnel::{TunnelEvent, TunnelMetadata};
use tokio::time::timeout;

use crate::{
    bandwidth_controller::BandwidthController,
    error::{Error, GatewayDirectoryError, Result, SetupMixTunnelError, SetupWgTunnelError},
    mixnet, platform,
    routing::{self, catch_all_ipv4, catch_all_ipv6, replace_default_prefixes},
    uniffi_custom_impls::{StatusEvent, TunStatus},
    vpn::{
        MixnetConnectionInfo, MixnetExitConnectionInfo, MixnetVpn, NymVpn, SpecificVpn,
        WireguardConnectionInfo, WireguardVpn, MIXNET_CLIENT_STARTUP_TIMEOUT_SECS,
    },
    wireguard_config,
    wireguard_setup::create_wireguard_tunnel,
};

pub(crate) struct TunnelSetup<T: TunnelSpecifcSetup> {
    pub(crate) specific_setup: T,
}

pub(crate) trait TunnelSpecifcSetup {}

pub(crate) struct MixTunnelSetup {
    pub(crate) mixnet_connection_info: MixnetConnectionInfo,
    pub(crate) exit_connection_info: MixnetExitConnectionInfo,
}

impl TunnelSpecifcSetup for MixTunnelSetup {}

pub(crate) struct WgTunnelSetup {
    pub(crate) connection_info: WireguardConnectionInfo,

    pub(crate) receiver: oneshot::Receiver<()>,
    pub(crate) handle: tokio::task::JoinHandle<()>,
    pub(crate) tunnel_close_tx: oneshot::Sender<()>,
}

impl TunnelSpecifcSetup for WgTunnelSetup {}

#[allow(clippy::large_enum_variant)]
pub(crate) enum AllTunnelsSetup {
    Mix(TunnelSetup<MixTunnelSetup>),
    Wg {
        entry: TunnelSetup<WgTunnelSetup>,
        exit: TunnelSetup<WgTunnelSetup>,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum WaitInterfaceUpError {
    #[error("auth failed")]
    AuthFailed,
    #[error("interface down")]
    Down,
    #[error("event tunnel closed")]
    EventTunnelClose,
}

async fn wait_interface_up(
    mut event_rx: mpsc::UnboundedReceiver<(TunnelEvent, oneshot::Sender<()>)>,
) -> std::result::Result<TunnelMetadata, WaitInterfaceUpError> {
    loop {
        match event_rx.next().await {
            Some((TunnelEvent::InterfaceUp(_, _), _)) => {
                debug!("Received interface up event");
                continue;
            }
            Some((TunnelEvent::Up(metadata), _)) => {
                debug!("Received up event");
                break Ok(metadata);
            }
            Some((TunnelEvent::AuthFailed(_), _)) => {
                debug!("Received tunnel auth failed");
                return Err(WaitInterfaceUpError::AuthFailed);
            }
            Some((TunnelEvent::Down, _)) => {
                debug!("Received tunnel down event when waiting for interface up");
                return Err(WaitInterfaceUpError::Down);
            }
            None => {
                debug!("Wireguard event channel closed when waiting for interface up");
                return Err(WaitInterfaceUpError::EventTunnelClose);
            }
        }
    }
}

async fn setup_wg_tunnel(
    nym_vpn: &mut NymVpn<WireguardVpn>,
    mixnet_client: mixnet::SharedMixnetClient,
    task_manager: &mut TaskManager,
    route_manager: &mut RouteManager,
    gateway_directory_client: GatewayClient,
    auth_addresses: AuthAddresses,
    default_lan_gateway_ip: routing::LanGatewayIp,
) -> std::result::Result<AllTunnelsSetup, SetupWgTunnelError> {
    // MTU is computed as (MTU of wire interface) - ((IP header size) + (UDP header size) + (WireGuard metadata size))
    // The IP header size is 20 for IPv4 and 40 for IPv6
    // The UDP header size is 8
    // The Wireguard metadata size is 32
    // Entry tunnel will only deal with IPv4 => 1500 - (20 + 8 + 32)
    let entry_mtu = 1440;
    // Exit tunnel will deal with both v4 and v6, and it's "wire" interface is entry tunnel's MTU
    // 1440 - (40 + 8 + 32)
    let exit_mtu = 1360;

    let bandwidth_controller =
        BandwidthController::new(mixnet_client.clone(), task_manager.subscribe());
    tokio::spawn(bandwidth_controller.run());

    let (Some(entry_auth_recipient), Some(exit_auth_recipient)) =
        (auth_addresses.entry().0, auth_addresses.exit().0)
    else {
        return Err(SetupWgTunnelError::AuthenticationNotPossible(
            auth_addresses.to_string(),
        ));
    };
    let auth_client = AuthClient::new_from_inner(mixnet_client.inner()).await;
    log::info!("Created wg gateway clients");
    let mut wg_entry_gateway_client = WgGatewayClient::new_entry(
        &nym_vpn.generic_config.data_path,
        auth_client.clone(),
        entry_auth_recipient,
    );
    let mut wg_exit_gateway_client = WgGatewayClient::new_exit(
        &nym_vpn.generic_config.data_path,
        auth_client.clone(),
        exit_auth_recipient,
    );

    let (mut exit_wireguard_config, _) = wireguard_config::init_wireguard_config(
        &gateway_directory_client,
        &mut wg_exit_gateway_client,
        None,
        exit_mtu,
    )
    .await?;
    let wg_gateway = exit_wireguard_config
        .talpid_config
        .peers
        .first()
        .map(|config| config.endpoint.ip());
    let (mut entry_wireguard_config, entry_gateway_ip) = wireguard_config::init_wireguard_config(
        &gateway_directory_client,
        &mut wg_entry_gateway_client,
        wg_gateway,
        entry_mtu,
    )
    .await?;

    if wg_entry_gateway_client.suspended().await? || wg_exit_gateway_client.suspended().await? {
        return Err(SetupWgTunnelError::NotEnoughBandwidthToSetupTunnel);
    }
    tokio::spawn(
        wg_entry_gateway_client.run(task_manager.subscribe_named("bandwidth_entry_client")),
    );
    tokio::spawn(wg_exit_gateway_client.run(task_manager.subscribe_named("bandwidth_exit_client")));
    entry_wireguard_config
        .talpid_config
        .peers
        .iter_mut()
        .for_each(|peer| {
            peer.allowed_ips.append(
                &mut exit_wireguard_config
                    .talpid_config
                    .peers
                    .iter()
                    .map(|peer| IpNetwork::from(peer.endpoint.ip()))
                    .collect::<Vec<_>>(),
            );
        });
    // If routing is disabled, we don't append the catch all routing rules
    if !nym_vpn.generic_config.disable_routing {
        exit_wireguard_config
            .talpid_config
            .peers
            .iter_mut()
            .for_each(|peer| {
                peer.allowed_ips
                    .append(&mut replace_default_prefixes(catch_all_ipv4()));
                peer.allowed_ips
                    .append(&mut replace_default_prefixes(catch_all_ipv6()));
            });
    } else {
        info!("Routing is disabled, skipping adding routes");
    }
    info!("Entry wireguard config: \n{entry_wireguard_config}");
    info!("Exit wireguard config: \n{exit_wireguard_config}");

    let default_node = if let Some(addr) = default_lan_gateway_ip.0.gateway.and_then(|g| {
        g.ipv4
            .first()
            .map(|a| IpAddr::from(*a))
            .or(g.ipv6.first().map(|a| IpAddr::from(*a)))
    }) {
        Node::new(addr, default_lan_gateway_ip.0.name)
    } else {
        Node::device(default_lan_gateway_ip.0.name)
    };
    let _routes = replace_default_prefixes(entry_gateway_ip.into())
        .into_iter()
        .map(move |ip| RequiredRoute::new(ip, default_node.clone()));
    #[cfg(target_os = "linux")]
    {
        let _routes = _routes.map(|route| route.use_main_table(false));
        route_manager.add_routes(_routes.collect()).await?;
    }

    std::env::set_var("TALPID_FORCE_USERSPACE_WIREGUARD", "1");
    let (wireguard_waiting_entry, event_rx) = create_wireguard_tunnel(
        route_manager,
        task_manager.subscribe_named("entry_wg_tunnel"),
        nym_vpn.tun_provider.clone(),
        entry_wireguard_config.clone(),
    )?;

    // Wait for entry gateway routes to be finished before moving to exit gateway routes, as the two might race if
    // started one after the other
    debug!("Waiting for first interface up");
    let metadata = wait_interface_up(event_rx).await.map_err(|source| {
        SetupWgTunnelError::FailedToBringInterfaceUp {
            gateway_id: Box::new(entry_wireguard_config.gateway_id),
            public_key: entry_wireguard_config.gateway_data.public_key.to_base64(),
            source,
        }
    })?;

    info!(
        "Created entry tun device {device_name} with ip={device_ip:?}",
        device_name = metadata.interface,
        device_ip = metadata.ips
    );

    let (wireguard_waiting_exit, event_rx) = create_wireguard_tunnel(
        route_manager,
        task_manager.subscribe_named("exit_wg_tunnel"),
        nym_vpn.tun_provider.clone(),
        exit_wireguard_config.clone(),
    )?;

    debug!("Waiting for second interface up");
    let metadata = wait_interface_up(event_rx).await.map_err(|source| {
        SetupWgTunnelError::FailedToBringInterfaceUp {
            gateway_id: Box::new(exit_wireguard_config.gateway_id),
            public_key: exit_wireguard_config.gateway_data.public_key.to_base64(),
            source,
        }
    })?;

    info!(
        "Created exit tun device {device_name} with ip={device_ip:?}",
        device_name = metadata.interface,
        device_ip = metadata.ips
    );
    let entry = TunnelSetup {
        specific_setup: wireguard_waiting_entry,
    };
    let exit = TunnelSetup {
        specific_setup: wireguard_waiting_exit,
    };

    Ok(AllTunnelsSetup::Wg { entry, exit })
}

#[allow(clippy::too_many_arguments)]
async fn setup_mix_tunnel(
    nym_vpn: &mut NymVpn<MixnetVpn>,
    mixnet_client: mixnet::SharedMixnetClient,
    task_manager: &mut TaskManager,
    route_manager: &mut RouteManager,
    dns_monitor: &mut DnsMonitor,
    gateway_directory_client: GatewayClient,
    exit_mix_addresses: &IpPacketRouterAddress,
    default_lan_gateway_ip: routing::LanGatewayIp,
) -> std::result::Result<AllTunnelsSetup, SetupMixTunnelError> {
    info!("Wireguard is disabled");

    let connection_info = nym_vpn
        .setup_tunnel_services(
            mixnet_client,
            route_manager,
            exit_mix_addresses,
            task_manager,
            &gateway_directory_client,
            default_lan_gateway_ip,
            dns_monitor,
        )
        .await?;

    Ok(AllTunnelsSetup::Mix(TunnelSetup {
        specific_setup: MixTunnelSetup {
            mixnet_connection_info: connection_info.0,
            exit_connection_info: connection_info.1,
        },
    }))
}

pub(crate) async fn setup_tunnel(
    nym_vpn: &mut SpecificVpn,
    task_manager: &mut TaskManager,
    route_manager: &mut RouteManager,
    dns_monitor: &mut DnsMonitor,
) -> Result<AllTunnelsSetup> {
    // The user agent is set on HTTP REST API calls, and ideally should identify the type of
    // client. This means it needs to be set way higher in the call stack, but set a default for
    // what we know here if we don't have anything.
    let user_agent = nym_vpn.user_agent().unwrap_or_else(|| {
        warn!("No user agent provided, using default");
        nym_bin_common::bin_info_local_vergen!().into()
    });
    info!("User agent: {user_agent}");

    // Create a gateway client that we use to interact with the entry gateway, in particular to
    // handle wireguard registration
    let gateway_directory_client = GatewayClient::new(nym_vpn.gateway_config(), user_agent.clone())
        .map_err(
            |err| GatewayDirectoryError::FailedtoSetupGatewayDirectoryClient {
                config: Box::new(nym_vpn.gateway_config()),
                source: err,
            },
        )?;

    let SelectedGateways { entry, exit } =
        select_gateways(&gateway_directory_client, nym_vpn).await?;

    // Get the IP address of the local LAN gateway
    let default_lan_gateway_ip = routing::LanGatewayIp::get_default_interface()?;
    debug!("default_lan_gateway_ip: {default_lan_gateway_ip}");

    platform::uniffi_set_listener_status(StatusEvent::Tun(TunStatus::EstablishingConnection));

    info!("Setting up mixnet client");
    info!("Connecting to mixnet gateway: {}", entry.identity());
    let mixnet_client = timeout(
        Duration::from_secs(MIXNET_CLIENT_STARTUP_TIMEOUT_SECS),
        mixnet::setup_mixnet_client(
            entry.identity(),
            &nym_vpn.data_path(),
            task_manager.subscribe_named("mixnet_client_main"),
            nym_vpn.mixnet_client_config(),
        ),
    )
    .await
    .map_err(|_| Error::StartMixnetClientTimeout(MIXNET_CLIENT_STARTUP_TIMEOUT_SECS))?
    .map_err(Error::FailedToSetupMixnetClient)?;

    let tunnels_setup = match nym_vpn {
        SpecificVpn::Wg(vpn) => {
            let auth_addresses = match setup_auth_addresses(&entry, &exit) {
                Ok(auth_addr) => auth_addr,
                Err(err) => {
                    // Put in some manual error handling, the correct long-term solution is that handling
                    // errors and diconnecting the mixnet client needs to be unified down this code path
                    // and merged with the mix tunnel one.
                    mixnet_client.disconnect().await;
                    return Err(err.into());
                }
            };

            // HERE BE DRAGONS: this can fail and the mixnet_client is not disconnected when that
            // happens!
            setup_wg_tunnel(
                vpn,
                mixnet_client,
                task_manager,
                route_manager,
                gateway_directory_client,
                auth_addresses,
                default_lan_gateway_ip,
            )
            .await
            .map_err(Error::from)
        }
        SpecificVpn::Mix(vpn) => setup_mix_tunnel(
            vpn,
            mixnet_client,
            task_manager,
            route_manager,
            dns_monitor,
            gateway_directory_client,
            &exit.ipr_address.unwrap(),
            default_lan_gateway_ip,
        )
        .await
        .map_err(Error::from),
    }?;
    Ok(tunnels_setup)
}

fn setup_auth_addresses(
    entry: &nym_gateway_directory::Gateway,
    exit: &nym_gateway_directory::Gateway,
) -> std::result::Result<AuthAddresses, SetupWgTunnelError> {
    let entry_authenticator_address = entry
        .authenticator_address
        .ok_or(SetupWgTunnelError::AuthenticatorAddressNotFound)?;
    let exit_authenticator_address = exit
        .authenticator_address
        .ok_or(SetupWgTunnelError::AuthenticatorAddressNotFound)?;
    Ok(AuthAddresses::new(
        entry_authenticator_address,
        exit_authenticator_address,
    ))
}

struct SelectedGateways {
    entry: nym_gateway_directory::Gateway,
    exit: nym_gateway_directory::Gateway,
}

async fn select_gateways(
    gateway_directory_client: &GatewayClient,
    nym_vpn: &SpecificVpn,
) -> std::result::Result<SelectedGateways, GatewayDirectoryError> {
    // The set of exit gateways is smaller than the set of entry gateways, so we start by selecting
    // the exit gateway and then filter out the exit gateway from the set of entry gateways.

    let (mut entry_gateways, exit_gateways) = if let SpecificVpn::Mix(_) = nym_vpn {
        // Setup the gateway that we will use as the exit point
        let exit_gateways = gateway_directory_client
            .lookup_exit_gateways()
            .await
            .map_err(|source| GatewayDirectoryError::FailedToLookupGateways { source })?;
        // Setup the gateway that we will use as the entry point
        let entry_gateways = gateway_directory_client
            .lookup_entry_gateways()
            .await
            .map_err(|source| GatewayDirectoryError::FailedToLookupGateways { source })?;
        (entry_gateways, exit_gateways)
    } else {
        let all_gateways = gateway_directory_client
            .lookup_all_gateways()
            .await
            .map_err(|source| GatewayDirectoryError::FailedToLookupGateways { source })?;
        (all_gateways.clone(), all_gateways)
    };

    let exit_gateway = nym_vpn
        .exit_point()
        .lookup_gateway(&exit_gateways)
        .map_err(|source| GatewayDirectoryError::FailedToSelectExitGateway { source })?;

    // Exclude the exit gateway from the list of entry gateways for privacy reasons
    entry_gateways.remove_gateway(&exit_gateway);

    let entry_gateway = nym_vpn
        .entry_point()
        .lookup_gateway(&entry_gateways)
        .await
        .map_err(|source| match source {
            nym_gateway_directory::Error::NoMatchingEntryGatewayForLocation {
                requested_location,
                available_countries: _,
            } if Some(requested_location.as_str())
                == exit_gateway.two_letter_iso_country_code() =>
            {
                GatewayDirectoryError::SameEntryAndExitGatewayFromCountry {
                    requested_location: requested_location.to_string(),
                }
            }
            _ => GatewayDirectoryError::FailedToSelectEntryGateway { source },
        })?;

    info!("Found {} entry gateways", entry_gateways.len());
    info!("Found {} exit gateways", exit_gateways.len());
    info!(
        "Using entry gateway: {}, location: {}, performance: {}",
        *entry_gateway.identity(),
        entry_gateway
            .two_letter_iso_country_code()
            .map_or_else(|| "unknown".to_string(), |code| code.to_string()),
        entry_gateway
            .performance
            .map_or_else(|| "unknown".to_string(), |perf| perf.to_string()),
    );
    info!(
        "Using exit gateway: {}, location: {}, performance: {}",
        *exit_gateway.identity(),
        exit_gateway
            .two_letter_iso_country_code()
            .map_or_else(|| "unknown".to_string(), |code| code.to_string()),
        entry_gateway
            .performance
            .map_or_else(|| "unknown".to_string(), |perf| perf.to_string()),
    );
    info!(
        "Using exit router address {}",
        exit_gateway
            .ipr_address
            .map_or_else(|| "none".to_string(), |ipr| ipr.to_string())
    );

    Ok(SelectedGateways {
        entry: entry_gateway,
        exit: exit_gateway,
    })
}
