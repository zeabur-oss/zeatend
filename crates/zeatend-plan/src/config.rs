//! zeatend's configuration file.
//!
//! One file holds every choice, so the same install can be planned again, kept in version control
//! and used as a test input. Secret values never appear here: a join token is in a file of mode
//! 0600 that the configuration names, or in the configuration file, which must then be mode 0600.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::PlanError;
use crate::cluster::Distribution;

/// The configuration of the first node of a cluster.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Config {
    /// `k3s`, the default; `rke2` for a control plane of three servers; `kubesolo` for one node
    /// with little memory.
    #[serde(default)]
    pub distribution: Distribution,
    /// An RKE2 release other than the pinned one, such as `v1.37.1+rke2r1`. Its downloads are
    /// checked only against the release's own checksum list, so the plan warns that it is
    /// unsupported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rke2_version: Option<String>,
    /// A K3s release other than the pinned one, such as `v1.37.1+k3s1`, checked the same way.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub k3s_version: Option<String>,
    /// Packaged components that K3s or RKE2 does not deploy. Absent: each deploys its own
    /// defaults.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub disable: Vec<Component>,
    /// Host settings.
    #[serde(default)]
    pub host: Host,
}

/// A packaged component of K3s or RKE2 that the configuration can turn off.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Component {
    /// The packaged ingress controller, Traefik on K3s and on RKE2 v1.36 and later.
    Traefik,
    /// metrics-server.
    MetricsServer,
    /// K3s's local-path provisioner and its default storage class. RKE2 has none.
    LocalStorage,
}

impl Component {
    /// The name in the configuration and in K3s's `disable` list.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Traefik => "traefik",
            Self::MetricsServer => "metrics-server",
            Self::LocalStorage => "local-storage",
        }
    }
}

/// Each component in `disable` must be one that `distribution` packages.
fn check_disable(distribution: Distribution, disable: &[Component]) -> Result<(), PlanError> {
    match distribution {
        Distribution::KubeSolo if !disable.is_empty() => Err(PlanError::Config(
            "disable: KubeSolo packages none of these components; remove disable".to_owned(),
        )),
        Distribution::Rke2 if disable.contains(&Component::LocalStorage) => Err(PlanError::Config(
            "disable: local-storage is K3s's; RKE2 packages no storage class".to_owned(),
        )),
        Distribution::KubeSolo | Distribution::K3s | Distribution::Rke2 => Ok(()),
    }
}

/// Host settings.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Host {
    /// Extra names and addresses for the API server certificate.
    #[serde(default)]
    pub tls_san: Vec<String>,
    /// Stop when the root or data directory is not on an encrypted device.
    #[serde(default)]
    pub require_encrypted_disk: bool,
    /// The node's name in Kubernetes. Absent: the host name in lower case, unless it is made
    /// from the host's address, then `node-<6 hex>`. Once installed, a node keeps its name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_name: Option<String>,
}

impl Host {
    fn check(&self) -> Result<(), PlanError> {
        self.node_name
            .as_deref()
            .map_or(Ok(()), crate::node_name::check)
    }
}

/// A configuration file: a new cluster, or a node joining one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Document {
    /// The first node: `KubeSolo`, K3s or RKE2.
    Install(Config),
    /// Another node: K3s or RKE2, joined to an existing cluster.
    Join(JoinConfig),
}

impl Document {
    /// Parses a configuration file of either kind: a top-level `join` key makes it a join.
    ///
    /// # Errors
    ///
    /// When the YAML does not match or a value is invalid.
    pub fn parse(yaml: &str) -> Result<Self, PlanError> {
        let value: Value =
            serde_saphyr::from_str(yaml).map_err(|e| PlanError::Config(e.to_string()))?;
        if value.get("join").is_some() {
            JoinConfig::parse(yaml).map(Self::Join)
        } else {
            Config::parse(yaml).map(Self::Install)
        }
    }
}

/// A node joining an existing cluster.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JoinConfig {
    /// The cluster to join.
    pub join: Join,
    /// The cluster's distribution, K3s or RKE2.
    #[serde(default)]
    pub distribution: Distribution,
    /// An RKE2 release other than the pinned one; it must match the cluster's.
    pub rke2_version: Option<String>,
    /// A K3s release other than the pinned one; it must match the cluster's.
    pub k3s_version: Option<String>,
    /// Servers only: the packaged components the cluster does not deploy. Give each server the
    /// same list, as each one deploys the packaged components.
    #[serde(default)]
    pub disable: Vec<Component>,
    /// Host settings. `tlsSan` applies to servers only.
    #[serde(default)]
    pub host: Host,
}

