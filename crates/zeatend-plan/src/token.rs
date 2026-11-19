//! The join token: `K10<ca-hash>::<credentials>`, or bare credentials without CA pinning.

use crate::PlanError;

/// A join token, split once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JoinToken<'a> {
    ca_hash: Option<&'a str>,
    credentials: &'a str,
}

impl<'a> JoinToken<'a> {
    /// Splits `text`, ignoring the whitespace around it. `K10<64 hex>::` pins the cluster CA and
    /// `K10::` does not.
    ///
    /// # Errors
    ///
    /// When the token is malformed or holds no credentials.
    pub fn parse(text: &'a str) -> Result<Self, PlanError> {
        let text = text.trim();
        let (ca_hash, credentials) = match text.strip_prefix("K10") {
            Some(rest) => {
                let (hash, credentials) = rest.split_once("::").ok_or_else(|| {
                    PlanError::Config("join token: expected K10<ca-hash>::<credentials>".to_owned())
                })?;
                let hex = hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit());
                if !hash.is_empty() && !hex {
                    return Err(PlanError::Config(
                        "join token: the CA hash is not 64 hexadecimal digits".to_owned(),
                    ));
                }
                ((!hash.is_empty()).then_some(hash), credentials)
            }
            None => (None, text),
        };
        if credentials.is_empty() || credentials.chars().any(char::is_whitespace) {
            return Err(PlanError::Config("join token: no credentials".to_owned()));
        }
        Ok(Self {
            ca_hash,
            credentials,
        })
    }

    /// Whether the token pins the cluster CA.
    #[must_use]
    pub const fn pins_ca(&self) -> bool {
        self.ca_hash.is_some()
    }

    /// The id of a bootstrap token, `[a-z0-9]{6}.[a-z0-9]{16}`, which `token delete` takes.
    #[must_use]
    pub fn bootstrap_id(&self) -> Option<&'a str> {
        let ok = |s: &str, n: usize| {
            s.len() == n
                && s.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        };
        self.credentials
            .split_once('.')
            .filter(|(id, secret)| ok(id, 6) && ok(secret, 16))
            .map(|(id, _)| id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HASH: &str = "2c2a4a3f5e0e5e7f3c1b1d9a8f7e6d5c4b3a29181716151413121110090807aa";

    #[test]
    fn the_ca_hash_is_optional() {
        let bare = JoinToken::parse(" abcdef.0123456789abcdef\n").unwrap();
        assert!(!bare.pins_ca());
        assert_eq!(bare.bootstrap_id(), Some("abcdef"));
        let text = format!("K10{HASH}::abcdef.0123456789abcdef");
        let pinned = JoinToken::parse(&text).unwrap();
        assert!(pinned.pins_ca());
        assert_eq!(pinned.bootstrap_id(), Some("abcdef"));
        assert!(!JoinToken::parse("K10::server:x").unwrap().pins_ca());
    }

    #[test]
    fn a_server_token_is_no_bootstrap_token() {
        let text = format!("K10{HASH}::server:s3cret");
        let token = JoinToken::parse(&text).unwrap();
        assert_eq!(token.bootstrap_id(), None);
    }

    #[test]
    fn a_malformed_token_is_refused() {
        for bad in ["K10abc::abcdef.0123456789abcdef", "K10", "K10::", "", "a b"] {
            assert!(JoinToken::parse(bad).is_err(), "{bad}");
        }
    }
}
