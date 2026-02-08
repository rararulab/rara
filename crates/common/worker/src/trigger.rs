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

// ============================================================================
// Pause Mode
// ============================================================================

/// Defines how a worker behaves when paused.
///
/// This affects what happens to the trigger driver when `pause()` is called:
///
/// - **Soft pause** (default): The driver continues running, but `work()` calls
///   are skipped. The timer/schedule keeps advancing.
/// - **Hard pause**: The driver stops completely and waits for `resume()`. The
///   timer/schedule pauses.
///
/// # Use Cases
///
/// - **Soft pause**: Good for temporary throttling where you don't want to miss
///   scheduled times entirely.
/// - **Hard pause**: Good when you need to completely suspend the worker and
///   resume exactly where you left off.
///
/// # Example
///
/// ```rust,no_run
/// use std::time::Duration;
///
/// use rsketch_common_worker::{Manager, Pausable, PauseMode, Worker, WorkerContext};
///
/// # struct MyWorker;
/// # #[async_trait::async_trait]
/// # impl Worker for MyWorker {
/// #     async fn work<S: Clone + Send + Sync>(&mut self, _ctx: WorkerContext<S>) {}
/// # }
/// # #[tokio::main]
/// # async fn main() {
/// let mut manager = Manager::new();
///
/// // Hard pause: timer stops completely when paused
/// let handle = manager
///     .worker(MyWorker)
///     .interval(Duration::from_secs(10))
///     .pause_mode(PauseMode::Hard)
///     .spawn();
///
/// handle.pause(); // Timer stops
/// // ... some time passes ...
/// handle.resume(); // Timer resumes from where it was
/// //
/// # }
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PauseMode {
    /// Soft pause: driver continues, work is skipped.
    ///
    /// - Interval: timer keeps running, missed ticks are skipped
    /// - Cron: scheduled times are skipped
    ///
    /// This is the default behavior.
    #[default]
    Soft,

    /// Hard pause: driver stops completely.
    ///
    /// - Interval: timer pauses, resumes from where it left off
    /// - Cron: scheduling pauses, resumes at next occurrence after resume
    Hard,
}

// ============================================================================
// Trigger
// ============================================================================

/// Defines when and how a worker should be executed.
///
/// Triggers control the scheduling strategy for worker execution. Each trigger
/// type returns a different handle type with appropriate control capabilities.
///
/// # Trigger Types
///
/// - **`Once`**: Runs immediately once at startup, then stops
/// - **`Notify`**: Runs only when explicitly triggered via handle
/// - **`Interval`**: Runs periodically at fixed intervals
/// - **`Cron`**: Runs on a cron schedule (e.g., "0 0 * * *" for daily at
///   midnight)
/// - **`IntervalOrNotify`**: Hybrid - runs on interval OR when notified (timer
///   resets)
/// - **`CronOrNotify`**: Hybrid - runs on cron schedule OR when notified
///
/// # Handle Mapping
///
/// Each trigger type yields a specific handle:
///
/// | Trigger | Handle | Capabilities |
/// |---------|--------|--------------|
/// | `Once` | `OnceHandle` | None (runs once) |
/// | `Notify` | `NotifyHandle` | Notifiable |
/// | `Interval` | `IntervalHandle` | Pausable |
/// | `Cron` | `CronHandle` | Pausable |
/// | `IntervalOrNotify` | `IntervalOrNotifyHandle` | Pausable + Notifiable |
/// | `CronOrNotify` | `CronOrNotifyHandle` | Pausable + Notifiable |
///
/// # Examples
///
/// ```rust
/// use std::{str::FromStr, time::Duration};
///
/// use rsketch_common_worker::Trigger;
///
/// // Run every 5 seconds
/// let trigger = Trigger::Interval(Duration::from_secs(5));
///
/// // Run every day at midnight (standard cron format)
/// let cron = croner::Cron::from_str("0 0 * * *").unwrap();
/// let trigger = Trigger::Cron(cron);
///
/// // Run every hour OR when manually notified
/// let trigger = Trigger::IntervalOrNotify(Duration::from_secs(3600));
/// ```
#[derive(Debug, Clone)]
pub enum Trigger {
    /// Execute once immediately on startup, then stop.
    ///
    /// Returns [`OnceHandle`](crate::OnceHandle) with no control methods.
    /// Useful for initialization tasks.
    Once,

    /// Execute only when explicitly notified via handle.
    ///
    /// Returns [`NotifyHandle`](crate::NotifyHandle) with `notify()` method.
    /// Useful for event-driven or on-demand tasks.
    Notify,

    /// Execute at fixed intervals.
    ///
    /// Returns [`IntervalHandle`](crate::IntervalHandle) with pause/resume
    /// methods. The interval starts immediately and repeats continuously.
    Interval(Duration),

    /// Execute on a cron schedule.
    ///
    /// Returns [`CronHandle`](crate::CronHandle) with pause/resume methods.
    /// Uses standard 5-field cron format: `minute hour day month weekday`.
    /// Example: `"0 0 * * *"` = daily at midnight.
    Cron(croner::Cron),

    /// Execute at intervals OR when manually notified.
    ///
    /// Returns [`IntervalOrNotifyHandle`](crate::IntervalOrNotifyHandle) with
    /// pause/resume and notify methods. When notified, the worker runs
    /// immediately and the interval timer resets.
    IntervalOrNotify(Duration),

    /// Execute on cron schedule OR when manually notified.
    ///
    /// Returns [`CronOrNotifyHandle`](crate::CronOrNotifyHandle) with
    /// pause/resume and notify methods. Manual notifications trigger
    /// immediate execution without affecting the cron schedule.
    CronOrNotify(croner::Cron),
}