/// The `join` section. `Debug` leaves the token out.
#[derive(Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Join {
    /// The server to join through: `https://<address>:6443` on K3s, `https://<address>:9345`
    /// (RKE2's supervisor) on RKE2.
    pub server: String,
    /// `worker` (default) or `server`.
    #[serde(default)]
    pub role: Role,
    /// The join token, `K10<ca-hash>::<credentials>`. The configuration file must then be mode
    /// 0600.
    pub token: Option<String>,
    /// A file (mode 0600) holding the join token.
    pub token_file: Option<String>,
    /// The names of the cluster's nodes; this node takes another.
    #[serde(default)]
    pub node_names: Vec<String>,
}

impl std::fmt::Debug for Join {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Join")
            .field("server", &self.server)
            .field("role", &self.role)
            .field("token", &self.token.as_ref().map(|_| "<redacted>"))
            .field("token_file", &self.token_file)
            .field("node_names", &self.node_names)
            .finish()
    }
}

/// What a joining node runs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Role {
    /// An agent: runs workloads.
    #[default]
    Worker,
    /// An RKE2 server: control plane and etcd, and workloads. K3s takes no second server.
    Server,
}

impl Role {
    /// `worker` or `server`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Worker => "worker",
            Self::Server => "server",
        }
    }
}

impl JoinConfig {
    /// Parses and checks a join configuration.
    ///
    /// # Errors
    ///
    /// When the YAML does not match or a value is invalid.
    pub fn parse(yaml: &str) -> Result<Self, PlanError> {
        let config: Self =
            serde_saphyr::from_str(yaml).map_err(|e| PlanError::Config(e.to_string()))?;
        config.check()?;
        Ok(config)
    }

    /// K3s or RKE2.
    ///
    /// # Errors
    ///
    /// When the distribution is `KubeSolo`, which takes no other node.
    pub fn rancher(&self) -> Result<crate::cluster::Rancher, PlanError> {
        self.distribution.rancher().ok_or_else(|| {
            PlanError::Config(
                "distribution: kubesolo runs one node; a node joins K3s or RKE2 only".to_owned(),
            )
        })
    }

    /// Checks the values serde cannot.
    ///
    /// # Errors
    ///
    /// The first invalid value.
    pub fn check(&self) -> Result<(), PlanError> {
        let join = &self.join;
        let rancher = self.rancher()?;
        check_version(
            self.distribution,
            self.rke2_version.as_ref(),
            self.k3s_version.as_ref(),
        )?;
        if join.role == Role::Server && !rancher.takes_servers() {
            return Err(PlanError::SecondServer);
        }
        server_address(&join.server, rancher.join_port())?;
        match (&join.token, &join.token_file) {
            (Some(_), Some(_)) => {
                return Err(PlanError::Config(
                    "join: set token or tokenFile, not both".to_owned(),
                ));
            }
            (None, None) => {
                return Err(PlanError::Config(
                    "join: set tokenFile (a file, mode 0600, holding the join token) or token"
                        .to_owned(),
                ));
            }
            (Some(token), None) => {
                check_token(token, join.role)?;
            }
            (None, Some(_)) => {}
        }
        if join.role == Role::Worker && !self.host.tls_san.is_empty() {
            return Err(PlanError::Config(
                "host.tlsSan applies to servers; a worker serves no Kubernetes API".to_owned(),
            ));
        }
        if join.role == Role::Worker && !self.disable.is_empty() {
            return Err(PlanError::Config(
                "disable applies to servers; a worker deploys no packaged components".to_owned(),
            ));
        }
        check_disable(self.distribution, &self.disable)?;
        self.host.check()
    }

    /// The release chosen instead of the pinned one, for the distribution.
    #[must_use]
    pub fn version(&self) -> Option<&str> {
        version(
            self.distribution,
            self.rke2_version.as_ref(),
            self.k3s_version.as_ref(),
        )
    }
}

