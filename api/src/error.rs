// SPDX-License-Identifier: GPL-3.0-or-later

use std::fmt;

/// Status code carried by every Anixart response body, independently of the
/// HTTP status. Mirrors `com.swiftsoft.anixartd.network.Response`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiCode {
    Successful,
    Failed,
    Banned,
    PermBanned,
    /// A code this client has not seen before. Kept rather than rejected so a
    /// server-side addition degrades into a reportable error instead of a
    /// deserialization failure.
    Unknown(i32),
}

impl ApiCode {
    pub const SUCCESSFUL: i32 = 0;
    pub const FAILED: i32 = 1;
    pub const BANNED: i32 = 402;
    pub const PERM_BANNED: i32 = 403;

    pub fn from_raw(code: i32) -> Self {
        match code {
            Self::SUCCESSFUL => Self::Successful,
            Self::FAILED => Self::Failed,
            Self::BANNED => Self::Banned,
            Self::PERM_BANNED => Self::PermBanned,
            other => Self::Unknown(other),
        }
    }

    pub fn is_success(self) -> bool {
        matches!(self, Self::Successful)
    }

    pub fn raw(self) -> i32 {
        match self {
            Self::Successful => Self::SUCCESSFUL,
            Self::Failed => Self::FAILED,
            Self::Banned => Self::BANNED,
            Self::PermBanned => Self::PERM_BANNED,
            Self::Unknown(c) => c,
        }
    }
}

impl fmt::Display for ApiCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Successful => write!(f, "successful"),
            Self::Failed => write!(f, "failed"),
            Self::Banned => write!(f, "banned"),
            Self::PermBanned => write!(f, "permanently banned"),
            Self::Unknown(c) => write!(f, "unknown code {c}"),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),

    #[error("invalid url: {0}")]
    Url(#[from] url::ParseError),

    /// HTTP was fine but the body carried a non-zero `code`.
    #[error("api rejected the request: {code}")]
    Api { code: ApiCode },

    /// Authentication is required for this endpoint and no token is set.
    #[error("this endpoint requires a token, but the client is anonymous")]
    Unauthenticated,

    #[error("http status {status}")]
    Status { status: reqwest::StatusCode },

    /// The body did not match the expected shape. Carries the raw payload so a
    /// server-side schema change can be diagnosed from a log rather than a
    /// packet capture.
    #[error("could not decode response: {source}")]
    Decode {
        #[source]
        source: serde_json::Error,
        body: String,
    },
}

impl Error {
    /// True when retrying the same request might succeed.
    pub fn is_transient(&self) -> bool {
        match self {
            Self::Network(e) => e.is_timeout() || e.is_connect(),
            Self::Status { status } => {
                status.is_server_error() || *status == reqwest::StatusCode::TOO_MANY_REQUESTS
            }
            _ => false,
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;
