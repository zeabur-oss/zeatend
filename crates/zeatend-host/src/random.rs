//! Random text from the operating system's random number generator.
//!
//! AWS-LC reads getrandom(2), which waits until the kernel's pool is seeded.

use crate::HostError;

/// `n` lower-case hexadecimal digits.
///
/// # Errors
///
/// When the operating system's random number generator fails.
pub fn hex(n: usize) -> Result<String, HostError> {
    from(b"0123456789abcdef", n)
}

/// `n` characters of `alphabet`, each equally likely. A byte at or above the largest multiple of
/// the alphabet's length is skipped, because it would favour the first characters.
fn from(alphabet: &[u8], n: usize) -> Result<String, HostError> {
    let limit = 256 - 256_usize.checked_rem(alphabet.len()).unwrap_or(256);
    let mut out = String::with_capacity(n);
    let mut bytes = [0_u8; 64];
    while out.len() < n {
        aws_lc_rs::rand::fill(&mut bytes).map_err(|_unspecified| HostError::Random)?;
        for &b in bytes.iter().filter(|&&b| usize::from(b) < limit) {
            if out.len() == n {
                break;
            }
            if let Some(&c) = alphabet.get(usize::from(b) % alphabet.len()) {
                out.push(char::from(c));
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draws_only_from_the_alphabet() {
        let hex = hex(6).unwrap();
        assert_eq!(hex.len(), 6);
        assert!(
            hex.bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        );
        assert_ne!(super::hex(16).unwrap(), super::hex(16).unwrap());
    }
}
