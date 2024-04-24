use std::cmp::Ordering;
use std::sync::Arc;
use std::time::Duration;

use nym_ip_packet_requests::IpPair;
use nym_ip_packet_requests::{
    request::IpPacketRequest,
    response::{
        DynamicConnectResponse, IpPacketResponse, IpPacketResponseData, StaticConnectResponse,
    },
};
use nym_sdk::mixnet::{MixnetClient, MixnetMessageSender, Recipient};
use tracing::{debug, error};

use nym_gateway_directory::IpPacketRouterAddress;

use crate::{Error, Result};

#[derive(Clone)]
pub struct SharedMixnetClient(Arc<tokio::sync::Mutex<Option<MixnetClient>>>);

impl SharedMixnetClient {
    pub fn new(mixnet_client: MixnetClient) -> Self {
        Self(Arc::new(tokio::sync::Mutex::new(Some(mixnet_client))))
    }

    pub async fn lock(&self) -> tokio::sync::MutexGuard<'_, Option<MixnetClient>> {
        self.0.lock().await
    }

    pub async fn nym_address(&self) -> Recipient {
        *self.lock().await.as_ref().unwrap().nym_address()
    }

    // pub async fn split_sender(&self) -> MixnetClientSender {
    //     self.lock().await.as_ref().unwrap().split_sender()
    // }

    // pub async fn gateway_ws_fd(&self) -> Option<RawFd> {
    //     self.lock()
    //         .await
    //         .as_ref()
    //         .unwrap()
    //         .gateway_connection()
    //         .gateway_ws_fd
    // }

    pub async fn send(&self, msg: nym_sdk::mixnet::InputMessage) -> Result<()> {
        self.lock().await.as_mut().unwrap().send(msg).await?;
        Ok(())
    }

    // pub async fn disconnect(self) -> Self {
    //     let handle = self.lock().await.take().unwrap();
    //     handle.disconnect().await;
    //     self
    // }

    pub fn inner(&self) -> Arc<tokio::sync::Mutex<Option<MixnetClient>>> {
        self.0.clone()
    }
}

async fn send_connect_to_ip_packet_router(
    mixnet_client: &SharedMixnetClient,
    ip_packet_router_address: &IpPacketRouterAddress,
    ips: Option<IpPair>,
    enable_two_hop: bool,
) -> Result<u64> {
    let hops = enable_two_hop.then_some(0);
    let mixnet_client_address = mixnet_client.nym_address().await;
    let (request, request_id) = if let Some(ips) = ips {
        debug!("Sending static connect request with ips: {ips}");
        IpPacketRequest::new_static_connect_request(ips, mixnet_client_address, hops, None, None)
    } else {
        debug!("Sending dynamic connect request");
        IpPacketRequest::new_dynamic_connect_request(mixnet_client_address, hops, None, None)
    };
    debug!("Sent connect request with version v{}", request.version);

    mixnet_client
        .send(nym_sdk::mixnet::InputMessage::new_regular_with_custom_hops(
            ip_packet_router_address.0,
            request.to_bytes().unwrap(),
            nym_task::connections::TransmissionLane::General,
            None,
            hops,
        ))
        .await?;

    Ok(request_id)
}

