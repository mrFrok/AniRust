// SPDX-License-Identifier: GPL-3.0-or-later

//! Deserialization helpers for an API that returns `null` where its own
//! nominal types say it will not.
//!
//! A single `search/releases` response was observed returning `null` for
//! `author`, `director`, `note`, `release_date`, `studio`, `translators` and
//! `year` (declared as strings) as well as `episodes_released`,
//! `episodes_total` and `profile_list_status` (declared as integers).
//!
//! Serde's container-level `default` covers *missing* fields only, so an
//! explicit `null` still fails the whole response. Since the API is
//! undocumented and any field can start returning `null` after a server-side
//! change, [`nullable`] is applied to every field of every response model
//! rather than only to the ones seen failing.

use serde::{Deserialize, Deserializer};

/// Deserializes `T`, mapping an explicit `null` to `T::default()`.
pub fn nullable<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Default, Deserialize, PartialEq)]
    #[serde(default)]
    struct Sample {
        #[serde(deserialize_with = "nullable")]
        text: String,
        #[serde(deserialize_with = "nullable")]
        number: i32,
        #[serde(deserialize_with = "nullable")]
        list: Vec<i32>,
    }

    #[test]
    fn explicit_nulls_become_defaults() {
        let parsed: Sample =
            serde_json::from_str(r#"{"text":null,"number":null,"list":null}"#).unwrap();
        assert_eq!(parsed, Sample::default());
    }

    #[test]
    fn missing_fields_become_defaults() {
        let parsed: Sample = serde_json::from_str("{}").unwrap();
        assert_eq!(parsed, Sample::default());
    }

    #[test]
    fn present_values_are_kept() {
        let parsed: Sample =
            serde_json::from_str(r#"{"text":"ok","number":7,"list":[1,2]}"#).unwrap();
        assert_eq!(
            parsed,
            Sample {
                text: "ok".to_owned(),
                number: 7,
                list: vec![1, 2],
            }
        );
    }

    #[test]
    fn unknown_fields_are_ignored() {
        let parsed: Sample = serde_json::from_str(r#"{"text":"ok","brand_new":123}"#).unwrap();
        assert_eq!(parsed.text, "ok");
    }
}