/// The host and port of a join URL, `https://<host>:<port>` with nothing after it; without a
/// port, `default_port`.
///
/// # Errors
///
/// When the URL is not of that form.
pub fn server_address(url: &str, default_port: u16) -> Result<(String, u16), PlanError> {
    let bad = || {
        PlanError::Config(format!(
            "join.server `{url}` is not of the form https://<address>:{default_port}"
        ))
    };
    let rest = url.strip_prefix("https://").ok_or_else(bad)?;
    let rest = rest.strip_suffix('/').unwrap_or(rest);
    if rest.contains('/') || rest.contains('@') || rest.is_empty() {
        return Err(bad());
    }
    let (host, port) = if let Some(v6) = rest.strip_prefix('[') {
        let (host, after) = v6.split_once(']').ok_or_else(bad)?;
        (host, after.strip_prefix(':'))
    } else {
        match rest.rsplit_once(':') {
            Some((host, port)) if !host.contains(':') => (host, Some(port)),
            Some(_) => return Err(bad()),
            None => (rest, None),
        }
    };
    let port = match port {
        Some(p) => p.parse::<u16>().ok().ok_or_else(bad)?,
        None => default_port,
    };
    if host.is_empty() {
        return Err(bad());
    }
    Ok((host.to_owned(), port))
}

/// The release override of another distribution is a mistake: it would be ignored. An override
/// is a release tag, `v1.37.1+k3s1` or `v1.37.1+rke2r1`. `KubeSolo` takes none.
fn check_version(
    distribution: Distribution,
    rke2: Option<&String>,
    k3s: Option<&String>,
) -> Result<(), PlanError> {
    for (name, version, build) in [("rke2Version", rke2, "rke2r"), ("k3sVersion", k3s, "k3s")] {
        if let Some(version) = version
            && !is_release_tag(version, build)
        {
            return Err(PlanError::Config(format!(
                "{name}: {version} is not a release tag such as v1.37.1+{build}1"
            )));
        }
    }
    match (distribution, rke2, k3s) {
        (Distribution::K3s, Some(_), _) => Err(PlanError::Config(
            "rke2Version applies to distribution: rke2; this one is k3s (set k3sVersion)"
                .to_owned(),
        )),
        (Distribution::Rke2, _, Some(_)) => Err(PlanError::Config(
            "k3sVersion applies to distribution: k3s; this one is rke2 (set rke2Version)"
                .to_owned(),
        )),
        (Distribution::KubeSolo, Some(_), _) | (Distribution::KubeSolo, _, Some(_)) => {
            Err(PlanError::Config(
                "distribution: kubesolo installs the pinned KubeSolo release; remove k3sVersion \
                 and rke2Version"
                    .to_owned(),
            ))
        }
        _ => Ok(()),
    }
}

fn version<'a>(
    distribution: Distribution,
    rke2: Option<&'a String>,
    k3s: Option<&'a String>,
) -> Option<&'a str> {
    match distribution {
        Distribution::KubeSolo => None,
        Distribution::K3s => k3s.map(String::as_str),
        Distribution::Rke2 => rke2.map(String::as_str),
    }
}

/// Whether `version` is `v<major>.<minor>.<patch>+<build><n>`, such as `v1.37.1+k3s1`.
fn is_release_tag(version: &str, build: &str) -> bool {
    let Some((release, metadata)) = version.strip_prefix('v').and_then(|v| v.split_once('+'))
    else {
        return false;
    };
    let numbers: Vec<&str> = release.split('.').collect();
    let number = |n: &str| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit());
    numbers.len() == 3
        && numbers.iter().all(|n| number(n))
        && metadata.strip_prefix(build).is_some_and(number)
}

/// A join token: `K10<64 hex>::<credentials>`, or bare credentials (no CA pinning).
///
/// # Errors
///
/// When it is malformed, or a bootstrap token is given to a server.
pub fn check_token(token: &str, role: Role) -> Result<(), PlanError> {
    let token = token.trim();
    let credentials = match token.strip_prefix("K10") {
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
            credentials
        }
        None => token,
    };
    if credentials.is_empty() || credentials.chars().any(char::is_whitespace) {
        return Err(PlanError::Config("join token: no credentials".to_owned()));
    }
    if role == Role::Server && is_bootstrap_token(credentials) {
        return Err(PlanError::BootstrapTokenForServer);
    }
    Ok(())
}

/// Whether a join token pins the cluster CA.
#[must_use]
pub fn token_pins_ca(token: &str) -> bool {
    token
        .trim()
        .strip_prefix("K10")
        .and_then(|rest| rest.split_once("::"))
        .is_some_and(|(hash, _)| !hash.is_empty())
}

