//! The API server's authentication configuration: the `apiServer.authenticationConfig` option,
//! written as the file that kube-apiserver's `--authentication-config` names.
//!
//! The option is an `AuthenticationConfiguration` of `apiserver.config.k8s.io/v1`, stable since
//! Kubernetes 1.34, as the Kubernetes documentation describes it. zeatend writes it unchanged and
//! leaves its validation to kube-apiserver. kube-apiserver reloads the file when it changes; only
//! the flag that names it needs a restart
//! (<https://kubernetes.io/docs/reference/access-authn-authz/authentication/#using-authentication-configuration>).
//!
//! What differs between the distributions is here:
//!
//! - K3s passes `--anonymous-auth=false` to kube-apiserver, except when an `authentication-config`
//!   argument is given (`pkg/daemons/control/server.go`, K3s v1.37.2+k3s1); RKE2 starts
//!   kube-apiserver through the same code. When the configuration has no `anonymous` section, the
//!   file gets `anonymous: {enabled: false}`, so anonymous requests stay off. `KubeSolo` does not
//!   pass the flag, so its file is written unchanged.
//! - RKE2 runs kube-apiserver as a static pod that mounts only its own directories
//!   (`pkg/executor/staticpod/staticpod.go`, RKE2 v1.37.2+rke2r1), so the file's directory is
//!   mounted with `kube-apiserver-extra-mount`. A directory, not the file: zeatend replaces the
//!   file by renaming a new one over it, which a bind mount of the old file would not show.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::PlanError;
use crate::cluster::HostFile;

/// The name of the file in its directory.
const FILE_NAME: &str = "config.yaml";

const API_VERSION: &str = "apiserver.config.k8s.io/v1";
const KIND: &str = "AuthenticationConfiguration";

/// The `apiServer` section of the configuration.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApiServer {
    /// The API server's authentication configuration. Absent: zeatend writes no file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authentication_config: Option<AuthenticationConfig>,
}

/// An `AuthenticationConfiguration`. zeatend reads only the fields it needs and keeps the others
/// as they are written.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthenticationConfig {
    /// `apiserver.config.k8s.io/v1`.
    pub api_version: String,
    /// `AuthenticationConfiguration`.
    pub kind: String,
    /// The `anonymous` section, as written.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anonymous: Option<serde_json::Value>,
    /// Every other field, such as `jwt`, as written.
    #[serde(flatten)]
    pub rest: BTreeMap<String, serde_json::Value>,
}

impl ApiServer {
    /// Whether the section sets nothing.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.authentication_config.is_none()
    }

    /// The section as the YAML lines of a configuration file, `apiServer:` first; none when it
    /// sets nothing.
    ///
    /// # Errors
    ///
    /// When it cannot be serialised.
    pub fn yaml_lines(&self) -> Result<Vec<String>, PlanError> {
        if self.is_empty() {
            return Ok(Vec::new());
        }
        let yaml = serde_saphyr::to_string(self).map_err(|e| PlanError::Config(e.to_string()))?;
        Ok(std::iter::once("apiServer:".to_owned())
            .chain(yaml.lines().map(|line| format!("  {line}")))
            .collect())
    }

    /// Checks that the configuration is an `AuthenticationConfiguration`. kube-apiserver checks
    /// the rest.
    ///
    /// # Errors
    ///
    /// When it has another `apiVersion` or `kind`.
    pub fn check(&self) -> Result<(), PlanError> {
        let Some(config) = &self.authentication_config else {
            return Ok(());
        };
        if config.api_version != API_VERSION || config.kind != KIND {
            return Err(PlanError::Config(format!(
                "apiServer.authenticationConfig: set apiVersion: {API_VERSION} and kind: {KIND}"
            )));
        }
        Ok(())
    }
}

