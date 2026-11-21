//! Registry mirrors, registry hosts and the proxy: how a node reaches images and downloads when
//! it cannot reach the registries or GitHub itself.
//!
//! No mirror is built in. containerd tries each mirror of a registry in turn and then the
//! registry itself (<https://github.com/containerd/containerd/blob/main/docs/hosts.md>), so
//! zeatend writes the settings and adds no fallback of its own. K3s and RKE2 read
//! `registries.yaml` (<https://docs.k3s.io/installation/private-registry>) and write containerd's
//! `hosts.toml` files from it. `KubeSolo` points containerd at
//! `/var/lib/kubesolo/containerd/registry`, so zeatend writes the `hosts.toml` files there itself.

use std::collections::BTreeMap;

use base64::Engine as _;
use serde::{Deserialize, Serialize};

use crate::PlanError;
use crate::cluster::{Distribution, HostFile};
use crate::config::Role;
use crate::uninstall::{Remove, parent};

/// The registry key that stands for every registry without mirrors of its own.
pub const ANY: &str = "*";

/// `KubeSolo`'s containerd `config_path`: a directory for each registry, holding its `hosts.toml`.
pub const KUBESOLO_REGISTRY_DIR: &str = "/var/lib/kubesolo/containerd/registry";

/// The first line of every file zeatend writes for these settings. A file without it is
/// someone else's, and zeatend does not remove it.
pub const HEADER: &str = "# Written by zeatend; `zeatend apply` writes it again.\n";

/// The pod and service networks of K3s, RKE2 and `KubeSolo`, and the names inside the cluster:
/// the node's runtime reaches them directly.
const CLUSTER_NO_PROXY: [&str; 7] = [
    "127.0.0.0/8",
    "::1",
    "localhost",
    "10.42.0.0/16",
    "10.43.0.0/16",
    ".svc",
    ".cluster.local",
];

/// A registry or mirror host's credentials and CA.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RegistryHost {
    /// The user name; set with `passwordFile`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    /// A file (mode 0600) holding the password. zeatend reads it on the host when it runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password_file: Option<String>,
    /// A PEM file of the CA that signed the host's certificate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ca_file: Option<String>,
}

/// The proxy for HTTPS: zeatend's downloads and the node's service, so containerd's pulls too.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Proxy {
    /// The proxy's URL, `http://<host>:<port>` or `https://<host>:<port>`.
    pub https: String,
    /// Hosts, domains (`.example.com`) and networks (`10.0.0.0/8`) reached directly.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub no_proxy: Vec<String>,
}

impl Proxy {
    /// `NO_PROXY` of the node's service: the configured entries, the cluster's own networks and
    /// names, and `direct`, such as the node's addresses and the server it joins.
    #[must_use]
    pub fn node_no_proxy(&self, direct: &[String]) -> String {
        let mut entries: Vec<&str> = Vec::new();
        for entry in self
            .no_proxy
            .iter()
            .map(String::as_str)
            .chain(CLUSTER_NO_PROXY)
            .chain(direct.iter().map(String::as_str))
        {
            if !entries.contains(&entry) {
                entries.push(entry);
            }
        }
        entries.join(",")
    }

    fn check(&self) -> Result<(), PlanError> {
        let url = Url::parse(&self.https)
            .filter(|url| url.path.is_empty() || url.path == "/")
            .ok_or_else(|| {
                PlanError::Config(format!(
                    "proxy.https `{}` is not of the form http://<host>:<port>",
                    self.https
                ))
            })?;
        if url.userinfo {
            return Err(PlanError::Config(
                "proxy.https: a user name or password in the URL would be kept in the \
                 configuration; allow this host on the proxy by its address instead"
                    .to_owned(),
            ));
        }
        if let Some(bad) = self
            .no_proxy
            .iter()
            .find(|entry| entry.is_empty() || !entry.chars().all(plain))
        {
            return Err(PlanError::Config(format!(
                "proxy.noProxy: `{bad}` is not a host, a domain or a network"
            )));
        }
        Ok(())
    }
}

/// A password read from a registry host's `passwordFile`. `Debug` leaves it out.
#[derive(Clone, PartialEq, Eq)]
pub struct Password(pub String);

impl std::fmt::Debug for Password {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<redacted>")
    }
}

