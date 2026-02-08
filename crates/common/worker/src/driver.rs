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

use std::time::Duration;

use crate::context::WorkerContext;

/// Internal enum for trigger execution strategy.
pub(crate) enum TriggerDriverEnum {
    Once(OnceDriver),
    Notify(NotifyDriver),
    Interval(IntervalDriver),
    Cron(CronDriver),
    IntervalOrNotify(IntervalOrNotifyDriver),
    CronOrNotify(CronOrNotifyDriver),
}

impl TriggerDriverEnum {
    pub async fn wait_next<S: Clone>(&mut self, ctx: &WorkerContext<S>) -> bool {
        match self {
            Self::Once(d) => d.wait_next(ctx).await,
            Self::Notify(d) => d.wait_next(ctx).await,
            Self::Interval(d) => d.wait_next(ctx).await,
            Self::Cron(d) => d.wait_next(ctx).await,
            Self::IntervalOrNotify(d) => d.wait_next(ctx).await,
            Self::CronOrNotify(d) => d.wait_next(ctx).await,
        }
    }
}

/// Internal trait for trigger execution strategy.
trait TriggerDriver: Send {
    /// Wait for next execution. Returns false if should stop.
    async fn wait_next<S: Clone>(&mut self, ctx: &WorkerContext<S>) -> bool;
}

/// Driver for Once trigger.
pub(crate) struct OnceDriver {
    executed: bool,
}

impl OnceDriver {
    pub const fn new() -> Self { Self { executed: false } }
}

impl TriggerDriver for OnceDriver {
    async fn wait_next<S: Clone>(&mut self, ctx: &WorkerContext<S>) -> bool {
        if self.executed {
            // Wait for cancellation
            ctx.cancelled().await;
            false
        } else {
            self.executed = true;
            true
        }
    }
}

/// Driver for Notify trigger.
pub(crate) struct NotifyDriver;

impl NotifyDriver {
    pub const fn new() -> Self { Self }
}

impl TriggerDriver for NotifyDriver {
    async fn wait_next<S: Clone>(&mut self, ctx: &WorkerContext<S>) -> bool {
        tokio::select! {
            () = ctx.notified() => true,
            () = ctx.cancelled() => false,
        }
    }
}

/// Driver for Interval trigger.
pub(crate) struct IntervalDriver {
    interval: tokio::time::Interval,
}

impl IntervalDriver {
    pub fn new(duration: Duration) -> Self {
        let mut interval = tokio::time::interval(duration);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        Self { interval }
    }
}

impl TriggerDriver for IntervalDriver {
    async fn wait_next<S: Clone>(&mut self, ctx: &WorkerContext<S>) -> bool {
        tokio::select! {
            _ = self.interval.tick() => true,
            () = ctx.cancelled() => false,
        }
    }
}

/// Driver for Cron trigger.
pub(crate) struct CronDriver {
    cron: croner::Cron,
}

impl CronDriver {
    pub const fn new(cron: croner::Cron) -> Self { Self { cron } }
}

impl TriggerDriver for CronDriver {
    async fn wait_next<S: Clone>(&mut self, ctx: &WorkerContext<S>) -> bool {
        let Ok(next) = self.cron.find_next_occurrence(&chrono::Utc::now(), false) else {
            ctx.cancelled().await;
            return false;
        };

        let now = chrono::Utc::now();
        if next > now {
            let duration = (next - now).to_std().unwrap_or(Duration::from_secs(0));

            tokio::select! {
                () = tokio::time::sleep(duration) => true,
                () = ctx.cancelled() => false,
            }
        } else {
            true
        }
    }
}

/// Driver for `IntervalOrNotify` trigger.
pub(crate) struct IntervalOrNotifyDriver {
    interval: tokio::time::Interval,
}

impl IntervalOrNotifyDriver {
    pub fn new(duration: Duration) -> Self {
        let mut interval = tokio::time::interval(duration);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        Self { interval }
    }
}

impl TriggerDriver for IntervalOrNotifyDriver {
    async fn wait_next<S: Clone>(&mut self, ctx: &WorkerContext<S>) -> bool {
        tokio::select! {
            _ = self.interval.tick() => true,
            () = ctx.notified() => {
                // Reset the interval
                self.interval.reset();
                true
            },
            () = ctx.cancelled() => false,
        }
    }
}

/// Driver for `CronOrNotify` trigger.
pub(crate) struct CronOrNotifyDriver {
    cron: croner::Cron,
}

impl CronOrNotifyDriver {
    pub const fn new(cron: croner::Cron) -> Self { Self { cron } }
}

impl TriggerDriver for CronOrNotifyDriver {
    async fn wait_next<S: Clone>(&mut self, ctx: &WorkerContext<S>) -> bool {
        let Ok(next) = self.cron.find_next_occurrence(&chrono::Utc::now(), false) else {
            return tokio::select! {
                () = ctx.notified() => true,
                () = ctx.cancelled() => false,
            };
        };

        let now = chrono::Utc::now();
        if next > now {
            let duration = (next - now).to_std().unwrap_or(Duration::from_secs(0));

            tokio::select! {
                () = tokio::time::sleep(duration) => true,
                () = ctx.notified() => true,
                () = ctx.cancelled() => false,
            }
        } else {
            tokio::select! {
                () = ctx.notified() => true,
                () = ctx.cancelled() => false,
                else => true,
            }
        }
    }
}