async fn wait_for_connect_response(
    mixnet_client: &SharedMixnetClient,
    request_id: u64,
) -> Result<IpPacketResponse> {
    let timeout = tokio::time::sleep(Duration::from_secs(5));
    tokio::pin!(timeout);

    // Connecting is basically synchronous from the perspective of the mixnet client, so it's safe
    // to just grab ahold of the mutex and keep it until we get the response.
    let mut mixnet_client_handle = mixnet_client.lock().await;
    let mixnet_client = mixnet_client_handle.as_mut().unwrap();

    loop {
        tokio::select! {
            _ = &mut timeout => {
                error!("Timed out waiting for reply to connect request");
                return Err(Error::TimeoutWaitingForConnectResponse);
            }
            msgs = mixnet_client.wait_for_messages() => {
                if let Some(msgs) = msgs {
                    for msg in msgs {

                        // Handle if the response is from an IPR running an older or newer version
                        if let Some(version) = msg.message.first() {
                            match version.cmp(&nym_ip_packet_requests::CURRENT_VERSION) {
                                Ordering::Greater => {
                                    log::error!("Received packet with newer version: v{version}, is your client up to date?");
                                    return Err(Error::ReceivedResponseWithNewVersion {
                                        expected: nym_ip_packet_requests::CURRENT_VERSION,
                                        received: *version,
                                    });
                                }
                                Ordering::Less => {
                                    log::error!("Received packet with older version: v{version}, you client appears to be too new for the exit gateway or exit ip-packet-router?");
                                    return Err(Error::ReceivedResponseWithOldVersion {
                                        expected: nym_ip_packet_requests::CURRENT_VERSION,
                                        received: *version,
                                    });
                                }
                                Ordering::Equal => {
                                    // We're good
                                }
                            }
                        }

                        debug!("MixnetProcessor: Got message while waiting for connect response");
                        let Ok(response) = IpPacketResponse::from_reconstructed_message(&msg) else {
                            // This is ok, it's likely just one of our self-pings
                            debug!("Failed to deserialize reconstructed message");
                            continue;
                        };
                        if response.id() == Some(request_id) {
                            debug!("Got response with matching id");
                            return Ok(response);
                        }
                    }
                } else {
                    return Err(Error::NoMixnetMessagesReceived);
                }
            }
        }
    }
}

async fn handle_static_connect_response(
    mixnet_client_address: &Recipient,
    response: StaticConnectResponse,
) -> Result<()> {
    debug!("Handling static connect response");
    if response.reply_to != *mixnet_client_address {
        error!("Got reply intended for wrong address");
        return Err(Error::GotReplyIntendedForWrongAddress);
    }
    match response.reply {
        nym_ip_packet_requests::response::StaticConnectResponseReply::Success => Ok(()),
        nym_ip_packet_requests::response::StaticConnectResponseReply::Failure(reason) => {
            Err(Error::StaticConnectRequestDenied { reason })
        }
    }
}

async fn handle_dynamic_connect_response(
    mixnet_client_address: &Recipient,
    response: DynamicConnectResponse,
) -> Result<IpPair> {
    debug!("Handling dynamic connect response");
    if response.reply_to != *mixnet_client_address {
        error!("Got reply intended for wrong address");
        return Err(Error::GotReplyIntendedForWrongAddress);
    }
    match response.reply {
        nym_ip_packet_requests::response::DynamicConnectResponseReply::Success(r) => Ok(r.ips),
        nym_ip_packet_requests::response::DynamicConnectResponseReply::Failure(reason) => {
            Err(Error::DynamicConnectRequestDenied { reason })
        }
    }
}

pub async fn connect_to_ip_packet_router(
    mixnet_client: SharedMixnetClient,
    ip_packet_router_address: &IpPacketRouterAddress,
    ips: Option<IpPair>,
    enable_two_hop: bool,
) -> Result<IpPair> {
    debug!("Sending connect request");
    let request_id = send_connect_to_ip_packet_router(
        &mixnet_client,
        ip_packet_router_address,
        ips,
        enable_two_hop,
    )
    .await?;

    debug!("Waiting for reply...");
    let response = wait_for_connect_response(&mixnet_client, request_id).await?;

    let mixnet_client_address = mixnet_client.nym_address().await;
    match response.data {
        IpPacketResponseData::StaticConnect(resp) if ips.is_some() => {
            handle_static_connect_response(&mixnet_client_address, resp).await?;
            Ok(ips.unwrap())
        }
        IpPacketResponseData::DynamicConnect(resp) if ips.is_none() => {
            handle_dynamic_connect_response(&mixnet_client_address, resp).await
        }
        response => {
            error!("Unexpected response: {:?}", response);
            Err(Error::UnexpectedConnectResponse)
        }
    }
}