/// The three settings of a configuration, and the passwords read for them.
#[derive(Debug, Clone, Copy)]
pub struct Settings<'a> {
    /// For each registry, or [`ANY`], the mirror endpoints to try before the registry itself.
    pub mirrors: &'a BTreeMap<String, Vec<String>>,
    /// Credentials and CAs, by registry or mirror host.
    pub hosts: &'a BTreeMap<String, RegistryHost>,
    /// The proxy.
    pub proxy: Option<&'a Proxy>,
    /// The password of each host in `hosts` with a `passwordFile`.
    pub passwords: &'a BTreeMap<String, Password>,
}

/// Checks the three settings of a configuration.
///
/// # Errors
///
/// The first value that is not valid.
pub fn check(
    mirrors: &BTreeMap<String, Vec<String>>,
    hosts: &BTreeMap<String, RegistryHost>,
    proxy: Option<&Proxy>,
) -> Result<(), PlanError> {
    for (registry, endpoints) in mirrors {
        if registry != ANY && !is_host(registry) {
            return Err(PlanError::Config(format!(
                "registryMirrors: `{registry}` is not a registry host such as docker.io, or `*`"
            )));
        }
        if endpoints.is_empty() {
            return Err(PlanError::Config(format!(
                "registryMirrors.{registry}: list at least one mirror, or remove it"
            )));
        }
        for endpoint in endpoints {
            let url = Url::parse(endpoint)
                .filter(|url| !url.userinfo)
                .ok_or_else(|| {
                    PlanError::Config(format!(
                        "registryMirrors.{registry}: `{endpoint}` is not of the form \
                     https://<host>[:<port>][/<path>]; set credentials in registryHosts"
                    ))
                })?;
            if !url.https && hosts.contains_key(url.host) {
                return Err(PlanError::Config(format!(
                    "registryMirrors.{registry}: `{endpoint}` is http://, and registryHosts \
                     gives {} credentials or a CA; use https:// so they are not sent in the clear",
                    url.host
                )));
            }
        }
    }
    for (host, settings) in hosts {
        if !is_host(host) {
            return Err(PlanError::Config(format!(
                "registryHosts: `{host}` is not a host such as mirror.example.com:5000"
            )));
        }
        match (&settings.username, &settings.password_file) {
            (Some(_), None) | (None, Some(_)) => {
                return Err(PlanError::Config(format!(
                    "registryHosts.{host}: set username and passwordFile together"
                )));
            }
            (None, None) if settings.ca_file.is_none() => {
                return Err(PlanError::Config(format!(
                    "registryHosts.{host}: set username and passwordFile, caFile, or both"
                )));
            }
            _ => {}
        }
        if let Some(username) = &settings.username
            && (username.is_empty()
                || username.contains(':')
                || !username.chars().all(|c| c.is_ascii_graphic()))
        {
            return Err(PlanError::Config(format!(
                "registryHosts.{host}.username: `{username}` is empty or holds a colon or a space"
            )));
        }
        for (key, path) in [
            ("passwordFile", &settings.password_file),
            ("caFile", &settings.ca_file),
        ] {
            if let Some(path) = path
                && !(path.starts_with('/') && path.chars().all(plain))
            {
                return Err(PlanError::Config(format!(
                    "registryHosts.{host}.{key}: `{path}` is not an absolute path"
                )));
            }
        }
    }
    proxy.map_or(Ok(()), Proxy::check)
}

/// Whether `c` may stand in a URL, path or `NO_PROXY` entry that zeatend writes into TOML, YAML
/// or an environment file without quoting it further.
const fn plain(c: char) -> bool {
    c.is_ascii_graphic() && !matches!(c, '"' | '\'' | '\\' | ',' | '#' | '$' | '`')
}

/// Whether `host` is `<name>[:<port>]` or `[<IPv6>][:<port>]`.
fn is_host(host: &str) -> bool {
    let (name, port) = if let Some(v6) = host.strip_prefix('[') {
        let Some((address, rest)) = v6.split_once(']') else {
            return false;
        };
        let port = match rest {
            "" => None,
            rest => match rest.strip_prefix(':') {
                Some(port) => Some(port),
                None => return false,
            },
        };
        (address, port)
    } else {
        match host.split_once(':') {
            Some((name, port)) => (name, Some(port)),
            None => (host, None),
        }
    };
    let v6 = host.starts_with('[');
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || (v6 && c == ':'))
        && port.is_none_or(|p| p.parse::<u16>().is_ok_and(|p| p != 0))
}

