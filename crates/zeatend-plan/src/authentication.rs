//! The JWT issuers the API server trusts: the `apiServer.jwt` option, written as the file that
//! kube-apiserver's `--authentication-config` names.
//!
//! The file is an `AuthenticationConfiguration` of `apiserver.config.k8s.io/v1`, stable since
//! Kubernetes 1.34. kube-apiserver reloads it when it changes; only the flag that names it needs
//! a restart
//! (<https://kubernetes.io/docs/reference/access-authn-authz/authentication/#using-authentication-configuration>).
//!
//! - Every issuer gets the user validation rules of that page's example, which refuse a username or
//!   group that starts with `system:`. A prefix of `""` cannot then make a token a member of
//!   `system:masters`.
//! - K3s passes `--anonymous-auth=false` to kube-apiserver, except when an `authentication-config`
//!   argument is given (`pkg/daemons/control/server.go`, K3s v1.37.2+k3s1); RKE2 starts
//!   kube-apiserver through the same code. The file turns anonymous requests off again with its
//!   `anonymous` section, which cannot be combined with the flag. `KubeSolo` does not pass the
//!   flag, so its file has no `anonymous` section and anonymous requests stay as they were.
//! - RKE2 runs kube-apiserver as a static pod that mounts only its own directories
//!   (`pkg/executor/staticpod/staticpod.go`, RKE2 v1.37.2+rke2r1), so the file's directory is
//!   mounted with `kube-apiserver-extra-mount`. A directory, not the file: zeatend replaces the
//!   file by renaming a new one over it, which a bind mount of the old file would not show.

use serde::{Deserialize, Serialize};

use crate::PlanError;
use crate::cluster::HostFile;

/// The name of the file in its directory.
const FILE_NAME: &str = "config.yaml";

/// Kubernetes takes at most this many JWT issuers.
const MAX_ISSUERS: usize = 64;

/// The `apiServer` section of the configuration.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApiServer {
    /// The JWT issuers whose tokens the API server accepts. Empty: zeatend writes no
    /// authentication file.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub jwt: Vec<JwtIssuer>,
}

impl ApiServer {
    /// Whether the section sets nothing.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.jwt.is_empty()
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

    /// Checks the values serde cannot.
    ///
    /// # Errors
    ///
    /// The first invalid value.
    pub fn check(&self) -> Result<(), PlanError> {
        if self.jwt.len() > MAX_ISSUERS {
            return Err(PlanError::Config(format!(
                "apiServer.jwt: Kubernetes takes at most {MAX_ISSUERS} issuers"
            )));
        }
        let mut seen = std::collections::BTreeSet::new();
        for jwt in &self.jwt {
            jwt.check()?;
            if !seen.insert(jwt.issuer.as_str()) {
                return Err(PlanError::Config(format!(
                    "apiServer.jwt: {} is listed twice; each issuer appears once",
                    jwt.issuer
                )));
            }
        }
        Ok(())
    }
}

/// A JWT issuer the API server trusts.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JwtIssuer {
    /// The issuer's URL, `https://`; the `iss` of its tokens. kube-apiserver reads the signing
    /// keys from `<issuer>/.well-known/openid-configuration`.
    pub issuer: String,
    /// The PEM certificates that the issuer's HTTPS certificate must chain to. Absent: the CAs
    /// that kube-apiserver trusts by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub certificate_authority: Option<String>,
    /// The `aud` values the API server accepts; a token must name one of them.
    pub audiences: Vec<String>,
    /// The claim that holds the username. Absent: `sub`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username_claim: Option<String>,
    /// Put before each username, such as `zeabur:`, so this issuer's users cannot take the names
    /// of others. `""` for none.
    pub username_prefix: String,
    /// The claim that holds the user's groups. Absent: the user is in no groups of this issuer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub groups_claim: Option<String>,
    /// Put before each group. Required with `groupsClaim`; `""` for none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub groups_prefix: Option<String>,
}

