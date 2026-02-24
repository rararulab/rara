use snafu::Snafu;

#[derive(Debug, Snafu)]
#[snafu(visibility(pub(crate)))]
pub enum ConsulError {
    #[snafu(display("HTTP request to Consul failed: {source}"))]
    HttpRequest { source: reqwest::Error },

    #[snafu(display("Consul returned error status: {message}"))]
    HttpStatus { message: String },

    #[snafu(display("failed to decode Consul JSON response: {source}"))]
    JsonDecode { source: reqwest::Error },

    #[snafu(display("failed to decode base64 value for key {key}: {source}"))]
    Base64Decode {
        key:    String,
        source: base64::DecodeError,
    },

    #[snafu(display("value for key {key} is not valid UTF-8: {source}"))]
    Utf8Decode {
        key:    String,
        source: std::string::FromUtf8Error,
    },
}