/// The parts of an endpoint or proxy URL zeatend needs.
struct Url<'a> {
    https: bool,
    userinfo: bool,
    /// `<host>[:<port>]`.
    host: &'a str,
    /// Everything after the host, `""` when there is nothing.
    path: &'a str,
}

impl<'a> Url<'a> {
    fn parse(url: &'a str) -> Option<Self> {
        let (https, rest) = if let Some(rest) = url.strip_prefix("https://") {
            (true, rest)
        } else {
            (false, url.strip_prefix("http://")?)
        };
        if !url.chars().all(plain) || url.contains(['?', '#']) {
            return None;
        }
        let (authority, path) = rest.find('/').map_or((rest, ""), |i| rest.split_at(i));
        let (userinfo, host) = match authority.rsplit_once('@') {
            Some((_, host)) => (true, host),
            None => (false, authority),
        };
        is_host(host).then_some(Self {
            https,
            userinfo,
            host,
            path,
        })
    }

    /// Whether the path is the mirror's own API path, not the registry API's `/v2` that
    /// containerd adds itself. K3s decides the same way.
    fn overrides_path(&self) -> bool {
        !matches!(self.path, "" | "/" | "/v2" | "/v2/")
    }
}

/// The URL that shows whether the mirror `endpoint` answers, and the endpoint's host.
///
/// The URL is the registry API root `/v2/`, or the endpoint itself when its path replaces the API
/// path. The host's `registryHosts` entry holds the CA. `None` for an endpoint [`check`] rejects.
#[must_use]
pub fn probe(endpoint: &str) -> Option<(String, &str)> {
    let url = Url::parse(endpoint)?;
    let probe = if url.overrides_path() {
        endpoint.to_owned()
    } else {
        let root = endpoint.trim_end_matches('/');
        format!("{}/v2/", root.strip_suffix("/v2").unwrap_or(root))
    };
    Some((probe, url.host))
}

