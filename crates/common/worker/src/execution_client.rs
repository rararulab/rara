use std::collections::BTreeMap;

use rara_api::pb::execution::v1::{
    self as pb, execution_worker_service_client::ExecutionWorkerServiceClient,
};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value as JsonValue;
use snafu::prelude::*;
use tonic::transport::{Channel, Endpoint};

#[derive(Debug, Clone)]
pub struct ExecutionWorkerClient {
    endpoint: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityInfo {
    pub name:           String,
    pub supports_sync:  bool,
    pub supports_async: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RemoteWorkerError {
    pub code:      String,
    pub message:   String,
    pub retryable: bool,
    pub details:   Option<JsonValue>,
}

#[derive(Debug, Snafu)]
pub enum ExecutionWorkerClientError {
    #[snafu(display("execution worker transport error: {source}"))]
    Transport { source: tonic::transport::Error },

    #[snafu(display("execution worker rpc failed: {source}"))]
    Rpc { source: tonic::Status },

    #[snafu(display("execution worker returned error {code}: {message}"))]
    Worker {
        code:      String,
        message:   String,
        retryable: bool,
        details:   Option<JsonValue>,
    },

    #[snafu(display("execution worker codec error: {message}"))]
    Codec { message: String },

    #[snafu(display("execution worker protocol error: {message}"))]
    Protocol { message: String },
}

type Result<T, E = ExecutionWorkerClientError> = std::result::Result<T, E>;

impl ExecutionWorkerClient {
    #[must_use]
    pub fn new(endpoint: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
        }
    }

    pub async fn list_capabilities(&self) -> Result<Vec<CapabilityInfo>> {
        let mut client = self.grpc_client()?;
        let resp = client
            .list_capabilities(pb::ListCapabilitiesRequest {})
            .await
            .context(RpcSnafu)?
            .into_inner();

        match resp.outcome {
            Some(pb::list_capabilities_response::Outcome::Success(success)) => Ok(success
                .capabilities
                .into_iter()
                .map(|c| CapabilityInfo {
                    name:           c.name,
                    supports_sync:  c.supports_sync,
                    supports_async: c.supports_async,
                })
                .collect()),
            Some(pb::list_capabilities_response::Outcome::Error(err)) => WorkerSnafu {
                code:      err.code,
                message:   err.message,
                retryable: err.retryable,
                details:   decode_error_details(&err.details)?,
            }
            .fail(),
            None => ProtocolSnafu {
                message: "ListCapabilitiesResponse.outcome is missing".to_owned(),
            }
            .fail(),
        }
    }

    pub async fn invoke_json<TReq, TResp>(&self, capability: &str, payload: &TReq) -> Result<TResp>
    where
        TReq: Serialize,
        TResp: DeserializeOwned,
    {
        self.invoke_json_with_options(capability, payload, 0, BTreeMap::new())
            .await
    }

    pub async fn invoke_json_with_options<TReq, TResp>(
        &self,
        capability: &str,
        payload: &TReq,
        timeout_ms: u32,
        metadata: BTreeMap<String, String>,
    ) -> Result<TResp>
    where
        TReq: Serialize,
        TResp: DeserializeOwned,
    {
        let payload_bytes =
            serde_json::to_vec(payload).map_err(|e| ExecutionWorkerClientError::Codec {
                message: format!("serialize request payload failed: {e}"),
            })?;

        let mut client = self.grpc_client()?;
        let resp = client
            .invoke(pb::InvokeRequest {
                capability: capability.to_owned(),
                payload: payload_bytes,
                timeout_ms,
                metadata: metadata.into_iter().collect(),
            })
            .await
            .context(RpcSnafu)?
            .into_inner();

        let result_json = match resp.outcome {
            Some(pb::invoke_response::Outcome::Success(success)) => {
                serde_json::from_slice(&success.result).map_err(|e| {
                    ExecutionWorkerClientError::Codec {
                        message: format!("decode invoke result bytes for {capability} failed: {e}"),
                    }
                })?
            }
            Some(pb::invoke_response::Outcome::Error(err)) => {
                return WorkerSnafu {
                    code:      err.code,
                    message:   err.message,
                    retryable: err.retryable,
                    details:   decode_error_details(&err.details)?,
                }
                .fail();
            }
            None => {
                return ProtocolSnafu {
                    message: format!(
                        "InvokeResponse.outcome is missing for capability {capability}"
                    ),
                }
                .fail();
            }
        };

        serde_json::from_value(result_json).map_err(|e| ExecutionWorkerClientError::Codec {
            message: format!("deserialize invoke result for {capability} failed: {e}"),
        })
    }

    fn grpc_client(&self) -> Result<ExecutionWorkerServiceClient<Channel>> {
        let endpoint = Endpoint::from_shared(self.endpoint.clone()).context(TransportSnafu)?;
        Ok(ExecutionWorkerServiceClient::new(endpoint.connect_lazy()))
    }
}

fn decode_error_details(bytes: &[u8]) -> Result<Option<JsonValue>> {
    if bytes.is_empty() {
        return Ok(None);
    }
    let value = serde_json::from_slice(bytes).map_err(|e| ExecutionWorkerClientError::Codec {
        message: format!("decode worker error details failed: {e}"),
    })?;
    Ok(Some(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_round_trips_through_bytes_codec_without_numeric_coercion() {
        let value = serde_json::json!({
            "int": 42,
            "float": 3.5,
            "bool": true,
            "null": null,
            "nested": {"x": 1},
            "list": [1, "a", false]
        });

        let bytes = serde_json::to_vec(&value).expect("serialize");
        let got: JsonValue = serde_json::from_slice(&bytes).expect("deserialize");

        assert_eq!(got, value);
    }
}
