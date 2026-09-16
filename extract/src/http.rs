// SPDX-License-Identifier: GPL-3.0-or-later

//! Fetching pages from hosts that are not always in a good mood.
//!
//! Embed hosts answer `500` or simply time out under load, and the failure is
//! usually gone a second later. Without a retry a single hiccup takes the
//! whole playback attempt down, which is exactly what happened twice while
//! bringing the player up.

use std::time::Duration;

use crate::{ExtractError, Result};

/// Attempts per request, including the first.
const MAX_ATTEMPTS: u32 = 3;

/// Delay before the second attempt; doubled for each one after.
const BASE_DELAY: Duration = Duration::from_millis(400);

/// Whether the failure is worth trying again.
///
/// Server errors and timeouts are transient. A `404`, a refused request or a
/// malformed URL will fail identically however many times it is sent.
fn worth_retrying(error: &reqwest::Error) -> bool {
    if error.is_timeout() || error.is_connect() {
        return true;
    }
    error.status().is_some_and(|status| {
        status.is_server_error() || status == reqwest::StatusCode::TOO_MANY_REQUESTS
    })
}

/// Sends a request, retrying transient failures with a widening delay.
///
/// The builder is cloned per attempt, so the request must not carry a
/// streaming body — none of the extractors do.
pub async fn send_with_retry(request: reqwest::RequestBuilder) -> Result<reqwest::Response> {
    let mut last: Option<reqwest::Error> = None;

    for attempt in 0..MAX_ATTEMPTS {
        let Some(attempt_request) = request.try_clone() else {
            // Cannot retry what cannot be cloned; send once and report plainly.
            return Ok(request.send().await?.error_for_status()?);
        };

        match attempt_request
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
        {
            Ok(response) => return Ok(response),
            Err(error) if worth_retrying(&error) => {
                tracing::debug!(
                    attempt = attempt + 1,
                    status = ?error.status(),
                    "host answered badly, retrying"
                );
                last = Some(error);
                if attempt + 1 < MAX_ATTEMPTS {
                    tokio::time::sleep(BASE_DELAY * 2u32.pow(attempt)).await;
                }
            }
            Err(error) => return Err(ExtractError::Network(error)),
        }
    }

    Err(ExtractError::Network(
        last.expect("a failure is recorded before the loop ends"),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delays_widen() {
        let delays: Vec<_> = (0..MAX_ATTEMPTS - 1)
            .map(|attempt| BASE_DELAY * 2u32.pow(attempt))
            .collect();
        assert_eq!(
            delays,
            vec![Duration::from_millis(400), Duration::from_millis(800)]
        );
    }

    #[test]
    fn total_wait_stays_short_enough_to_feel_responsive() {
        let total: Duration = (0..MAX_ATTEMPTS - 1)
            .map(|attempt| BASE_DELAY * 2u32.pow(attempt))
            .sum();
        assert!(
            total < Duration::from_secs(2),
            "a user is waiting on this: {total:?}"
        );
    }
}