impl Settings<'_> {
    /// Whether nothing is set.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.mirrors.is_empty() && self.hosts.is_empty() && self.proxy.is_none()
    }

    /// The files of these settings on a node of `distribution` that runs `role`. `direct` names
    /// the hosts the node reaches without the proxy: its addresses and the server it joins.
    ///
    /// # Errors
    ///
    /// When a password was not read, or a file cannot be serialised.
    pub fn files(
        &self,
        distribution: Distribution,
        role: Role,
        direct: &[String],
    ) -> Result<Vec<HostFile>, PlanError> {
        let mut files = if distribution == Distribution::KubeSolo {
            self.hosts_toml()?
        } else {
            self.registries_yaml(distribution)?.into_iter().collect()
        };
        if let Some(proxy) = self.proxy {
            files.extend(proxy_files(distribution, role, proxy, direct));
        }
        Ok(files)
    }

    fn password(&self, host: &str) -> Result<Option<(&str, &str)>, PlanError> {
        let Some(username) = self.hosts.get(host).and_then(|h| h.username.as_deref()) else {
            return Ok(None);
        };
        let password = self.passwords.get(host).ok_or_else(|| {
            PlanError::Config(format!(
                "registryHosts.{host}: the password file was not read"
            ))
        })?;
        Ok(Some((username, password.0.as_str())))
    }

    /// K3s's and RKE2's `registries.yaml`; none when nothing is set.
    fn registries_yaml(&self, distribution: Distribution) -> Result<Option<HostFile>, PlanError> {
        if self.mirrors.is_empty() && self.hosts.is_empty() {
            return Ok(None);
        }
        let mut configs = BTreeMap::new();
        for (host, settings) in self.hosts {
            let auth = self.password(host)?.map(|(username, password)| Auth {
                username: username.to_owned(),
                password: password.to_owned(),
            });
            let tls = settings.ca_file.clone().map(|ca_file| Tls { ca_file });
            configs.insert(host.clone(), RegistryConfig { auth, tls });
        }
        let file = Registries {
            mirrors: self
                .mirrors
                .iter()
                .map(|(registry, endpoints)| {
                    (
                        registry.clone(),
                        Mirror {
                            endpoint: endpoints.clone(),
                        },
                    )
                })
                .collect(),
            configs,
        };
        HostFile::yaml(registries_path(distribution), HEADER, &file).map(Some)
    }

    /// `KubeSolo`'s `hosts.toml` files: one for each registry with mirrors or settings of its
    /// own, and `_default` for [`ANY`].
    fn hosts_toml(&self) -> Result<Vec<HostFile>, PlanError> {
        self.mirrors
            .keys()
            .chain(self.hosts.keys().filter(|h| !self.mirrors.contains_key(*h)))
            .map(|registry| {
                let (dir, server) = if registry == ANY {
                    ("_default", None)
                } else if registry == "docker.io" {
                    ("docker.io", Some("https://registry-1.docker.io".to_owned()))
                } else {
                    (registry.as_str(), Some(format!("https://{registry}")))
                };
                let mut lines = Vec::new();
                if let Some(server) = server {
                    lines.push(format!("server = {}", quoted(&server)));
                    self.host_settings(&mut lines, registry, "")?;
                }
                for endpoint in self.mirrors.get(registry).into_iter().flatten() {
                    let url = Url::parse(endpoint).ok_or_else(|| {
                        PlanError::Config(format!("registryMirrors: `{endpoint}`"))
                    })?;
                    let key = if url.overrides_path() {
                        endpoint.trim_end_matches('/').to_owned()
                    } else {
                        format!(
                            "{}://{}",
                            if url.https { "https" } else { "http" },
                            url.host
                        )
                    };
                    let table = format!("host.{}", quoted(&key));
                    lines.push(String::new());
                    lines.push(format!("[{table}]"));
                    lines.push("  capabilities = [\"pull\", \"resolve\"]".to_owned());
                    if url.overrides_path() {
                        lines.push("  override_path = true".to_owned());
                    }
                    self.host_settings(&mut lines, url.host, &table)?;
                }
                Ok(HostFile {
                    path: format!("{KUBESOLO_REGISTRY_DIR}/{dir}/hosts.toml"),
                    mode: 0o600,
                    content: format!("{HEADER}{}\n", lines.join("\n")),
                })
            })
            .collect()
    }

    /// The CA and the `Authorization` header of `host`, in the table `table` (`""`: the top
    /// level, the registry itself). containerd's `hosts.toml` has no credential fields; a header
    /// goes only to the host whose table holds it.
    fn host_settings(
        &self,
        lines: &mut Vec<String>,
        host: &str,
        table: &str,
    ) -> Result<(), PlanError> {
        let indent = if table.is_empty() { "" } else { "  " };
        if let Some(ca) = self.hosts.get(host).and_then(|h| h.ca_file.as_deref()) {
            lines.push(format!("{indent}ca = {}", quoted(ca)));
        }
        if let Some((username, password)) = self.password(host)? {
            let basic =
                base64::engine::general_purpose::STANDARD.encode(format!("{username}:{password}"));
            let header = if table.is_empty() {
                "header".to_owned()
            } else {
                format!("{table}.header")
            };
            lines.push(format!("{indent}[{header}]"));
            lines.push(format!(
                "{indent}  Authorization = {}",
                quoted(&format!("Basic {basic}"))
            ));
        }
        Ok(())
    }
}

/// A TOML basic string. Everything zeatend quotes is checked to be [`plain`] or is Base64,
/// except a password, which only reaches TOML Base64-encoded.
fn quoted(value: &str) -> String {
    format!("\"{value}\"")
}

/// Where K3s or RKE2 reads its registry settings.
fn registries_path(distribution: Distribution) -> String {
    format!("{}/registries.yaml", distribution.layout().config_dir)
}

/// The environment file with the proxy, and the unit drop-in that reads it.
fn proxy_paths(distribution: Distribution, role: Role) -> (String, String) {
    let layout = distribution.layout();
    (
        format!("{}/zeatend-proxy.env", layout.config_dir),
        format!(
            "/etc/systemd/system/{}.service.d/zeatend-proxy.conf",
            layout.unit(role)
        ),
    )
}

