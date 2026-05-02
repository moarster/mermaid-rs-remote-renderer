//! Decode the path segment of `/svg/:encoded` into a Mermaid source.
//!
//! Two formats are accepted, matching the kroki / mermaid.ink convention used
//! by the upstream renderer:
//!
//! - Plain: `base64url(source)` (with or without `=` padding).
//! - Compressed: `pako:` + `base64url(zlib_deflate(json))`, where the JSON is
//!   `{"code": "...", "mermaid": {"theme": "..."}}`.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;

#[derive(Debug, PartialEq, Eq)]
pub struct DecodedRequest {
    pub source: String,
    pub theme: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error("invalid base64url: {0}")]
    Base64(#[from] base64::DecodeError),
    #[error("zlib inflate failed: {0}")]
    Inflate(String),
    #[error("payload is not valid UTF-8")]
    Utf8,
    #[error("invalid JSON payload: {0}")]
    Json(#[from] serde_json::Error),
    #[error("encoded payload is empty")]
    Empty,
}

pub fn decode_request(encoded: &str) -> Result<DecodedRequest, DecodeError> {
    if encoded.is_empty() {
        return Err(DecodeError::Empty);
    }

    if let Some(rest) = encoded.strip_prefix("pako:") {
        let compressed = b64_decode(rest)?;
        let inflated = miniz_oxide::inflate::decompress_to_vec_zlib(&compressed)
            .map_err(|e| DecodeError::Inflate(format!("{e:?}")))?;
        let json = std::str::from_utf8(&inflated).map_err(|_| DecodeError::Utf8)?;
        let payload: PakoPayload = serde_json::from_str(json)?;
        Ok(DecodedRequest {
            source: payload.code,
            theme: payload.mermaid.and_then(|m| m.theme),
        })
    } else {
        let bytes = b64_decode(encoded)?;
        let source = String::from_utf8(bytes).map_err(|_| DecodeError::Utf8)?;
        Ok(DecodedRequest {
            source,
            theme: None,
        })
    }
}

fn b64_decode(input: &str) -> Result<Vec<u8>, DecodeError> {
    // Strip any padding so URL_SAFE_NO_PAD accepts both padded and non-padded inputs.
    let trimmed = input.trim_end_matches('=');
    URL_SAFE_NO_PAD
        .decode(trimmed.as_bytes())
        .map_err(DecodeError::from)
}

#[derive(Debug, Deserialize)]
struct PakoPayload {
    code: String,
    #[serde(default)]
    mermaid: Option<MermaidConfig>,
}

#[derive(Debug, Deserialize)]
struct MermaidConfig {
    #[serde(default)]
    theme: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::URL_SAFE;
    use miniz_oxide::deflate::compress_to_vec_zlib;

    const SRC: &str = "flowchart LR; A-->B";

    #[test]
    fn decodes_plain_base64url_padded() {
        let encoded = URL_SAFE.encode(SRC.as_bytes());
        let decoded = decode_request(&encoded).unwrap();
        assert_eq!(decoded.source, SRC);
        assert!(decoded.theme.is_none());
    }

    #[test]
    fn decodes_plain_base64url_no_pad() {
        let encoded = URL_SAFE_NO_PAD.encode(SRC.as_bytes());
        let decoded = decode_request(&encoded).unwrap();
        assert_eq!(decoded.source, SRC);
    }

    #[test]
    fn decodes_pako_payload_with_theme() {
        let json = format!(
            "{{ \"code\":\"{}\", \"mermaid\":{{\"theme\":\"default\"}}}}",
            SRC
        );
        // zlib level 6 matches Java's `new Deflater()` (DEFAULT_COMPRESSION).
        let compressed = compress_to_vec_zlib(json.as_bytes(), 6);
        let encoded = format!("pako:{}", URL_SAFE.encode(&compressed));
        let decoded = decode_request(&encoded).unwrap();
        assert_eq!(decoded.source, SRC);
        assert_eq!(decoded.theme.as_deref(), Some("default"));
    }

    #[test]
    fn decodes_pako_payload_without_theme() {
        let json = format!("{{\"code\":\"{SRC}\"}}");
        let compressed = compress_to_vec_zlib(json.as_bytes(), 6);
        let encoded = format!("pako:{}", URL_SAFE.encode(&compressed));
        let decoded = decode_request(&encoded).unwrap();
        assert_eq!(decoded.source, SRC);
        assert!(decoded.theme.is_none());
    }

    #[test]
    fn rejects_empty_input() {
        assert!(matches!(decode_request(""), Err(DecodeError::Empty)));
    }

    #[test]
    fn rejects_garbage_base64() {
        let err = decode_request("!!!not-base64!!!").unwrap_err();
        assert!(matches!(err, DecodeError::Base64(_)));
    }

    #[test]
    fn rejects_pako_with_invalid_zlib() {
        // Valid base64url but not a valid zlib stream.
        let bogus = URL_SAFE_NO_PAD.encode(b"definitely not zlib");
        let err = decode_request(&format!("pako:{bogus}")).unwrap_err();
        assert!(matches!(err, DecodeError::Inflate(_)));
    }

    #[test]
    fn rejects_pako_with_non_json_payload() {
        let compressed = compress_to_vec_zlib(b"<not json>", 6);
        let encoded = format!("pako:{}", URL_SAFE.encode(&compressed));
        let err = decode_request(&encoded).unwrap_err();
        assert!(matches!(err, DecodeError::Json(_)));
    }
}
