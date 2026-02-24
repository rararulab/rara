use snafu::Snafu;

#[derive(Debug, Snafu)]
#[snafu(visibility(pub(crate)))]
pub enum InfisicalError {
    #[snafu(display("failed to create Infisical client: {source}"))]
    ClientBuild { source: infisical::InfisicalError },

    #[snafu(display("failed to authenticate with Infisical: {source}"))]
    Auth { source: infisical::InfisicalError },

    #[snafu(display("failed to list secrets: {source}"))]
    ListSecrets { source: infisical::InfisicalError },
}
