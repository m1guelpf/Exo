//! Serde adapters for Google's JSON encodings that serde can't express directly.
//!
//! `int64`/`uint64` fields travel as JSON strings; use `serde_with::DisplayFromStr` for those.
//! `bytes` fields travel as base64url, which is what [`Base64Url`] handles.

use base64::Engine;
use base64::alphabet;
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};
use serde::{Deserialize, Deserializer, Serializer};
use serde_with::{DeserializeAs, SerializeAs};

/// The encoding Gmail emits for `bytes` fields (`raw`, `body.data`): URL-safe alphabet, padded.
const ENCODE: GeneralPurpose = base64::engine::general_purpose::URL_SAFE;

/// Decoders for what Google (and its clients) send in practice: either alphabet, padding optional.
const DECODE_URL_SAFE: GeneralPurpose = GeneralPurpose::new(
    &alphabet::URL_SAFE,
    GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent),
);
const DECODE_STANDARD: GeneralPurpose = GeneralPurpose::new(
    &alphabet::STANDARD,
    GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent),
);

/// Encodes bytes the way Gmail does in `bytes` fields.
#[must_use]
pub fn encode_base64url(bytes: &[u8]) -> String {
    ENCODE.encode(bytes)
}

/// Decodes a `bytes` field, accepting both base64 alphabets with or without padding.
///
/// # Errors
/// Returns the decoder's error when the input is not base64 in either alphabet.
pub fn decode_base64url(text: &str) -> Result<Vec<u8>, base64::DecodeError> {
    DECODE_URL_SAFE
        .decode(text)
        .or_else(|_| DECODE_STANDARD.decode(text))
}

/// `serde_as` adapter for `bytes` fields (`Vec<u8>` ⇄ base64url string).
pub struct Base64Url;

impl SerializeAs<Vec<u8>> for Base64Url {
    fn serialize_as<S: Serializer>(source: &Vec<u8>, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&encode_base64url(source))
    }
}

impl<'de> DeserializeAs<'de, Vec<u8>> for Base64Url {
    fn deserialize_as<D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
        let text = String::deserialize(deserializer)?;
        decode_base64url(&text).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_url_safe_padded() {
        assert_eq!(encode_base64url(b"\xfb\xff"), "-_8=");
    }

    #[test]
    fn decodes_any_alphabet_and_padding() {
        for input in ["-_8=", "-_8", "+/8=", "+/8"] {
            assert_eq!(decode_base64url(input).unwrap(), b"\xfb\xff", "{input}");
        }
    }
}
