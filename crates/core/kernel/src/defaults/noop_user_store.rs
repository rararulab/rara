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

//! NoopUserStore — test-only user store that always returns a fully
//! permissioned user, ensuring existing tests don't break.

use async_trait::async_trait;

use crate::error::Result;
use crate::process::principal::Role;
use crate::process::user::{KernelUser, Permission, PlatformIdentity, UserStore};

/// A no-op user store for tests.
///
/// `get_by_name()` always returns a fully-permissioned admin user so that
/// `validate_principal()` passes without requiring a real database.
pub struct NoopUserStore;

#[async_trait]
impl UserStore for NoopUserStore {
    async fn get_by_id(&self, _id: uuid::Uuid) -> Result<Option<KernelUser>> {
        Ok(None)
    }

    async fn get_by_name(&self, name: &str) -> Result<Option<KernelUser>> {
        Ok(Some(KernelUser {
            id: uuid::Uuid::nil(),
            name: name.to_string(),
            role: Role::Admin,
            permissions: vec![Permission::All],
            enabled: true,
            created_at: jiff::Timestamp::now(),
            updated_at: jiff::Timestamp::now(),
        }))
    }

    async fn get_by_platform(
        &self,
        _platform: &str,
        _platform_user_id: &str,
    ) -> Result<Option<KernelUser>> {
        Ok(None)
    }

    async fn create(&self, _user: &KernelUser) -> Result<()> {
        Ok(())
    }

    async fn update(&self, _user: &KernelUser) -> Result<()> {
        Ok(())
    }

    async fn delete(&self, _id: uuid::Uuid) -> Result<()> {
        Ok(())
    }

    async fn list(&self) -> Result<Vec<KernelUser>> {
        Ok(vec![])
    }

    async fn link_platform(&self, _identity: &PlatformIdentity) -> Result<()> {
        Ok(())
    }

    async fn unlink_platform(&self, _id: uuid::Uuid) -> Result<()> {
        Ok(())
    }

    async fn list_platforms(&self, _user_id: uuid::Uuid) -> Result<Vec<PlatformIdentity>> {
        Ok(vec![])
    }
}