impl JwtIssuer {
    fn check(&self) -> Result<(), PlanError> {
        let bad = |what: &str| {
            Err(PlanError::Config(format!(
                "apiServer.jwt: {}: {what}",
                self.issuer
            )))
        };
        let host = self.issuer.strip_prefix("https://").unwrap_or_default();
        if host.is_empty() || host.starts_with('/') {
            return bad("the issuer is not an https:// URL");
        }
        if self
            .certificate_authority
            .as_ref()
            .is_some_and(|pem| !pem.contains("-----BEGIN CERTIFICATE-----"))
        {
            return bad("certificateAuthority holds no PEM certificate");
        }
        if self.audiences.iter().all(String::is_empty) {
            return bad("set audiences, the aud values its tokens carry");
        }
        if self.username_claim.as_deref() == Some("") {
            return bad("usernameClaim is empty; remove it to use sub");
        }
        match (&self.groups_claim, &self.groups_prefix) {
            (Some(claim), _) if claim.is_empty() => bad("groupsClaim is empty"),
            (Some(_), None) => bad("set groupsPrefix with groupsClaim; \"\" for none"),
            (None, Some(_)) => bad("groupsPrefix applies only with groupsClaim"),
            _ => Ok(()),
        }
    }
}

/// The file that kube-apiserver's `--authentication-config` names, in `dir`, or `None` without
/// issuers. `anonymous_off` turns anonymous requests off in the file.
///
/// # Errors
///
/// When the file cannot be serialised.
pub fn file(
    dir: &str,
    api_server: &ApiServer,
    anonymous_off: bool,
) -> Result<Option<HostFile>, PlanError> {
    if api_server.is_empty() {
        return Ok(None);
    }
    let reserved = |expression: &str| Rule {
        expression: expression.to_owned(),
        message: "the system: prefix is reserved".to_owned(),
    };
    let configuration = AuthenticationConfiguration {
        api_version: "apiserver.config.k8s.io/v1".to_owned(),
        kind: "AuthenticationConfiguration".to_owned(),
        anonymous: anonymous_off.then_some(Anonymous { enabled: false }),
        jwt: api_server
            .jwt
            .iter()
            .map(|jwt| Jwt {
                issuer: Issuer {
                    url: jwt.issuer.clone(),
                    certificate_authority: jwt.certificate_authority.clone(),
                    audiences: jwt.audiences.clone(),
                    // Required with more than one audience.
                    audience_match_policy: (jwt.audiences.len() > 1).then(|| "MatchAny".to_owned()),
                },
                claim_mappings: ClaimMappings {
                    username: PrefixedClaim {
                        claim: jwt
                            .username_claim
                            .clone()
                            .unwrap_or_else(|| "sub".to_owned()),
                        prefix: jwt.username_prefix.clone(),
                    },
                    groups: jwt.groups_claim.clone().map(|claim| PrefixedClaim {
                        claim,
                        prefix: jwt.groups_prefix.clone().unwrap_or_default(),
                    }),
                },
                user_validation_rules: vec![
                    reserved("!user.username.startsWith('system:')"),
                    reserved("user.groups.all(group, !group.startsWith('system:'))"),
                ],
            })
            .collect(),
    };
    HostFile::yaml(
        path(dir),
        "# Written by zeatend; `zeatend apply` writes it again.\n",
        &configuration,
    )
    .map(Some)
}

/// The path of the file in `dir`.
#[must_use]
pub fn path(dir: &str) -> String {
    format!("{dir}/{FILE_NAME}")
}

