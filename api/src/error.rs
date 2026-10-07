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
    /// Made through `From`, which takes the token out of the URL first.
    #[error("network error: {0}")]
    Network(reqwest::Error),

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

impl From<reqwest::Error> for Error {
    /// reqwest names the request's URL in its message, and the session token
    /// travels in that URL's query: kept as it was, a timeout would put the
    /// token in the log, and the log in a bug report.
    fn from(mut error: reqwest::Error) -> Self {
        if let Some(url) = error.url_mut() {
            without_token(url);
        }
        Self::Network(error)
    }
}

fn without_token(url: &mut url::Url) {
    if !url.query_pairs().any(|(name, _)| name == "token") {
        return;
    }
    let kept: Vec<(String, String)> = url
        .query_pairs()
        .filter(|(name, _)| name != "token")
        .map(|(name, value)| (name.into_owned(), value.into_owned()))
        .collect();
    if kept.is_empty() {
        url.set_query(None);
    } else {
        url.query_pairs_mut().clear().extend_pairs(kept);
    }
}

pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_network_error_does_not_carry_the_token() {
        // Nothing listens on port 1: the request fails before any reply, the
        // way a timeout or a dropped connection does.
        let error = reqwest::Client::new()
            .get("http://127.0.0.1:1/profile/1?page=2&token=secret")
            .send()
            .await
            .expect_err("nothing listens on port 1");
        let error = Error::from(error);

        for text in [error.to_string(), format!("{error:?}")] {
            assert!(!text.contains("secret"), "{text}");
            assert!(text.contains("/profile/1?page=2"), "{text}");
        }
    }

    #[test]
    fn a_url_with_only_the_token_loses_its_query() {
        let mut url = url::Url::parse("https://example.com/me?token=secret").unwrap();
        without_token(&mut url);
        assert_eq!(url.as_str(), "https://example.com/me");
    }
}
