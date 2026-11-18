//! Join tokens for new workers: Kubernetes bootstrap tokens, as `k3s token create` and
//! `rke2 token create` make them.
//!
//! The token is a Secret in `kube-system`, so anyone allowed to create Secrets there can make one
//! through the API, and kube-controller-manager deletes it when it
//! expires. The K3s and RKE2 supervisors accept it on the agent routes only, so it cannot join a
//! server.

use std::time::Duration;

use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use zeatend_plan::cluster::Rancher;
use zeatend_plan::plan::MANAGER;

use crate::HostError;

/// How long a token joins workers unless told otherwise.
pub const TTL: Duration = Duration::from_hours(1);
/// The longest lifetime `parse_ttl` accepts.
pub const MAX_TTL: Duration = Duration::from_hours(24);

/// A bootstrap token, `<id>.<secret>`. `Debug` leaves the secret out.
#[derive(Clone, PartialEq, Eq)]
pub struct BootstrapToken {
    /// Six lowercase letters or digits.
    pub id: String,
    /// Sixteen lowercase letters or digits.
    pub secret: String,
}

impl std::fmt::Debug for BootstrapToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BootstrapToken")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl BootstrapToken {
    /// A new random token.
    ///
    /// # Errors
    ///
    /// When the operating system's random number generator fails.
    pub fn generate() -> Result<Self, HostError> {
        Ok(Self {
            id: crate::random::lower_alnum(6)?,
            secret: crate::random::lower_alnum(16)?,
        })
    }

    /// The token as K3s and RKE2 read it, pinned to the cluster CA: `K10<ca-hash>::<id>.<secret>`.
    #[must_use]
    pub fn joining(&self, ca_hash: &str) -> String {
        format!("K10{ca_hash}::{}.{}", self.id, self.secret)
    }

    /// The Secret holding it, expiring at `expires` (RFC 3339), in the group whose members the
    /// supervisor of the distribution lets join as agents.
    #[must_use]
    pub fn secret_object(&self, distribution: Rancher, description: &str, expires: &str) -> Value {
        json!({
            "apiVersion": "v1",
            "kind": "Secret",
            "type": "bootstrap.kubernetes.io/token",
            "metadata": {
                "name": format!("bootstrap-token-{}", self.id),
                "namespace": "kube-system",
                "labels": {"app.kubernetes.io/managed-by": MANAGER},
            },
            "stringData": {
                "token-id": self.id,
                "token-secret": self.secret,
                "description": description,
                "expiration": expires,
                "usage-bootstrap-authentication": "true",
                "usage-bootstrap-signing": "true",
                "auth-extra-groups": distribution.bootstrap_group(),
            },
        })
    }
}

/// A lifetime such as `30m` or `1h`, from one minute to a day.
#[must_use]
pub fn parse_ttl(text: &str) -> Option<Duration> {
    let text = text.trim();
    let (number, unit) = text.split_at(text.len().checked_sub(1)?);
    let n: u64 = number.parse().ok()?;
    let ttl = match unit {
        "m" => Duration::from_mins(n),
        "h" => Duration::from_hours(n),
        _ => return None,
    };
    (Duration::from_mins(1)..=MAX_TTL)
        .contains(&ttl)
        .then_some(ttl)
}

/// The CA hash of a join token, as k3s computes it for a CA file of one certificate: the SHA-256
/// of the PEM bytes exactly as stored. `None` for a bundle, whose hash is that of its root's DER.
#[must_use]
pub fn ca_hash(pem: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(pem).ok()?;
    if text.matches("-----BEGIN CERTIFICATE-----").count() != 1 {
        return None;
    }
    Some(hex::encode(Sha256::digest(pem)))
}

/// The CA hash a server token carries, `K10<hash>::server:<secret>`.
#[must_use]
pub fn hash_of_token(token: &str) -> Option<&str> {
    let (hash, _) = token.trim().strip_prefix("K10")?.split_once("::")?;
    (hash.len() == 64).then_some(hash)
}

/// `now + ttl` in RFC 3339, UTC, to the second.
#[must_use]
pub fn expires(ttl: Duration) -> String {
    let ttl = i64::try_from(ttl.as_secs()).unwrap_or(i64::MAX);
    rfc3339(jiff::Timestamp::now().as_second().saturating_add(ttl))
}

/// Seconds since the epoch in RFC 3339, UTC. Seconds outside jiff's range
/// (`-009999-01-02T01:59:59Z` to `9999-12-30T22:00:00Z`) are clamped to it.
#[must_use]
pub fn rfc3339(secs: i64) -> String {
    let secs = secs.clamp(
        jiff::Timestamp::MIN.as_second(),
        jiff::Timestamp::MAX.as_second(),
    );
    jiff::Timestamp::from_second(secs)
        .unwrap_or_default()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_fit_the_bootstrap_pattern() {
        let t = BootstrapToken::generate().unwrap();
        let joining = t.joining(&"a".repeat(64));
        let credentials = joining.split_once("::").unwrap().1;
        assert!(zeatend_plan::config::is_bootstrap_token(credentials));
        assert!(!format!("{t:?}").contains(&t.secret));
        let secret = t.secret_object(Rancher::Rke2, "zeatend node add", "2026-10-24T08:00:00Z");
        assert_eq!(
            secret["metadata"]["name"],
            format!("bootstrap-token-{}", t.id)
        );
        assert_eq!(secret["type"], "bootstrap.kubernetes.io/token");
        assert_eq!(
            secret["stringData"]["auth-extra-groups"],
            "system:bootstrappers:rke2:default-node-token"
        );
        let k3s = t.secret_object(Rancher::K3s, "", "2026-10-24T08:00:00Z");
        assert_eq!(
            k3s["stringData"]["auth-extra-groups"],
            "system:bootstrappers:k3s:default-node-token"
        );
        assert_eq!(
            secret["stringData"]["usage-bootstrap-authentication"],
            "true"
        );
    }

    #[test]
    fn ca_hashes() {
        let pem = b"-----BEGIN CERTIFICATE-----\nMIIB\n-----END CERTIFICATE-----\n";
        assert_eq!(
            ca_hash(pem).unwrap(),
            hex::encode(Sha256::digest(pem.as_slice()))
        );
        let bundle = [pem.as_slice(), pem.as_slice()].concat();
        assert_eq!(ca_hash(&bundle), None);
        let hash = "0123456789abcdef".repeat(4);
        assert_eq!(
            hash_of_token(&format!("K10{hash}::server:x\n")),
            Some(hash.as_str())
        );
        assert_eq!(hash_of_token("abcdef.0123456789abcdef"), None);
    }

    #[test]
    fn lifetimes() {
        assert_eq!(parse_ttl("30m"), Some(Duration::from_mins(30)));
        assert_eq!(parse_ttl("1h"), Some(Duration::from_hours(1)));
        assert_eq!(parse_ttl("24h"), Some(MAX_TTL));
        for bad in ["", "m", "0m", "25h", "90", "1d", "-1h", "1.5h"] {
            assert_eq!(parse_ttl(bad), None, "{bad}");
        }
    }

    #[test]
    fn dates() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(rfc3339(1_792_713_600 + 3_661), "2026-10-23T01:01:01Z");
        assert_eq!(rfc3339(i64::MAX), "9999-12-30T22:00:00Z");
    }
}
