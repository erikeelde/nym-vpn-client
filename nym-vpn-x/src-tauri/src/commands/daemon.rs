use crate::error::BackendError;
use crate::grpc::client::{GrpcClient, VpndStatus};
use crate::states::SharedAppState;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::State;
use tracing::{debug, instrument, warn};
use ts_rs::TS;

#[derive(Serialize, Deserialize, Debug, Clone, TS)]
#[ts(export)]
pub struct DaemonInfo {
    version: String,
    network: String,
}

#[instrument(skip_all)]
#[tauri::command]
pub async fn daemon_status(
    app_state: State<'_, SharedAppState>,
    grpc_client: State<'_, Arc<GrpcClient>>,
) -> Result<VpndStatus, BackendError> {
    debug!("daemon_status");
    let status = grpc_client
        .check(app_state.inner())
        .await
        .inspect_err(|e| {
            warn!("failed to check daemon status: {:?}", e);
        })
        .unwrap_or(VpndStatus::NotOk);
    debug!("daemon status: {:?}", status);
    Ok(status)
}

#[instrument(skip_all)]
#[tauri::command]
pub async fn daemon_info(
    grpc_client: State<'_, Arc<GrpcClient>>,
) -> Result<DaemonInfo, BackendError> {
    debug!("daemon_info");
    let res = grpc_client.vpnd_info().await.inspect_err(|e| {
        warn!("failed to get daemon info: {:?}", e);
    })?;

    Ok(DaemonInfo {
        version: res.version,
        network: res.network_name,
    })
}
