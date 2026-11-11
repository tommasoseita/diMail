use base64::Engine;
use base64::alphabet;
use base64::engine::general_purpose::{GeneralPurpose, GeneralPurposeConfig, URL_SAFE_NO_PAD};
use base64::engine::DecodePaddingMode;

use crate::{Error, Result};

/// Millisecondi dall'epoch Unix.
pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

pub fn random_bytes<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    getrandom::fill(&mut b).expect("generatore casuale del sistema non disponibile");
    b
}

pub fn b64url(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

const URL_SAFE_LENIENT: GeneralPurpose = GeneralPurpose::new(
    &alphabet::URL_SAFE,
    GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent),
);

/// Decodifica base64url con o senza padding (Gmail usa entrambe le forme).
pub fn b64url_decode(s: &str) -> Result<Vec<u8>> {
    let cleaned: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    URL_SAFE_LENIENT
        .decode(cleaned.as_bytes())
        .map_err(|e| Error::other(format!("base64 non valido: {e}")))
}

/// Gli ID di Gmail (`historyId`, `internalDate`) arrivano come stringhe di interi.
pub fn parse_i64(s: Option<&str>) -> Option<i64> {
    s.and_then(|v| v.parse::<i64>().ok())
}

/// Jitter in [0, max_ms).
pub fn jitter_ms(max_ms: u64) -> u64 {
    if max_ms == 0 {
        return 0;
    }
    let b = random_bytes::<8>();
    u64::from_le_bytes(b) % max_ms
}

/// Attesa crescente: 1s, 2s, 4s, ... fino a `cap_ms`, con jitter.
pub fn backoff_ms(attempt: u32, cap_ms: u64) -> u64 {
    let base = 1000u64.saturating_mul(1u64 << attempt.min(16));
    base.min(cap_ms) + jitter_ms(500)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_with_and_without_padding() {
        assert_eq!(b64url_decode("aGk").unwrap(), b"hi");
        assert_eq!(b64url_decode("aGk=").unwrap(), b"hi");
        assert_eq!(b64url_decode("-_8").unwrap(), vec![0xfb, 0xff]);
    }

    #[test]
    fn backoff_grows_and_caps() {
        assert!(backoff_ms(0, 60_000) < 1_500);
        assert!(backoff_ms(3, 60_000) >= 8_000);
        assert!(backoff_ms(30, 60_000) <= 60_500);
    }
}
