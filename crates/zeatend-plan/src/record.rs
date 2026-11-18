//! The install record that `apply` and `join` keep on the host, and what the next `apply` changes
//! against it.

use std::cmp::Ordering;

use serde::Serialize;
use serde_json::Value;

use crate::PlanError;
use crate::cluster::Distribution;

/// This zeatend's version.
pub const ZEATEND_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The file in zeatend's state directory that holds the [`Record`] as JSON.
pub const RECORD_FILE: &str = "record.json";

/// What an earlier `apply` or `join` recorded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Record {
    /// The version of the zeatend that wrote it.
    pub zeatend_version: String,
    /// The distribution's id, such as `k3s`.
    pub distribution: Option<String>,
    /// The K3s, RKE2 or `KubeSolo` release.
    pub version: Option<String>,
}

impl Record {
    /// The record of this zeatend installing `version` of `distribution`.
    #[must_use]
    pub fn new(distribution: Distribution, version: &str) -> Self {
        Self {
            zeatend_version: ZEATEND_VERSION.to_owned(),
            distribution: Some(distribution.id().to_owned()),
            version: Some(version.to_owned()),
        }
    }

    /// Reads the record from the JSON of [`RECORD_FILE`]. `None` when it names no zeatend
    /// version.
    #[must_use]
    pub fn from_data(data: &Value) -> Option<Self> {
        let text = |key: &str| data.get(key).and_then(Value::as_str).map(str::to_owned);
        Some(Self {
            zeatend_version: text("zeatendVersion")?,
            distribution: text("distribution"),
            version: text("version"),
        })
    }

    /// Stops when a newer zeatend wrote the record.
    ///
    /// # Errors
    ///
    /// [`PlanError::NewerZeatend`].
    pub fn check_zeatend_version(&self) -> Result<(), PlanError> {
        check_zeatend_version(&self.zeatend_version)
    }
}

/// Stops when `recorded` is newer than this zeatend.
///
/// # Errors
///
/// [`PlanError::NewerZeatend`].
pub fn check_zeatend_version(recorded: &str) -> Result<(), PlanError> {
    if precedence(recorded, ZEATEND_VERSION) == Some(Ordering::Greater) {
        return Err(PlanError::NewerZeatend {
            recorded: recorded.to_owned(),
            own: ZEATEND_VERSION.to_owned(),
        });
    }
    Ok(())
}

/// What `apply` changes against the record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Changes {
    /// The version of the zeatend that wrote the record.
    pub zeatend_version: String,
    /// The K3s, RKE2 or `KubeSolo` release, when it changes.
    pub version: Option<VersionChange>,
    /// Host files whose content changes; `apply` fills it.
    pub host_files: Vec<String>,
    /// `apply` restarts K3s, RKE2 or `KubeSolo`.
    pub restart: bool,
}

/// A release that changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VersionChange {
    /// The recorded release.
    pub from: String,
    /// The planned release.
    pub to: String,
}

impl Changes {
    /// The changes a plan that installs `version` makes against `record`.
    #[must_use]
    pub fn new(record: &Record, version: &str) -> Self {
        let version = record
            .version
            .as_ref()
            .filter(|from| *from != version)
            .map(|from| VersionChange {
                from: from.clone(),
                to: version.to_owned(),
            });
        Self {
            zeatend_version: record.zeatend_version.clone(),
            restart: version.is_some(),
            version,
            host_files: Vec::new(),
        }
    }

    /// Records the host files whose content changes; a change restarts the service.
    pub fn set_host_files(&mut self, paths: Vec<String>) {
        self.restart |= !paths.is_empty();
        self.host_files = paths;
    }
}

/// The precedence of two versions as Semantic Versioning 2.0.0 orders them
/// (<https://semver.org/#spec-item-11>); `None` when either is not one.
#[must_use]
fn precedence(a: &str, b: &str) -> Option<Ordering> {
    let a = semver::Version::parse(a).ok()?;
    let b = semver::Version::parse(b).ok()?;
    Some(a.cmp_precedence(&b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_order_as_semver_says() {
        let ordered = [
            "0.1.1",
            "0.1.2-alpha",
            "0.1.2-alpha.1",
            "0.1.2-alpha.beta",
            "0.1.2-beta.2",
            "0.1.2-beta.11",
            "0.1.2-rc.1",
            "0.1.2",
            "0.2.0",
            "1.0.0",
        ];
        for pair in ordered.windows(2) {
            let [a, b] = pair else { unreachable!() };
            assert_eq!(precedence(a, b), Some(Ordering::Less), "{a} < {b}");
            assert_eq!(precedence(b, a), Some(Ordering::Greater), "{b} > {a}");
        }
        assert_eq!(precedence("0.1.2+build.5", "0.1.2"), Some(Ordering::Equal));
        assert_eq!(precedence("", "0.1.2"), None);
    }

    #[test]
    fn a_record_from_a_newer_zeatend_names_the_version_to_use() {
        let record = Record {
            zeatend_version: "99.0.0".to_owned(),
            distribution: None,
            version: None,
        };
        let error = record.check_zeatend_version().unwrap_err().to_string();
        assert!(error.contains("zeatend 99.0.0 or newer"), "{error}");
        let own = Record {
            zeatend_version: ZEATEND_VERSION.to_owned(),
            ..record
        };
        own.check_zeatend_version().unwrap();
    }

    #[test]
    fn a_new_release_or_changed_file_restarts() {
        let record = Record {
            zeatend_version: ZEATEND_VERSION.to_owned(),
            distribution: Some("k3s".to_owned()),
            version: Some("v1.0.0+k3s1".to_owned()),
        };
        let same = Changes::new(&record, "v1.0.0+k3s1");
        assert!(!same.restart);
        let mut files = same;
        files.set_host_files(vec!["/etc/rancher/k3s/config.yaml".to_owned()]);
        assert!(files.restart);
        let newer = Changes::new(&record, "v1.1.0+k3s1");
        assert!(newer.restart);
        assert_eq!(
            newer.version,
            Some(VersionChange {
                from: "v1.0.0+k3s1".to_owned(),
                to: "v1.1.0+k3s1".to_owned()
            })
        );
        assert_eq!(Record::from_data(&serde_json::json!({})), None);
    }

    #[test]
    fn a_record_reads_back_what_it_wrote() {
        let record = Record::new(Distribution::Rke2, "v1.0.0+rke2r1");
        let json = serde_json::to_value(&record).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "zeatendVersion": ZEATEND_VERSION,
                "distribution": "rke2",
                "version": "v1.0.0+rke2r1",
            })
        );
        assert_eq!(Record::from_data(&json), Some(record));
    }
}
