//! The env file (now botconf's own, for imports too) and new tokens.

use std::fmt::Write as _;

pub use botconf::env_file::EnvFile;

/// A new random token: 32 bytes from the OS, as 64 hex digits.
pub fn generate_token() -> Result<String, String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|error| format!("no randomness: {error}"))?;
    Ok(bytes
        .iter()
        .fold(String::with_capacity(64), |mut out, byte| {
            let _ = write!(out, "{byte:02x}");
            out
        }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_new_each_time() {
        let (first, second) = (generate_token().unwrap(), generate_token().unwrap());
        assert_eq!(first.len(), 64);
        assert!(first.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(first, second);
    }
}