/// The proxy's environment and the drop-in of the node's unit. The drop-in names an
/// environment file, as a file it names overrides the unit's own environment files, such as the
/// one K3s's `install.sh` writes.
fn proxy_files(
    distribution: Distribution,
    role: Role,
    proxy: &Proxy,
    direct: &[String],
) -> [HostFile; 2] {
    let (environment, drop_in) = proxy_paths(distribution, role);
    [
        HostFile {
            content: format!(
                "{HEADER}HTTPS_PROXY={}\nNO_PROXY={}\n",
                proxy.https,
                proxy.node_no_proxy(direct)
            ),
            path: environment.clone(),
            mode: 0o600,
        },
        HostFile {
            path: drop_in,
            mode: 0o644,
            content: format!("{HEADER}[Service]\nEnvironmentFile={environment}\n"),
        },
    ]
}

/// The files of these settings that a node of `distribution` running `role` may have, besides
/// the `hosts.toml` files under [`KUBESOLO_REGISTRY_DIR`]. zeatend removes one it wrote and no
/// longer writes.
#[must_use]
pub fn optional_files(distribution: Distribution, role: Role) -> Vec<String> {
    let (environment, drop_in) = proxy_paths(distribution, role);
    let mut paths = vec![environment, drop_in];
    if distribution != Distribution::KubeSolo {
        paths.push(registries_path(distribution));
    }
    paths
}

/// What `uninstall` removes of these settings on a node of `distribution` running `role`.
/// `KubeSolo`'s `hosts.toml` files are in its data directory, which goes as a whole.
#[must_use]
pub fn footprint(distribution: Distribution, role: Role) -> Vec<Remove> {
    let (_, drop_in) = proxy_paths(distribution, role);
    optional_files(distribution, role)
        .into_iter()
        .map(Remove::Tree)
        .chain([Remove::EmptyDir(parent(&drop_in))])
        .collect()
}

/// What zeatend removes: the files among `found` it wrote before that `files` no longer holds.
#[must_use]
pub fn removed(found: &[String], files: &[HostFile]) -> Vec<String> {
    found
        .iter()
        .filter(|path| !files.iter().any(|f| &f.path == *path))
        .cloned()
        .collect()
}

/// K3s's and RKE2's `registries.yaml`.
#[derive(Serialize)]
struct Registries {
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    mirrors: BTreeMap<String, Mirror>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    configs: BTreeMap<String, RegistryConfig>,
}

#[derive(Serialize)]
struct Mirror {
    endpoint: Vec<String>,
}

#[derive(Serialize)]
struct RegistryConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    auth: Option<Auth>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tls: Option<Tls>,
}

#[derive(Serialize)]
struct Auth {
    username: String,
    password: String,
}

#[derive(Serialize)]
struct Tls {
    ca_file: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mirror_is_probed_at_the_api_root_unless_its_path_replaces_it() {
        for (endpoint, url, host) in [
            (
                "https://mirror.example",
                "https://mirror.example/v2/",
                "mirror.example",
            ),
            (
                "https://mirror.example/v2/",
                "https://mirror.example/v2/",
                "mirror.example",
            ),
            (
                "http://10.0.0.5:5000",
                "http://10.0.0.5:5000/v2/",
                "10.0.0.5:5000",
            ),
            (
                "https://harbor.example/v2/docker.io",
                "https://harbor.example/v2/docker.io",
                "harbor.example",
            ),
        ] {
            assert_eq!(probe(endpoint), Some((url.to_owned(), host)), "{endpoint}");
        }
        assert_eq!(probe("ftp://mirror.example"), None);
    }

    struct Given {
        mirrors: BTreeMap<String, Vec<String>>,
        hosts: BTreeMap<String, RegistryHost>,
        proxy: Option<Proxy>,
        passwords: BTreeMap<String, Password>,
    }

    impl Given {
        fn parse(yaml: &str) -> Self {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Doc {
                #[serde(default)]
                registry_mirrors: BTreeMap<String, Vec<String>>,
                #[serde(default)]
                registry_hosts: BTreeMap<String, RegistryHost>,
                proxy: Option<Proxy>,
            }
            let doc: Doc = serde_saphyr::from_str(yaml).unwrap();
            check(
                &doc.registry_mirrors,
                &doc.registry_hosts,
                doc.proxy.as_ref(),
            )
            .unwrap();
            let passwords = doc
                .registry_hosts
                .iter()
                .filter(|(_, h)| h.password_file.is_some())
                .map(|(host, _)| (host.clone(), Password(format!("pw-{host}"))))
                .collect();
            Self {
                mirrors: doc.registry_mirrors,
                hosts: doc.registry_hosts,
                proxy: doc.proxy,
                passwords,
            }
        }

