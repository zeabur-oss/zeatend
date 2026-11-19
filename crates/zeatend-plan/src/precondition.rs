//! What a plan found on the host and what applying it leaves there.
//!
//! `apply --plan` makes the plan again and goes on only when it writes the same and the host is
//! as the reviewed plan found or left it, as a JSON Patch `test` operation stops a patch.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::record::{Record, Writer};

/// A value on the host: what the plan found, and what applying the plan leaves.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Change<T> {
    /// When the plan was made.
    pub found: T,
    /// After `apply`.
    pub leaves: T,
}

impl<T: PartialEq> Change<T> {
    /// Whether `now` is what the plan found, or what an `apply` of it that stopped or finished
    /// left.
    fn holds(&self, now: &T) -> bool {
        *now == self.found || *now == self.leaves
    }
}

/// An install record, as a plan shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordState {
    /// The tool that wrote it, such as `zeatend 0.1.0`.
    pub written_by: String,
    /// The distribution's id.
    pub distribution: Option<String>,
    /// The release. It is empty for an installer record: the host reads that release from the
    /// installed binary, and the release precondition already covers it.
    pub version: Option<String>,
}

impl From<&Record> for RecordState {
    fn from(record: &Record) -> Self {
        let own = matches!(record.writer, Writer::Zeatend(_));
        Self {
            written_by: record.writer.to_string(),
            distribution: record.distribution.clone(),
            version: record.version.clone().filter(|_| own),
        }
    }
}

/// What a plan assumes about the host and what it writes there.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Preconditions {
    /// The SHA-256 of each file the plan writes, by path; `None` is a missing file.
    pub files: BTreeMap<String, Change<Option<String>>>,
    /// The release tag the installed binary reports; `None` when there is none.
    pub release: Change<Option<String>>,
    /// The install record.
    pub record: Change<Option<RecordState>>,
    /// The SHA-256 of each file `apply` makes from what the cluster made, such as `KubeSolo`'s
    /// kubeconfig, by path. `leaves` is what `apply` writes from the cluster as it is now; `None`
    /// before the cluster made it.
    pub generated: BTreeMap<String, Change<Option<String>>>,
}

/// Why a reviewed plan no longer applies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stale {
    /// The plan made now writes other files, other content, another release or another record:
    /// the configuration, the host or zeatend changed.
    Writes,
    /// This file changed on the host.
    File(String),
    /// Another release is installed.
    Release {
        /// The release the reviewed plan found.
        reviewed: Option<String>,
        /// The release installed now.
        now: Option<String>,
    },
    /// The install record changed.
    Record,
}

