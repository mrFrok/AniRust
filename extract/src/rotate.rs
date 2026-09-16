// SPDX-License-Identifier: GPL-3.0-or-later

//! Undoes the "rotate the alphabet, then base64" obfuscation some hosts apply
//! to stream URLs before putting them in a JSON response.
//!
//! The shift is deliberately *not* hardcoded. A sample captured in September
//! 2026 used 18, but the shift is a one-character server-side change and
//! nothing stops it moving. Since a wrong shift produces either invalid base64
//! or bytes that are not a URL, trying all 26 and keeping the one that yields a
//! URL is both more robust than a constant and self-validating — at a cost of
//! at most 26 base64 decodes of a ~200 byte string.

use base64::Engine;
use base64::engine::general_purpose::{STANDARD_NO_PAD, URL_SAFE_NO_PAD};

/// Rotates ASCII letters by `shift`, leaving every other byte untouched.
#[must_use]
pub fn rotate(input: &str, shift: u8) -> String {
    let shift = shift % 26;
    input
        .chars()
        .map(|c| match c {
            'a'..='z' => shift_char(c, b'a', shift),
            'A'..='Z' => shift_char(c, b'A', shift),
            other => other,
        })
        .collect()
}

fn shift_char(c: char, base: u8, shift: u8) -> char {
    let offset = (c as u8) - base;
    char::from(base + (offset + shift) % 26)
}

/// Recovers a URL from a rotated-then-base64 string.
///
/// Returns `None` when no rotation yields a URL, which is the signal that the
/// host changed its encoding rather than merely its shift.
#[must_use]
pub fn decode_url(encoded: &str) -> Option<String> {
    // Padding is inconsistent across hosts, so normalise it away and decode
    // without it.
    let trimmed = encoded.trim_end_matches('=');

    (0..26u8).find_map(|shift| {
        let rotated = rotate(trimmed, shift);
        let bytes = STANDARD_NO_PAD
            .decode(&rotated)
            .or_else(|_| URL_SAFE_NO_PAD.decode(&rotated))
            .ok()?;
        let text = String::from_utf8(bytes).ok()?;
        looks_like_url(&text).then(|| normalize_scheme(text))
    })
}

fn looks_like_url(s: &str) -> bool {
    // Protocol-relative URLs are common in these payloads.
    (s.starts_with("https://") || s.starts_with("http://") || s.starts_with("//"))
        && !s.contains(char::is_whitespace)
}

/// Gives a protocol-relative URL an explicit scheme, since players and HTTP
/// clients need one.
fn normalize_scheme(url: String) -> String {
    if let Some(rest) = url.strip_prefix("//") {
        format!("https://{rest}")
    } else {
        url
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Captured from a live response in September 2026. Kept verbatim as the
    /// regression anchor for the decoder.
    const OBSERVED: &str = "iPZ0kPU6Tg9hi3sck29aj2ZrHO4cG29bT2NciE1tlPHhHFRxHFUdUrG4VBlpWEQeVEQfG2Mh\
GrC3ULs0UEDqUhU2G2HpGBVsHLsfHOU1UA8hVBIhGECfWEHsWLk5HuQeGuZrVhRtGuDsGEM3HOCgWLwgULQ2ULsfVrMhThQ0UK5b\
kLY6iOfhWu1pjutuHFV0Tu0hlBo";

    const EXPECTED: &str = "https://sky.solodcdn.com/animetvseries/\
26857a9b05b1ca3b570941eb336cfaa3dd91dc51/3503ae19fd879fb0bdc72ebedaa7de28:2026091613/\
240.mp4:hls:manifest.m3u8";

    #[test]
    fn decodes_an_observed_payload() {
        assert_eq!(decode_url(OBSERVED).as_deref(), Some(EXPECTED));
    }

    #[test]
    fn rotation_round_trips() {
        let original = "Hello, World! 123";
        for shift in 0..26 {
            assert_eq!(rotate(&rotate(original, shift), 26 - shift), original);
        }
    }

    #[test]
    fn rotation_leaves_non_letters_alone() {
        assert_eq!(rotate("ab-12_YZ", 1), "bc-12_ZA");
    }

    /// The whole point of searching: a different shift must still decode.
    #[test]
    fn any_shift_decodes() {
        let plain = "https://cdn.example.com/a/b/manifest.m3u8";
        for shift in 0..26u8 {
            let encoded = rotate(&STANDARD_NO_PAD.encode(plain), shift);
            assert_eq!(
                decode_url(&encoded).as_deref(),
                Some(plain),
                "failed at shift {shift}"
            );
        }
    }

    #[test]
    fn protocol_relative_gets_a_scheme() {
        let encoded = STANDARD_NO_PAD.encode("//cdn.example.com/x.m3u8");
        assert_eq!(
            decode_url(&encoded).as_deref(),
            Some("https://cdn.example.com/x.m3u8")
        );
    }

    #[test]
    fn padding_is_tolerated() {
        // Length deliberately not a multiple of 3, so the encoder emits '='.
        let plain = "https://cdn.example.com/qq.m3u8";
        let padded = base64::engine::general_purpose::STANDARD.encode(plain);
        assert!(padded.ends_with('='), "test needs a padded sample");
        assert_eq!(decode_url(&padded).as_deref(), Some(plain));
    }

    #[test]
    fn non_url_payloads_are_rejected() {
        assert_eq!(decode_url(&STANDARD_NO_PAD.encode("just some text")), None);
        assert_eq!(decode_url("!!!not base64!!!"), None);
        assert_eq!(decode_url(""), None);
    }
}