/// The file that kube-apiserver's `--authentication-config` names, in `dir`, or `None` without a
/// configuration. `anonymous_off` turns anonymous requests off when the configuration does not
/// say.
///
/// # Errors
///
/// When the file cannot be serialised.
pub fn file(
    dir: &str,
    api_server: &ApiServer,
    anonymous_off: bool,
) -> Result<Option<HostFile>, PlanError> {
    let Some(config) = &api_server.authentication_config else {
        return Ok(None);
    };
    let mut config = config.clone();
    if anonymous_off && config.anonymous.is_none() {
        config.anonymous = Some(serde_json::json!({"enabled": false}));
    }
    HostFile::yaml(
        path(dir),
        "# Written by zeatend; `zeatend apply` writes it again.\n",
        &config,
    )
    .map(Some)
}

/// The path of the file in `dir`.
#[must_use]
pub fn path(dir: &str) -> String {
    format!("{dir}/{FILE_NAME}")
}

/// The configuration in a file that [`file()`] wrote, so a new server gets the same one.
///
/// # Errors
///
/// When the text is not such a file.
pub fn read(text: &str) -> Result<ApiServer, PlanError> {
    let config = serde_saphyr::from_str(text)
        .map_err(|e| PlanError::Config(format!("the authentication file: {e}")))?;
    Ok(ApiServer {
        authentication_config: Some(config),
    })
}

/// A configuration with one issuer, for tests.
#[cfg(test)]
#[must_use]
pub(crate) fn example() -> ApiServer {
    tests::api_server(tests::ZEABUR).unwrap()
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;

    pub(super) fn api_server(yaml: &str) -> Result<ApiServer, PlanError> {
        let api_server: ApiServer =
            serde_saphyr::from_str(yaml).map_err(|e| PlanError::Config(e.to_string()))?;
        api_server.check()?;
        Ok(api_server)
    }

    pub(super) const ZEABUR: &str = "authenticationConfig:
  apiVersion: apiserver.config.k8s.io/v1
  kind: AuthenticationConfiguration
  jwt:
    - issuer:
        url: https://identity.example.com
        audiences: [kubernetes]
      claimMappings:
        username:
          claim: sub
          prefix: \"zeabur:\"
";

    #[test]
    fn the_configuration_is_written_as_it_is_with_anonymous_requests_off() {
        let file = file("/etc/x", &api_server(ZEABUR).unwrap(), true)
            .unwrap()
            .unwrap();
        assert_eq!(file.path, "/etc/x/config.yaml");
        assert_eq!(file.mode, 0o600);
        let config: Value = serde_saphyr::from_str(&file.content).unwrap();
        assert_eq!(
            config,
            json!({
                "apiVersion": "apiserver.config.k8s.io/v1",
                "kind": "AuthenticationConfiguration",
                "anonymous": {"enabled": false},
                "jwt": [{
                    "issuer": {
                        "url": "https://identity.example.com",
                        "audiences": ["kubernetes"],
                    },
                    "claimMappings": {"username": {"claim": "sub", "prefix": "zeabur:"}},
                }],
            })
        );
    }

    #[test]
    fn an_anonymous_section_is_kept() {
        let with = format!(
            "{ZEABUR}  anonymous:\n    enabled: true\n    conditions:\n      - path: /livez\n"
        );
        let api_server = api_server(&with).unwrap();
        let file = file("/etc/x", &api_server, true).unwrap().unwrap();
        let config: Value = serde_saphyr::from_str(&file.content).unwrap();
        assert_eq!(
            config["anonymous"],
            json!({"enabled": true, "conditions": [{"path": "/livez"}]})
        );
        assert_eq!(read(&file.content).unwrap(), api_server);
    }

    #[test]
    fn without_anonymous_off_the_file_is_unchanged() {
        let api_server = api_server(ZEABUR).unwrap();
        let file = file("/etc/x", &api_server, false).unwrap().unwrap();
        assert_eq!(read(&file.content).unwrap(), api_server);
    }

    #[test]
    fn no_configuration_writes_no_file() {
        assert_eq!(file("/etc/x", &ApiServer::default(), true).unwrap(), None);
    }

    #[test]
    fn another_kind_is_refused() {
        for bad in [
            ZEABUR.replace("config.k8s.io/v1", "config.k8s.io/v1beta1"),
            ZEABUR.replace("kind: AuthenticationConfiguration", "kind: Config"),
        ] {
            let err = api_server(&bad).unwrap_err().to_string();
            assert!(err.contains("apiVersion"), "{bad}: {err}");
        }
    }
}
