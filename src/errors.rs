use hyper::StatusCode;
use std::fmt;

pub type GenericError = Box<dyn std::error::Error + Send + Sync>;
pub type Result<T> = std::result::Result<T, ApiError>;

#[derive(Debug)]
pub struct ApiError {
    pub message: String,
    pub code: StatusCode,
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "({}) {}", self.code, self.message)
    }
}

impl<T: std::error::Error> From<T> for ApiError {
    fn from(e: T) -> Self {
        ApiError {
            code: StatusCode::INTERNAL_SERVER_ERROR,
            message: format!("Internal: {} ", e),
        }
    }
}