impl Preconditions {
    /// Checks the preconditions of a plan made now against these, the reviewed plan's.
    ///
    /// # Errors
    ///
    /// The first [`Stale`] reason found.
    pub fn check(&self, now: &Self) -> Result<(), Stale> {
        let leaves = |p: &Self| {
            p.files
                .iter()
                .map(|(path, file)| (path.clone(), file.leaves.clone()))
                .collect::<Vec<_>>()
        };
        if leaves(self) != leaves(now)
            || !self.generated.keys().eq(now.generated.keys())
            || self.release.leaves != now.release.leaves
            || self.record.leaves != now.record.leaves
        {
            return Err(Stale::Writes);
        }
        let changed = self.files.iter().find(|(path, file)| {
            now.files
                .get(*path)
                .is_none_or(|found| !file.holds(&found.found))
        });
        if let Some((path, _)) = changed {
            return Err(Stale::File(path.clone()));
        }
        // What the cluster makes, such as a client certificate, can change between the plan and
        // `apply`. A generated file holds when it is as the plan found it or as `apply` writes it
        // now.
        let edited = self.generated.iter().find(|(path, file)| {
            now.generated.get(*path).is_none_or(|found| {
                found.found != file.found && (found.leaves.is_none() || found.found != found.leaves)
            })
        });
        if let Some((path, _)) = edited {
            return Err(Stale::File(path.clone()));
        }
        if !self.release.holds(&now.release.found) {
            return Err(Stale::Release {
                reviewed: self.release.found.clone(),
                now: now.release.found.clone(),
            });
        }
        if !self.record.holds(&now.record.found) {
            return Err(Stale::Record);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reviewed() -> Preconditions {
        Preconditions {
            files: BTreeMap::from([
                (
                    "/etc/rancher/k3s/config.yaml".to_owned(),
                    Change {
                        found: Some("old".to_owned()),
                        leaves: Some("new".to_owned()),
                    },
                ),
                (
                    "/etc/rancher/k3s/admission.yaml".to_owned(),
                    Change {
                        found: None,
                        leaves: Some("admission".to_owned()),
                    },
                ),
            ]),
            release: Change {
                found: Some("v1.36.5+k3s1".to_owned()),
                leaves: Some("v1.37.2+k3s1".to_owned()),
            },
            record: Change {
                found: None,
                leaves: Some(RecordState {
                    written_by: "zeatend 0.1.0".to_owned(),
                    distribution: Some("k3s".to_owned()),
                    version: Some("v1.37.2+k3s1".to_owned()),
                }),
            },
            generated: BTreeMap::from([(
                KUBECONFIG.to_owned(),
                Change {
                    found: None,
                    leaves: None,
                },
            )]),
        }
    }

    const KUBECONFIG: &str = "/etc/zeatend/kubeconfig.yaml";

    /// `reviewed` as a plan made after its `apply` finished finds the host.
    fn applied(reviewed: &Preconditions) -> Preconditions {
        let mut now = reviewed.clone();
        for file in now.files.values_mut() {
            file.found.clone_from(&file.leaves);
        }
        now.release.found.clone_from(&now.release.leaves);
        now.record.found.clone_from(&now.record.leaves);
        for file in now.generated.values_mut() {
            file.found = Some("kubeconfig".to_owned());
            file.leaves = Some("kubeconfig".to_owned());
        }
        now
    }

    #[test]
    fn an_installer_record_does_not_carry_the_release_of_the_binary() {
        let record = |version: &str| Record {
            writer: Writer::Installer(Some("0.9.0".to_owned())),
            distribution: Some("k3s".to_owned()),
            version: Some(version.to_owned()),
        };
        assert_eq!(
            RecordState::from(&record("v1.36.5+k3s1")),
            RecordState::from(&record("v1.37.2+k3s1"))
        );
    }

    #[test]
    fn an_unchanged_host_or_one_the_plan_left_applies() {
        let reviewed = reviewed();
        assert_eq!(reviewed.check(&reviewed), Ok(()));
        assert_eq!(reviewed.check(&applied(&reviewed)), Ok(()));
    }

    #[test]
    fn a_file_changed_by_hand_stops_apply() {
        let reviewed = reviewed();
        let mut now = reviewed.clone();
        let path = "/etc/rancher/k3s/admission.yaml";
        if let Some(file) = now.files.get_mut(path) {
            file.found = Some("edited".to_owned());
        }
        assert_eq!(reviewed.check(&now), Err(Stale::File(path.to_owned())));
    }

    #[test]
    fn a_generated_file_changed_by_hand_stops_apply() {
        // On a new host the cluster has not made it yet, so a plan cannot say what `apply` writes.
        for reviewed in [reviewed(), applied(&reviewed())] {
            let mut now = reviewed.clone();
            if let Some(file) = now.generated.get_mut(KUBECONFIG) {
                file.found = Some("edited".to_owned());
            }
            assert_eq!(
                reviewed.check(&now),
                Err(Stale::File(KUBECONFIG.to_owned()))
            );
        }
    }

    #[test]
    fn a_generated_file_apply_rewrote_from_a_new_cluster_applies() {
        let reviewed = applied(&reviewed());
        let mut now = reviewed.clone();
        if let Some(file) = now.generated.get_mut(KUBECONFIG) {
            file.found = Some("renewed".to_owned());
            file.leaves = Some("renewed".to_owned());
        }
        assert_eq!(reviewed.check(&now), Ok(()));
    }

    #[test]
    fn another_installed_release_stops_apply() {
        let reviewed = reviewed();
        let mut now = reviewed.clone();
        now.release.found = Some("v1.36.6+k3s1".to_owned());
        assert_eq!(
            reviewed.check(&now),
            Err(Stale::Release {
                reviewed: Some("v1.36.5+k3s1".to_owned()),
                now: Some("v1.36.6+k3s1".to_owned()),
            })
        );
    }

    #[test]
    fn another_record_stops_apply() {
        let reviewed = reviewed();
        let mut now = reviewed.clone();
        now.record.found = Some(RecordState {
            written_by: "zeabur-installer".to_owned(),
            distribution: Some("k3s".to_owned()),
            version: None,
        });
        assert_eq!(reviewed.check(&now), Err(Stale::Record));
    }

    #[test]
    fn other_writes_stop_apply() {
        let reviewed = reviewed();
        let mut content = reviewed.clone();
        if let Some(file) = content.files.get_mut("/etc/rancher/k3s/config.yaml") {
            file.leaves = Some("other".to_owned());
        }
        let mut fewer = reviewed.clone();
        fewer.files.remove("/etc/rancher/k3s/admission.yaml");
        let mut release = reviewed.clone();
        release.release.leaves = Some("v1.37.3+k3s1".to_owned());
        for now in [content, fewer, release] {
            assert_eq!(reviewed.check(&now), Err(Stale::Writes));
        }
        assert_eq!(
            Preconditions::default().check(&reviewed),
            Err(Stale::Writes)
        );
    }

    #[test]
    fn preconditions_round_trip_through_json() {
        let reviewed = reviewed();
        let json = serde_json::to_string_pretty(&reviewed).unwrap();
        let read: Preconditions = serde_json::from_str(&json).unwrap();
        assert_eq!(read, reviewed);
    }
}