        fn settings(&self) -> Settings<'_> {
            Settings {
                mirrors: &self.mirrors,
                hosts: &self.hosts,
                proxy: self.proxy.as_ref(),
                passwords: &self.passwords,
            }
        }

        fn files(&self, distribution: Distribution) -> Vec<HostFile> {
            self.settings()
                .files(distribution, Role::Server, &["10.0.0.5".to_owned()])
                .unwrap()
        }
    }

    fn checked(yaml: &str) -> Result<(), PlanError> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Doc {
            #[serde(default)]
            registry_mirrors: BTreeMap<String, Vec<String>>,
            #[serde(default)]
            registry_hosts: BTreeMap<String, RegistryHost>,
            proxy: Option<Proxy>,
        }
        let doc: Doc =
            serde_saphyr::from_str(yaml).map_err(|e| PlanError::Config(e.to_string()))?;
        check(
            &doc.registry_mirrors,
            &doc.registry_hosts,
            doc.proxy.as_ref(),
        )
    }

    const HARBOR: &str = "registryMirrors:
  '*': [https://harbor.example.com/v2/proxy-cache]
  docker.io: [https://harbor.example.com/v2/docker-hub, https://mirror.gcr.io]
registryHosts:
  harbor.example.com:
    username: robot$pull
    passwordFile: /etc/zeatend/harbor-password
    caFile: /etc/zeatend/harbor-ca.pem
";

    #[test]
    fn kubesolo_gets_a_hosts_toml_with_credentials_a_ca_an_override_path_and_a_default() {
        let given = Given::parse(&HARBOR.replace("robot$pull", "robot-pull"));
        let files = given.files(Distribution::KubeSolo);
        let basic =
            base64::engine::general_purpose::STANDARD.encode("robot-pull:pw-harbor.example.com");
        let default = &files[0];
        assert_eq!(
            default.path,
            "/var/lib/kubesolo/containerd/registry/_default/hosts.toml"
        );
        assert_eq!(default.mode, 0o600);
        assert_eq!(
            default.content,
            format!(
                "{HEADER}
[host.\"https://harbor.example.com/v2/proxy-cache\"]
  capabilities = [\"pull\", \"resolve\"]
  override_path = true
  ca = \"/etc/zeatend/harbor-ca.pem\"
  [host.\"https://harbor.example.com/v2/proxy-cache\".header]
    Authorization = \"Basic {basic}\"
"
            )
        );
        let docker = &files[1];
        assert_eq!(
            docker.path,
            "/var/lib/kubesolo/containerd/registry/docker.io/hosts.toml"
        );
        assert_eq!(
            docker.content,
            format!(
                "{HEADER}server = \"https://registry-1.docker.io\"

[host.\"https://harbor.example.com/v2/docker-hub\"]
  capabilities = [\"pull\", \"resolve\"]
  override_path = true
  ca = \"/etc/zeatend/harbor-ca.pem\"
  [host.\"https://harbor.example.com/v2/docker-hub\".header]
    Authorization = \"Basic {basic}\"

[host.\"https://mirror.gcr.io\"]
  capabilities = [\"pull\", \"resolve\"]
"
            )
        );
        // The mirror host's own settings make a directory of their own, for pulls from it.
        assert_eq!(
            files[2].path,
            "/var/lib/kubesolo/containerd/registry/harbor.example.com/hosts.toml"
        );
        assert!(
            files[2].content.starts_with(&format!(
                "{HEADER}server = \"https://harbor.example.com\"\nca = "
            )),
            "{}",
            files[2].content
        );
        assert!(files[2].content.contains("[header]\n  Authorization = "));
        assert_eq!(files.len(), 3);
    }

    #[test]
    fn k3s_and_rke2_get_registries_yaml_with_the_same_settings() {
        let given = Given::parse(HARBOR);
        for (distribution, dir) in [
            (Distribution::K3s, "/etc/rancher/k3s"),
            (Distribution::Rke2, "/etc/rancher/rke2"),
        ] {
            let files = given.files(distribution);
            assert_eq!(files.len(), 1);
            assert_eq!(files[0].path, format!("{dir}/registries.yaml"));
            assert_eq!(files[0].mode, 0o600);
            let yaml: serde_json::Value = serde_saphyr::from_str(&files[0].content).unwrap();
            assert_eq!(
                yaml,
                serde_json::json!({
                    "mirrors": {
                        "*": {"endpoint": ["https://harbor.example.com/v2/proxy-cache"]},
                        "docker.io": {"endpoint": [
                            "https://harbor.example.com/v2/docker-hub",
                            "https://mirror.gcr.io"
                        ]},
                    },
                    "configs": {"harbor.example.com": {
                        "auth": {"username": "robot$pull", "password": "pw-harbor.example.com"},
                        "tls": {"ca_file": "/etc/zeatend/harbor-ca.pem"},
                    }},
                })
            );
        }
    }

    #[test]
    fn credentials_and_cas_are_never_for_http() {
        let err = checked(
            "registryMirrors: {docker.io: ['http://mirror.example.com:5000']}\n\
             registryHosts: {'mirror.example.com:5000': {caFile: /etc/ca.pem}}\n",
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("use https://"), "{err}");
        // A mirror over http:// without credentials is a local mirror; it is allowed.
        let local = Given::parse("registryMirrors: {docker.io: ['http://10.0.0.9:5000']}\n");
        let toml = &local.files(Distribution::KubeSolo)[0].content;
        assert!(toml.contains("[host.\"http://10.0.0.9:5000\"]"), "{toml}");
        for bad in [
            "registryMirrors: {docker.io: ['https://user:pw@mirror.example.com']}\n",
            "registryMirrors: {docker.io: ['mirror.example.com']}\n",
            "registryMirrors: {docker.io: []}\n",
            "registryMirrors: {'docker.io/library': ['https://mirror.example.com']}\n",
            "registryHosts: {mirror.example.com: {username: pull}}\n",
            "registryHosts: {mirror.example.com: {}}\n",
            "registryHosts: {mirror.example.com: {caFile: ca.pem}}\n",
            "proxy: {https: 'http://user:pw@proxy.example.com:3128'}\n",
            "proxy: {https: 'proxy.example.com:3128'}\n",
            "proxy: {https: 'http://proxy.example.com:3128', noProxy: ['a,b']}\n",
        ] {
            assert!(checked(bad).is_err(), "accepted: {bad}");
        }
    }

    #[test]
    fn the_proxy_reaches_the_node_unit_and_the_cluster_stays_direct() {
        let given = Given::parse(
            "proxy: {https: 'http://proxy.example.com:3128', noProxy: [.internal, 10.0.0.5]}\n",
        );
        let files = given.files(Distribution::Rke2);
        assert_eq!(files[0].path, "/etc/rancher/rke2/zeatend-proxy.env");
        assert_eq!(
            files[0].content,
            format!(
                "{HEADER}HTTPS_PROXY=http://proxy.example.com:3128\n\
                 NO_PROXY=.internal,10.0.0.5,127.0.0.0/8,::1,localhost,10.42.0.0/16,\
                 10.43.0.0/16,.svc,.cluster.local\n"
            )
        );
        assert_eq!(
            files[1].path,
            "/etc/systemd/system/rke2-server.service.d/zeatend-proxy.conf"
        );
        assert!(
            files[1]
                .content
                .ends_with("[Service]\nEnvironmentFile=/etc/rancher/rke2/zeatend-proxy.env\n")
        );
        let worker = given
            .settings()
            .files(Distribution::K3s, Role::Worker, &[])
            .unwrap();
        assert_eq!(
            worker[1].path,
            "/etc/systemd/system/k3s-agent.service.d/zeatend-proxy.conf"
        );
        let solo = given.files(Distribution::KubeSolo);
        assert_eq!(
            solo.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(),
            [
                "/etc/kubesolo/zeatend-proxy.env",
                "/etc/systemd/system/kubesolo.service.d/zeatend-proxy.conf"
            ]
        );
    }

    #[test]
    fn files_zeatend_no_longer_writes_are_removed() {
        let given = Given::parse("registryMirrors: {docker.io: ['https://mirror.gcr.io']}\n");
        let files = given.files(Distribution::K3s);
        let found = optional_files(Distribution::K3s, Role::Server);
        assert_eq!(
            removed(&found, &files),
            [
                "/etc/rancher/k3s/zeatend-proxy.env",
                "/etc/systemd/system/k3s.service.d/zeatend-proxy.conf"
            ]
        );
    }
}
