use snafu::Snafu;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Snafu)]
pub enum Error {}