/// `[a-z0-9]{6}.[a-z0-9]{16}`.
#[must_use]
pub fn is_bootstrap_token(credentials: &str) -> bool {
    let ok = |s: &str, n: usize| {
        s.len() == n
            && s.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
    };
    credentials
        .split_once('.')
        .is_some_and(|(id, secret)| ok(id, 6) && ok(secret, 16))
}

impl Config {
    /// Parses and checks a configuration file.
    ///
    /// # Errors
    ///
    /// When the YAML does not match or a value is invalid.
    pub fn parse(yaml: &str) -> Result<Self, PlanError> {
        let config: Self =
            serde_saphyr::from_str(yaml).map_err(|e| PlanError::Config(e.to_string()))?;
        config.check()?;
        Ok(config)
    }

    /// Checks the values serde cannot.
    ///
    /// # Errors
    ///
    /// The first invalid value.
    pub fn check(&self) -> Result<(), PlanError> {
        check_version(
            self.distribution,
            self.rke2_version.as_ref(),
            self.k3s_version.as_ref(),
        )?;
        check_disable(self.distribution, &self.disable)?;
        self.host.check()
    }

    /// The release chosen instead of the pinned one, for the distribution.
    #[must_use]
    pub fn version(&self) -> Option<&str> {
        version(
            self.distribution,
            self.rke2_version.as_ref(),
            self.k3s_version.as_ref(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_file_installs_k3s() {
        let c = Config::parse("{}\n").unwrap();
        assert_eq!(c, Config::default());
        assert_eq!(c.distribution, Distribution::K3s);
        let solo = Config::parse("distribution: kubesolo\n").unwrap();
        assert_eq!(solo.distribution, Distribution::KubeSolo);
        assert!(Config::parse("typo: 1\n").is_err());
        assert!(Config::parse("use: personal\n").is_err());
    }

    #[test]
    fn a_node_name_must_be_a_kubernetes_name() {
        let named = Config::parse("host: {nodeName: db-1}\n").unwrap();
        assert_eq!(named.host.node_name.as_deref(), Some("db-1"));
        assert!(Config::parse("host: {nodeName: DB_1}\n").is_err());
        assert!(Config::parse("distribution: kubesolo\nhost: {nodeName: -solo}\n").is_err());
        assert!(join("host: {nodeName: Worker.1}\n").is_err());
        assert!(join("host: {nodeName: worker-1}\n").is_ok());
    }

    const HASH: &str = "2c2a4a3f5e0e5e7f3c1b1d9a8f7e6d5c4b3a29181716151413121110090807aa";

    fn join(extra: &str) -> Result<Document, PlanError> {
        Document::parse(&format!(
            "distribution: rke2\njoin:\n  server: https://10.0.0.5:9345\n  token: K10{HASH}::abcdef.0123456789abcdef\n{extra}"
        ))
    }

    #[test]
    fn a_join_key_makes_a_join_configuration() {
        let Document::Join(c) = join("").unwrap() else {
            panic!("not a join");
        };
        assert_eq!(c.join.role, Role::Worker);
        assert!(!format!("{c:?}").contains("0123456789abcdef"));
        assert!(matches!(
            Document::parse("{}\n").unwrap(),
            Document::Install(_)
        ));
    }

    #[test]
    fn join_rejects_what_does_not_fit() {
        assert!(join("host: {tlsSan: [k8s.example.com]}\n").is_err());
        assert!(join("  tokenFile: /etc/zeatend/join-token\n").is_err());
        // Bootstrap tokens cannot join servers.
        let err = join("  role: server\n").unwrap_err();
        assert!(matches!(err, PlanError::BootstrapTokenForServer), "{err}");
        let server = Document::parse(&format!(
            "distribution: rke2\njoin:\n  server: https://10.0.0.5:9345\n  role: server\n  token: K10{HASH}::server:s3cret\n"
        ));
        assert!(matches!(server, Ok(Document::Join(_))), "{server:?}");
        let solo = Document::parse(&format!(
            "distribution: kubesolo\njoin:\n  server: https://10.0.0.5:6443\n  token: K10{HASH}::abcdef.0123456789abcdef\n"
        ));
        assert!(solo.is_err());
    }

    #[test]
    fn k3s_takes_workers_and_points_a_second_server_at_rke2() {
        let worker = Document::parse(&format!(
            "join:\n  server: https://10.0.0.5:6443\n  token: K10{HASH}::abcdef.0123456789abcdef\n"
        ));
        let Ok(Document::Join(worker)) = worker else {
            panic!("{worker:?}");
        };
        assert_eq!(worker.distribution, Distribution::K3s);
        let err = Document::parse(&format!(
            "join:\n  server: https://10.0.0.5:6443\n  role: server\n  token: K10{HASH}::server:s3cret\n"
        ))
        .unwrap_err();
        assert!(matches!(err, PlanError::SecondServer), "{err}");
        assert!(err.to_string().contains("distribution: rke2"), "{err}");
    }

    #[test]
    fn a_release_override_must_match_the_distribution() {
        assert!(Config::parse("k3sVersion: v1.37.1+k3s1\n").is_ok());
        assert!(Config::parse("rke2Version: v1.37.1+rke2r1\n").is_err());
        let rke2 = Config::parse("distribution: rke2\nrke2Version: v1.37.1+rke2r1\n").unwrap();
        assert_eq!(rke2.version(), Some("v1.37.1+rke2r1"));
        assert!(Config::parse("distribution: rke2\nk3sVersion: v1.37.1+k3s1\n").is_err());
        assert!(Config::parse("distribution: kubesolo\nk3sVersion: v1.37.1+k3s1\n").is_err());
        for bad in [
            "stable",
            "1.37.1+k3s1",
            "v1.37+k3s1",
            "v1.37.1",
            "v1.37.1+rke2r1",
        ] {
            let err = Config::parse(&format!("k3sVersion: {bad}\n")).unwrap_err();
            assert!(err.to_string().contains("k3sVersion"), "{bad}: {err}");
        }
        assert!(Config::parse("distribution: rke2\nrke2Version: v1.37.1+k3s1\n").is_err());
    }

    #[test]
    fn server_urls_and_tokens() {
        assert_eq!(
            server_address("https://10.0.0.5:9345", 6443).unwrap(),
            ("10.0.0.5".to_owned(), 9345)
        );
        assert_eq!(
            server_address("https://[fd00::5]:9345/", 9345).unwrap(),
            ("fd00::5".to_owned(), 9345)
        );
        assert_eq!(
            server_address("https://k8s.example.com", 6443).unwrap(),
            ("k8s.example.com".to_owned(), 6443)
        );
        for bad in [
            "http://10.0.0.5:9345",
            "https://10.0.0.5:9345/v1",
            "https://:9345",
            "https://fd00::5",
        ] {
            assert!(server_address(bad, 9345).is_err(), "{bad}");
        }
        check_token("abcdef.0123456789abcdef", Role::Worker).unwrap();
        assert!(matches!(
            check_token("abcdef.0123456789abcdef", Role::Server),
            Err(PlanError::BootstrapTokenForServer)
        ));
        assert!(!token_pins_ca("abcdef.0123456789abcdef"));
        assert!(token_pins_ca(&format!(
            "K10{HASH}::abcdef.0123456789abcdef"
        )));
        assert!(check_token("K10abc::abcdef.0123456789abcdef", Role::Worker).is_err());
        assert!(check_token("K10", Role::Worker).is_err());
        check_token(&format!("K10{HASH}::server:x"), Role::Server).unwrap();
    }

    #[test]
    fn disable_names_components_the_distribution_packages() {
        let k3s = Config::parse("disable: [traefik, metrics-server, local-storage]\n").unwrap();
        assert_eq!(
            k3s.disable,
            [
                Component::Traefik,
                Component::MetricsServer,
                Component::LocalStorage
            ]
        );
        assert!(Config::parse("disable: [servicelb]\n").is_err());
        assert!(Config::parse("distribution: rke2\ndisable: [traefik, metrics-server]\n").is_ok());
        assert!(Config::parse("distribution: rke2\ndisable: [local-storage]\n").is_err());
        assert!(Config::parse("distribution: kubesolo\ndisable: [traefik]\n").is_err());
        assert!(join("disable: [traefik]\n").is_err());
        let server = Document::parse(&format!(
            "distribution: rke2\njoin:\n  server: https://10.0.0.5:9345\n  role: server\n  token: K10{HASH}::server:x\ndisable: [traefik]\n"
        ));
        assert!(matches!(server, Ok(Document::Join(c)) if c.disable == [Component::Traefik]));
    }
}
