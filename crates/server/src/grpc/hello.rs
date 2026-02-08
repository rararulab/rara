// Copyright 2025 Crrow
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//      http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use std::sync::Arc;

use async_trait::async_trait;
use rsketch_api::pb::hello::v1::{HelloRequest, HelloResponse, hello_service_server};
use tokio_util::sync::CancellationToken;
use tonic::service::RoutesBuilder;
use tonic_health::server::HealthReporter;

use crate::{
    error::{ApiError, ApiResult},
    grpc::GrpcServiceHandler,
};

#[derive(Default)]
pub struct HelloService;

#[async_trait]
impl hello_service_server::HelloService for HelloService {
    async fn hello(
        &self,
        request: tonic::Request<HelloRequest>,
    ) -> std::result::Result<tonic::Response<HelloResponse>, tonic::Status> {
        let response = Self::hello_inner(request).map_err(tonic::Status::from)?;
        Ok(response)
    }
}

impl HelloService {
    fn hello_inner(
        request: tonic::Request<HelloRequest>,
    ) -> ApiResult<tonic::Response<HelloResponse>> {
        let name = request.into_inner().name;
        if name.trim().is_empty() {
            return Err(ApiError::InvalidArgument {
                reason: "name must not be empty".to_string(),
            });
        }
        let message = format!("Hello, {name}!");
        Ok(tonic::Response::new(HelloResponse { message }))
    }
}

#[async_trait]
impl GrpcServiceHandler for HelloService {
    fn service_name(&self) -> &'static str { "HelloService" }

    fn file_descriptor_set(&self) -> &'static [u8] { rsketch_api::pb::GRPC_DESC }

    fn register_service(self: &Arc<Self>, builder: &mut RoutesBuilder) {
        builder.add_service(hello_service_server::HelloServiceServer::from_arc(
            self.clone(),
        ));
    }

    async fn readiness_reporting(
        self: &Arc<Self>,
        _cancellation_token: CancellationToken,
        reporter: HealthReporter,
    ) {
        reporter
            .set_serving::<hello_service_server::HelloServiceServer<Self>>()
            .await;
    }
}
