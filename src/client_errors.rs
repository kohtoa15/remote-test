use std::error::Error;
use std::fmt::Display;

/// Wrapper for errors that we know will end up as ErrorSource::Local
pub struct LocalError(pub Box<dyn Error>);

impl From<Box<dyn Error>> for LocalError {
    fn from(e: Box<dyn Error>) -> Self { Self(e) }
}

impl From<LocalError> for Box<dyn Error> {
    fn from(e: LocalError) -> Self { e.0 }
}

#[derive(Debug)]
pub enum ErrorSource {
    /// Connection prevented by error
    FailedConnect,
    /// Errors reported on server side
    Remote,
    /// Error occurring locally with this client
    Local,
}

impl Display for ErrorSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self {
            Self::FailedConnect => write!(f, "Failed to connect to remote host"),
            Self::Remote => write!(f, "Error reported by remote host"),
            Self::Local => write!(f, "Error occurred in client"),
        }
    }
}

#[derive(Debug)]
pub struct ClientError {
    source: ErrorSource,
    cause: Box<dyn Error>,
}

impl Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.source, self.cause)
    }
}

impl Error for ClientError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.cause.source()
    }
}

impl ClientError {
    pub fn failed_connect(e: impl Into<Box<dyn Error>>) -> Self {
        ClientError { source: ErrorSource::FailedConnect, cause: e.into() }
    }

    pub fn remote(e: impl Into<Box<dyn Error>>) -> Self {
        ClientError { source: ErrorSource::Remote, cause: e.into() }
    }

    pub fn local(e: impl Into<Box<dyn Error>>) -> Self {
        ClientError { source: ErrorSource::Local, cause: e.into() }
    }
}

impl From<LocalError> for ClientError {
    fn from(e: LocalError) -> Self {
        ClientError::local(e)
    }
}