/// The issuers of a file that [`file()`] wrote, so a new server trusts the same ones.
///
/// # Errors
///
/// When the text is not such a file.
pub fn read(text: &str) -> Result<ApiServer, PlanError> {
    let configuration: AuthenticationConfiguration = serde_saphyr::from_str(text)
        .map_err(|e| PlanError::Config(format!("the authentication file: {e}")))?;
    let jwt = configuration
        .jwt
        .into_iter()
        .map(|jwt| {
            let username = jwt.claim_mappings.username;
            let (groups_claim, groups_prefix) = jwt
                .claim_mappings
                .groups
                .map(|g| (g.claim, g.prefix))
                .unzip();
            JwtIssuer {
                issuer: jwt.issuer.url,
                certificate_authority: jwt.issuer.certificate_authority,
                audiences: jwt.issuer.audiences,
                username_claim: (username.claim != "sub").then_some(username.claim),
                username_prefix: username.prefix,
                groups_claim,
                groups_prefix,
            }
        })
        .collect();
    Ok(ApiServer { jwt })
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuthenticationConfiguration {
    api_version: String,
    kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    anonymous: Option<Anonymous>,
    jwt: Vec<Jwt>,
}

#[derive(Serialize, Deserialize)]
struct Anonymous {
    enabled: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Jwt {
    issuer: Issuer,
    claim_mappings: ClaimMappings,
    user_validation_rules: Vec<Rule>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Issuer {
    url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    certificate_authority: Option<String>,
    audiences: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    audience_match_policy: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct ClaimMappings {
    username: PrefixedClaim,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    groups: Option<PrefixedClaim>,
}

#[derive(Serialize, Deserialize)]
struct PrefixedClaim {
    claim: String,
    prefix: String,
}

#[derive(Serialize, Deserialize)]
struct Rule {
    expression: String,
    message: String,
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;

    fn api_server(yaml: &str) -> Result<ApiServer, PlanError> {
        let api_server: ApiServer =
            serde_saphyr::from_str(yaml).map_err(|e| PlanError::Config(e.to_string()))?;
        api_server.check()?;
        Ok(api_server)
    }

    const ZEABUR: &str = "jwt:\n  - issuer: https://identity.example.com\n    audiences: [kubernetes]\n    usernamePrefix: \"zeabur:\"\n";

    #[test]
    fn an_issuer_becomes_a_jwt_authenticator() {
        let file = file("/etc/x", &api_server(ZEABUR).unwrap(), true)
            .unwrap()
            .unwrap();
        assert_eq!(file.path, "/etc/x/config.yaml");
        assert_eq!(file.mode, 0o600);
        let config: Value = serde_saphyr::from_str(&file.content).unwrap();
        assert_eq!(config["apiVersion"], "apiserver.config.k8s.io/v1");
        assert_eq!(config["anonymous"], json!({"enabled": false}));
        let jwt = &config["jwt"][0];
        assert_eq!(
            jwt["issuer"],
            json!({"url": "https://identity.example.com", "audiences": ["kubernetes"]})
        );
        assert_eq!(
            jwt["claimMappings"],
            json!({"username": {"claim": "sub", "prefix": "zeabur:"}})
        );
        assert_eq!(jwt["userValidationRules"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn groups_and_several_audiences() {
        let api_server = api_server(
            "jwt:\n  - issuer: https://a.example.com\n    audiences: [kubernetes, other]\n    usernameClaim: preferred_username\n    usernamePrefix: \"a:\"\n    groupsClaim: groups\n    groupsPrefix: \"a:\"\n",
        )
        .unwrap();
        let file = file("/etc/x", &api_server, false).unwrap().unwrap();
        let config: Value = serde_saphyr::from_str(&file.content).unwrap();
        assert!(config.get("anonymous").is_none());
        let jwt = &config["jwt"][0];
        assert_eq!(jwt["issuer"]["audienceMatchPolicy"], "MatchAny");
        assert_eq!(
            jwt["claimMappings"],
            json!({
                "username": {"claim": "preferred_username", "prefix": "a:"},
                "groups": {"claim": "groups", "prefix": "a:"},
            })
        );
        assert_eq!(read(&file.content).unwrap(), api_server);
    }

    #[test]
    fn no_issuers_write_no_file() {
        assert_eq!(file("/etc/x", &ApiServer::default(), true).unwrap(), None);
    }

    #[test]
    fn invalid_issuers_are_refused() {
        for (bad, says) in [
            (
                "jwt:\n  - issuer: http://a.example.com\n    audiences: [k]\n    usernamePrefix: \"\"\n",
                "https://",
            ),
            (
                "jwt:\n  - issuer: https://a.example.com\n    audiences: []\n    usernamePrefix: \"\"\n",
                "audiences",
            ),
            (
                "jwt:\n  - issuer: https://a.example.com\n    audiences: [k]\n",
                "usernamePrefix",
            ),
            (
                "jwt:\n  - issuer: https://a.example.com\n    audiences: [k]\n    usernamePrefix: \"\"\n    groupsClaim: groups\n",
                "groupsPrefix",
            ),
            (
                "jwt:\n  - issuer: https://a.example.com\n    audiences: [k]\n    usernamePrefix: \"\"\n  - issuer: https://a.example.com\n    audiences: [j]\n    usernamePrefix: \"\"\n",
                "twice",
            ),
        ] {
            let err = api_server(bad).unwrap_err().to_string();
            assert!(err.contains(says), "{bad}: {err}");
        }
    }
}
