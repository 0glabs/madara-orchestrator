use async_trait::async_trait;
use color_eyre::{eyre::eyre, Result};
use serde::{Deserialize, Serialize};
use serde_json;
use starknet::core::types::FieldElement;
use std::sync::Arc;
use tokio::sync::{Mutex, Semaphore};

use config::ZgDaConfig;
use da_client_interface::{DaClient, DaVerificationStatus};

pub mod disperser {
    tonic::include_proto!("disperser");
}

use disperser::{
    disperser_client::DisperserClient, BlobStatus, BlobStatusReply, BlobStatusRequest, DisperseBlobRequest,
};

pub mod config;
pub struct ZgDaClient {
    client: Arc<Mutex<DisperserClient<tonic::transport::Channel>>>,

    config: ZgDaConfig,
    disperser_permits: Semaphore,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct BlobKey {
    id: Vec<u8>,
    data_root: Vec<u8>,
    epoch: u32,
    quorum_id: u32,
}

#[async_trait]
impl DaClient for ZgDaClient {
    async fn publish_state_diff(&self, state_diff: Vec<FieldElement>) -> Result<String> {
        let mut data = vec![];
        for diff in state_diff {
            let s = diff.to_bytes_be();
            data.extend_from_slice(&s);
        }

        let (id, status) = self.store_blob(&data).await?;
        let blob_header = status
            .info
            .ok_or_else(|| eyre!("blob info not none"))?
            .blob_header
            .ok_or_else(|| eyre!("blob header not none"))?;
        let key = BlobKey {
            id,
            data_root: blob_header.data_root,
            epoch: blob_header.epoch,
            quorum_id: blob_header.quorum_id,
        };

        Ok(serde_json::to_string(&key)?)
    }

    async fn verify_inclusion(&self, external_id: &str) -> Result<DaVerificationStatus> {
        let key: BlobKey = serde_json::from_str(&external_id)?;

        let resp = self.get_blob_confirmation(&key.id).await?;
        match BlobStatus::try_from(resp.status).ok() {
            Some(BlobStatus::Confirmed) | Some(BlobStatus::Finalized) => Ok(DaVerificationStatus::Verified),
            Some(BlobStatus::Processing) => Ok(DaVerificationStatus::Pending),
            _ => Ok(DaVerificationStatus::Rejected),
        }
    }
}

impl ZgDaClient {
    async fn store_blob(&self, data: &[u8]) -> Result<(Vec<u8>, BlobStatusReply)> {
        let request_id = self.disperse_blob_inner(data).await?;

        // confirm blobs
        let confirmation = self.wait_for_blob_confirmation(&request_id).await?;
        return Ok((request_id, confirmation));
    }

    async fn disperse_blob_inner(&self, data: &[u8]) -> Result<Vec<u8>> {
        let _permit = self.disperser_permits.acquire().await.expect("request permit");

        let mut client = self.client.lock().await;
        let response = loop {
            let request = tonic::Request::new(self.disperse_blob_request(&data));
            match client.disperse_blob(request).await {
                Ok(resp) => {
                    break resp;
                }
                Err(_resp) => {
                    tokio::time::sleep(tokio::time::Duration::from_millis(self.config.disperser_retry_delay_ms.into()))
                        .await;
                }
            }
        };
        Ok(response.into_inner().request_id.clone())
    }

    fn disperse_blob_request(&self, data: &[u8]) -> DisperseBlobRequest {
        disperser::DisperseBlobRequest { data: data.to_vec(), security_params: vec![], target_row_num: 0 }
    }

    async fn wait_for_blob_confirmation(&self, request_id: &Vec<u8>) -> Result<BlobStatusReply> {
        let mut client = self.client.lock().await;
        let response = loop {
            let response = client.get_blob_status(BlobStatusRequest { request_id: request_id.clone() }).await;
            let reply = response.unwrap().into_inner();
            let blob_status = BlobStatus::try_from(reply.status).ok();
            if let Some(BlobStatus::Confirmed) = blob_status {
                break reply;
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(self.config.status_retry_delay_ms.into())).await
        };

        Ok(response)
    }

    async fn get_blob_confirmation(&self, request_id: &Vec<u8>) -> Result<BlobStatusReply> {
        let mut client = self.client.lock().await;
        let response = client.get_blob_status(BlobStatusRequest { request_id: request_id.clone() }).await;
        Ok(response.unwrap().into_inner())
    }
}

impl From<ZgDaConfig> for ZgDaClient {
    fn from(config: ZgDaConfig) -> Self {
        let client = Arc::new(Mutex::new(
            futures::executor::block_on(async { DisperserClient::connect(config.url.clone()).await })
                .expect("Failed to create da client"),
        ));
        ZgDaClient { client, config, disperser_permits: Semaphore::new(1 as usize) }
    }
}
