//! Shared capability-token helpers for loopback control services.

use rand::RngCore;

pub const MIN_TOKEN_BYTES: usize = 32;

pub fn generate_token() -> String {
    let mut bytes = [0u8; MIN_TOKEN_BYTES];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

pub fn validate_token(token: &str) -> Result<(), String> {
    if token.len() < MIN_TOKEN_BYTES {
        return Err(format!(
            "capability token must contain at least {MIN_TOKEN_BYTES} bytes"
        ));
    }
    if !token.bytes().all(|byte| (0x21..=0x7e).contains(&byte)) {
        return Err(
            "capability token must contain only visible ASCII without whitespace".to_string(),
        );
    }
    Ok(())
}

pub fn bearer_matches(supplied: Option<&str>, expected_token: &str) -> bool {
    let expected = format!("Bearer {expected_token}");
    supplied.is_some_and(|value| constant_time_eq(value.as_bytes(), expected.as_bytes()))
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let mut difference = left.len() ^ right.len();
    let max_len = left.len().max(right.len());
    for index in 0..max_len {
        let left_byte = left.get(index).copied().unwrap_or(0);
        let right_byte = right.get(index).copied().unwrap_or(0);
        difference |= usize::from(left_byte ^ right_byte);
    }
    difference == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_tokens_are_strong_and_bearer_comparison_is_exact() {
        let token = generate_token();
        assert_eq!(token.len(), MIN_TOKEN_BYTES * 2);
        assert!(bearer_matches(Some(&format!("Bearer {token}")), &token));
        assert!(!bearer_matches(Some(&format!("bearer {token}")), &token));
        assert!(!bearer_matches(Some("Bearer wrong"), &token));
        assert!(!bearer_matches(None, &token));
    }

    #[test]
    fn weak_tokens_are_rejected() {
        assert!(validate_token("too-short").is_err());
        assert!(validate_token("0123456789abcdef0123456789abcdef").is_ok());
        assert!(validate_token("0123456789abcdef 123456789abcdef").is_err());
        assert!(validate_token("0123456789abcdef\n123456789abcdef").is_err());
        assert!(validate_token("éééééééééééééééé").is_err());
    }
}
