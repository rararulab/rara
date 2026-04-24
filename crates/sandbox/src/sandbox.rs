//! Concrete [`Sandbox`] handle backed by a boxlite `LiteBox`.

use boxlite::{
    BoxCommand, BoxOptions, BoxliteRuntime, ExecStderr, ExecStdout, Execution, LiteBox, RootfsSpec,
};
use snafu::ResultExt;
use tracing::instrument;

use crate::{
    config::{ExecRequest, SandboxConfig},
    error::{BoxliteSnafu, MissingStdoutSnafu, Result},
};

/// Live handle to a hardware-isolated sandbox.
///
/// Owning a `Sandbox` implies the underlying boxlite VM has been created but
/// not necessarily booted — boxlite starts the VM lazily on the first
/// [`Sandbox::exec`] call. To reclaim resources, call [`Sandbox::destroy`];
/// dropping the handle alone leaves the VM registered in the boxlite
/// runtime's state.
pub struct Sandbox {
    /// Shared reference to the process-wide boxlite runtime. `default_runtime`
    /// returns a `&'static` singleton, so the `'static` bound is free.
    runtime:  &'static BoxliteRuntime,
    /// boxlite's own handle to this specific box.
    litebox:  LiteBox,
    /// Cached name (if the caller supplied one) for use as the removal key.
    box_name: Option<String>,
}

/// Streams and completion future returned by a single [`Sandbox::exec`] call.
///
/// Callers drive `stdout` / `stderr` to consume output and `await` the
/// `execution` future (via [`Execution::wait`]) to obtain the exit status.
///
/// `stdout` is returned as a concrete [`ExecStdout`] which already
/// implements [`futures::Stream<Item = String>`](futures::Stream), matching
/// the API described in issue #1698.
pub struct ExecOutcome {
    /// Line-delimited stdout stream. Always present.
    pub stdout:    ExecStdout,
    /// Line-delimited stderr stream. `None` if boxlite declined to
    /// materialise one (e.g. tty mode).
    pub stderr:    Option<ExecStderr>,
    /// The underlying boxlite execution handle. Call [`Execution::wait`] on
    /// it after the streams drain to retrieve the exit status.
    pub execution: Execution,
}

impl Sandbox {
    /// Create a new sandbox from a [`SandboxConfig`].
    ///
    /// Uses boxlite's process-wide default runtime. The VM is registered but
    /// not booted; the first [`Sandbox::exec`] call pays that cost.
    #[instrument(skip_all, fields(image = %config.rootfs_image))]
    pub async fn create(config: SandboxConfig) -> Result<Self> {
        let runtime = BoxliteRuntime::default_runtime();
        let options = BoxOptions {
            rootfs: RootfsSpec::Image(config.rootfs_image),
            ..Default::default()
        };
        let litebox = runtime
            .create(options, config.name.clone())
            .await
            .context(BoxliteSnafu)?;
        Ok(Self {
            runtime,
            litebox,
            box_name: config.name,
        })
    }

    /// Execute a single command inside the sandbox.
    ///
    /// Returns the stdout stream plus the underlying [`Execution`] handle so
    /// callers retain access to stderr, stdin, and the exit status without
    /// this crate having to re-export every boxlite surface.
    #[instrument(skip_all, fields(command = %request.command))]
    pub async fn exec(&self, request: ExecRequest) -> Result<ExecOutcome> {
        let mut command = BoxCommand::new(request.command).args(request.args);
        for (key, value) in request.env {
            command = command.env(key, value);
        }
        if let Some(timeout) = request.timeout {
            command = command.timeout(timeout);
        }

        let mut execution = self.litebox.exec(command).await.context(BoxliteSnafu)?;
        // boxlite hands out stdout via `take`-style semantics; missing means
        // another consumer already grabbed it — surface that as an error
        // instead of silently degrading.
        let stdout = execution.stdout().ok_or(MissingStdoutSnafu.build())?;
        let stderr = execution.stderr();
        Ok(ExecOutcome {
            stdout,
            stderr,
            execution,
        })
    }

    /// Remove the underlying box from the boxlite runtime.
    ///
    /// Uses `force = true` to tear down even if the VM is still running.
    /// After this call the sandbox handle is consumed; create a new one if
    /// further work is needed.
    #[instrument(skip_all)]
    pub async fn destroy(self) -> Result<()> {
        // Prefer the user-supplied name, but fall back to the box ID because
        // `name()` only exists when the caller passed one in `SandboxConfig`.
        let key = self
            .box_name
            .unwrap_or_else(|| self.litebox.id().to_string());
        self.runtime
            .remove(&key, true)
            .await
            .context(BoxliteSnafu)?;
        Ok(())
    }

    /// Read-only access to the wrapped boxlite handle.
    ///
    /// Exposed for integration tests and advanced callers that need a
    /// boxlite knob we have not yet lifted into this crate's public API.
    pub fn litebox(&self) -> &LiteBox { &self.litebox }
}
