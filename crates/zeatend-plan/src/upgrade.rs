//! What an `apply` on an installed cluster may change: no older release, and Kubernetes one
//! minor version at a time.
//!
//! The Kubernetes version skew policy (<https://kubernetes.io/releases/version-skew-policy/>)
//! upgrades `kube-apiserver` from 1.(n-1) to 1.n only, and a kubelet must not be newer than
//! `kube-apiserver`. K3s says the same for its releases: "upgrade server nodes first one at a time,
//! then any agent nodes", and an upgrade must "not skip intermediate minor versions"
//! (<https://docs.k3s.io/upgrades/manual>).

use std::cmp::Ordering;

use crate::PlanError;
use crate::cluster::Distribution;
use crate::config::Role;
use crate::record::Record;

/// A Kubernetes version: the minor, and the patch when it is known.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Kubernetes {
    /// `37` in 1.37.
    pub minor: u32,
    /// `2` in 1.37.2.
    pub patch: Option<u32>,
}

impl Kubernetes {
    /// Reads `1.37`, `v1.37.2` or a `gitVersion` such as `v1.37.2+k3s1`. `None` for another
    /// major version or text.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.strip_prefix('v').unwrap_or(text);
        let text = text.split_once(['+', '-']).map_or(text, |(v, _)| v);
        let mut parts = text.split('.');
        if parts.next()? != "1" {
            return None;
        }
        let minor = parts.next()?.parse().ok()?;
        let patch = match parts.next() {
            Some(p) => Some(p.parse().ok()?),
            None => None,
        };
        parts.next().is_none().then_some(Self { minor, patch })
    }
}

impl std::fmt::Display for Kubernetes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.patch {
            Some(patch) => write!(f, "1.{}.{patch}", self.minor),
            None => write!(f, "1.{}", self.minor),
        }
    }
}

/// The numbers of a release tag, `v1.37.2+k3s1` as `[1, 37, 2, 1]` and `v1.3.0` as
/// `[1, 3, 0, 0]`, so that tags of one distribution order as their releases do.
fn release_numbers(tag: &str) -> Option<[u32; 4]> {
    let tag = tag.strip_prefix('v')?;
    let (version, build) = tag.split_once('+').unwrap_or((tag, ""));
    let mut numbers = [0; 4];
    let mut parts = version.split('.');
    for n in numbers.iter_mut().take(3) {
        *n = parts.next()?.parse().ok()?;
    }
    if parts.next().is_some() {
        return None;
    }
    // The build's last number: `1` in `k3s1` and `rke2r1`.
    let digits = build
        .rsplit(|c: char| !c.is_ascii_digit())
        .next()
        .unwrap_or_default();
    if !digits.is_empty() {
        numbers[3] = digits.parse().ok()?;
    }
    Some(numbers)
}

/// The release tag in the `--version` output of K3s, RKE2 or `KubeSolo`: `v1.36.5+k3s1` in
/// `k3s version v1.36.5+k3s1 (abc)`, `v1.2.3` in `KubeSolo`'s `{"version":"v1.2.3",...}`.
#[must_use]
pub fn release_tag(version_output: &str) -> Option<String> {
    version_output
        .split(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '+' | '-')))
        .find(|word| {
            word.strip_prefix('v').is_some_and(|rest| {
                rest.starts_with(|c: char| c.is_ascii_digit()) && rest.contains('.')
            })
        })
        .map(str::to_owned)
}

/// Stops an `apply` that would install an older release of the same distribution than the
/// record names.
///
/// # Errors
///
/// [`PlanError::Downgrade`].
pub fn check_release(
    record: Option<&Record>,
    distribution: Distribution,
    planned: &str,
) -> Result<(), PlanError> {
    let Some(record) = record else {
        return Ok(());
    };
    if record
        .distribution
        .as_deref()
        .is_some_and(|d| d != distribution.id())
    {
        return Ok(());
    }
    let Some(installed) = record.version.as_deref() else {
        return Ok(());
    };
    match (release_numbers(installed), release_numbers(planned)) {
        (Some(have), Some(want)) if want.cmp(&have) == Ordering::Less => {
            Err(PlanError::Downgrade {
                title: distribution.title(),
                installed: installed.to_owned(),
                planned: planned.to_owned(),
            })
        }
        _ => Ok(()),
    }
}

