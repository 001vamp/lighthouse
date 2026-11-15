use toasty_proto::{ErrorCode, Failure};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{message}", message = .0.message)]
    Reported(Failure),
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Conflict(String),
    #[error("{0}")]
    Unavailable(String),
    #[error(transparent)]
    Store(#[from] toasty_store::StoreError),
    #[error(transparent)]
    Broker(#[from] toasty_broker::BrokerError),
    #[error(transparent)]
    Open(#[from] toasty_backend::OpenError),
    #[error(transparent)]
    Session(#[from] toasty_backend::SessionError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}
impl From<Error> for Failure {
    fn from(error: Error) -> Self {
        let code = match &error {
            Error::Reported(failure) => failure.code.clone(),
            Error::Invalid(_) | Error::Broker(_) | Error::Json(_) => ErrorCode::InvalidInput,
            Error::NotFound(_) => ErrorCode::NotFound,
            Error::Conflict(_) => ErrorCode::Conflict,
            Error::Store(toasty_store::StoreError::InvalidInput(_)) => ErrorCode::InvalidInput,
            Error::Store(toasty_store::StoreError::ThreadNotFound(_)) => ErrorCode::NotFound,
            Error::Store(_) => ErrorCode::Storage,
            Error::Open(_) | Error::Session(_) => ErrorCode::Agent,
            Error::Unavailable(_) | Error::Io(_) => ErrorCode::Unavailable,
        };
        Self {
            code,
            message: error.to_string(),
        }
    }
}
pub type Result<T> = std::result::Result<T, Error>;
