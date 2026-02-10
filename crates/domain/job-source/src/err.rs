use snafu::Snafu;

/// Errors that a job source driver can produce.
///
/// The variants carry enough information for callers to decide whether
/// to retry, back off, or give up.
#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum SourceError {
    /// A transient failure that can be retried.
    #[snafu(display("Retryable error from source '{source_name}': {message}"))]
    Retryable {
        source_name: String,
        message:     String,
    },

    /// A permanent failure that should not be retried.
    #[snafu(display("Non-retryable error from source '{source_name}': {message}"))]
    NonRetryable {
        source_name: String,
        message:     String,
    },

    /// The source has rate-limited us.
    #[snafu(display("Rate limited by source '{source_name}', retry after {retry_after_secs}s"))]
    RateLimited {
        source_name:      String,
        retry_after_secs: u64,
    },

    /// Authentication / authorization failure.
    #[snafu(display("Auth error for source '{source_name}': {message}"))]
    AuthError {
        source_name: String,
        message:     String,
    },

    /// The raw data could not be normalized into a valid
    /// [`NormalizedJob`].
    #[snafu(display(
        "Normalization failed for job '{source_job_id}' from '{source_name}': {message}"
    ))]
    NormalizationFailed {
        source_name:   String,
        source_job_id: String,
        message:       String,
    },
}