/// Stops an `apply` that moves the cluster's Kubernetes further than the skew policy allows.
///
/// `running` is what the cluster's API server reports, `planned` what this node would run. A
/// server, or a single `KubeSolo` node, moves at most one minor version; a worker never runs a
/// newer minor than the API server.
///
/// # Errors
///
/// [`PlanError::SkipsMinor`] or [`PlanError::WorkerAhead`].
pub fn check_kubernetes(
    running: Option<Kubernetes>,
    planned: Kubernetes,
    role: Role,
    distribution: Distribution,
) -> Result<(), PlanError> {
    let Some(running) = running else {
        return Ok(());
    };
    match role {
        Role::Server if planned.minor > running.minor + 1 => Err(PlanError::SkipsMinor {
            running: running.to_string(),
            planned: format!("1.{}", planned.minor),
            next: format!("1.{}", running.minor + 1),
            how: match distribution {
                Distribution::KubeSolo => format!(
                    "apply a zeatend release that installs Kubernetes 1.{} first",
                    running.minor + 1
                ),
                Distribution::K3s | Distribution::Rke2 => format!(
                    "set version to a {} release of Kubernetes 1.{} and apply, then remove it \
                     and apply again",
                    distribution.title(),
                    running.minor + 1
                ),
            },
        }),
        Role::Worker if planned.minor > running.minor => Err(PlanError::WorkerAhead {
            running: running.to_string(),
            planned: format!("1.{}", planned.minor),
        }),
        Role::Server | Role::Worker => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::record::{Writer, ZEATEND_VERSION};

    fn k8s(text: &str) -> Kubernetes {
        Kubernetes::parse(text).unwrap()
    }

    fn record(distribution: &str, version: &str) -> Record {
        Record {
            writer: Writer::Zeatend(ZEATEND_VERSION.to_owned()),
            distribution: Some(distribution.to_owned()),
            version: Some(version.to_owned()),
        }
    }

    #[test]
    fn kubernetes_versions_are_read_from_tags_and_minors() {
        assert_eq!(
            k8s("v1.37.2+k3s1"),
            Kubernetes {
                minor: 37,
                patch: Some(2)
            }
        );
        assert_eq!(k8s("1.36").patch, None);
        assert_eq!(k8s("v1.36.6+rke2r1").minor, 36);
        assert_eq!(k8s("v1.35.1-k3s1").minor, 35);
        for bad in ["", "2.0", "v1", "v1.x", "1.37.2.1", "k3s"] {
            assert_eq!(Kubernetes::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn an_older_release_is_refused_and_a_newer_or_equal_one_applies() {
        let k3s = record("k3s", "v1.37.2+k3s1");
        let ok = |planned| check_release(Some(&k3s), Distribution::K3s, planned).is_ok();
        assert!(ok("v1.37.2+k3s1"));
        assert!(ok("v1.37.2+k3s2"));
        assert!(ok("v1.38.0+k3s1"));
        assert!(!ok("v1.37.2+k3s0"));
        assert!(!ok("v1.37.1+k3s1"));
        assert!(!ok("v1.36.9+k3s1"));
        let error = check_release(Some(&k3s), Distribution::K3s, "v1.36.6+k3s1")
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("K3s v1.37.2+k3s1") && error.contains("v1.36.6+k3s1"),
            "{error}"
        );

        let solo = record("kubesolo", "v1.3.0");
        assert!(check_release(Some(&solo), Distribution::KubeSolo, "v1.2.9").is_err());
        assert!(check_release(Some(&solo), Distribution::KubeSolo, "v1.3.1").is_ok());
        assert!(check_release(None, Distribution::K3s, "v1.0.0+k3s1").is_ok());
        assert!(
            check_release(Some(&solo), Distribution::K3s, "v1.0.0+k3s1").is_ok(),
            "another distribution's record is not compared"
        );
    }

    #[test]
    fn a_server_moves_one_minor_at_a_time() {
        let check = |running, planned| {
            check_kubernetes(
                Some(k8s(running)),
                k8s(planned),
                Role::Server,
                Distribution::K3s,
            )
        };
        assert!(check("v1.36.6+k3s1", "1.36").is_ok());
        assert!(check("v1.36.6+k3s1", "1.37").is_ok());
        assert!(
            check("v1.37.2+k3s1", "1.36").is_ok(),
            "check_release refuses that"
        );
        let error = check("v1.35.4+k3s1", "1.37").unwrap_err().to_string();
        assert!(
            error.contains("Kubernetes 1.35.4")
                && error.contains("set version to a K3s release")
                && error.contains("1.36"),
            "{error}"
        );
        assert!(
            check_kubernetes(None, k8s("1.40"), Role::Server, Distribution::K3s).is_ok(),
            "a cluster that does not answer is not checked"
        );
    }

    #[test]
    fn a_worker_waits_for_its_servers() {
        let check = |running, planned| {
            check_kubernetes(
                Some(k8s(running)),
                k8s(planned),
                Role::Worker,
                Distribution::Rke2,
            )
        };
        assert!(check("v1.37.2+rke2r1", "1.37").is_ok());
        assert!(check("v1.37.2+rke2r1", "1.36").is_ok());
        let error = check("v1.36.6+rke2r1", "1.37").unwrap_err().to_string();
        assert!(error.contains("servers first"), "{error}");
    }

    #[test]
    fn the_release_is_read_from_version_output() {
        assert_eq!(
            release_tag("k3s version v1.36.5+k3s1 (4a6d6d0b)\ngo version go1.25.6\n").as_deref(),
            Some("v1.36.5+k3s1")
        );
        assert_eq!(
            release_tag("rke2 version v1.36.5+rke2r1 (abc)\n").as_deref(),
            Some("v1.36.5+rke2r1")
        );
        assert_eq!(
            release_tag(
                &json!({"level": "info", "version": "v1.2.3", "message": "kubesolo version"})
                    .to_string()
            )
            .as_deref(),
            Some("v1.2.3")
        );
        assert_eq!(release_tag("no version here"), None);
    }
}
