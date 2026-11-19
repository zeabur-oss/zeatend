//! `node add`, run on a server of a K3s or RKE2 cluster: what a new VM needs to join it.
//!
//! A worker gets a bootstrap token that expires, made by `k3s token create` or
//! `rke2 token create` (<https://docs.k3s.io/cli/token>). An RKE2 server gets the cluster's server
//! token, which a bootstrap token cannot replace. K3s takes workers only. The new VM's own
//! `zeatend plan` then checks the ports and the round-trip time to this server before it joins.

use std::path::Path;
use std::time::Duration;

use k8s_openapi::List;
use k8s_openapi::api::core::v1::Node;
use zeatend_host::kubectl::Kubectl;
use zeatend_host::{cmd, facts, files};
use zeatend_plan::cluster::{self, Rancher};
use zeatend_plan::config::{Component, Role};
use zeatend_plan::token::JoinToken;
use zeatend_plan::{PlanError, join, ports};

use crate::{InstallError, apply, output};

/// What `node add` takes from the command line.
#[derive(Debug, Clone)]
pub(crate) struct AddOptions {
    pub(crate) role: Role,
    /// The address the new VM reaches this server at.
    pub(crate) address: Option<String>,
    /// How long a worker's token can join nodes.
    pub(crate) ttl: Duration,
}

/// The distribution of this server: the one whose admin kubeconfig is here.
fn server_distribution() -> Result<Rancher, InstallError> {
    Rancher::ALL
        .into_iter()
        .find(|d| Path::new(d.layout().admin_kubeconfig).exists())
        .ok_or_else(|| {
            let kubesolo = Path::new(zeatend_plan::kubesolo::CONFIG_FILE).exists();
            if kubesolo {
                InstallError::KubeSoloTakesNoNodes
            } else {
                InstallError::NotAServer
            }
        })
}

pub(crate) async fn add(options: &AddOptions) -> Result<(), InstallError> {
    let distribution = server_distribution()?;
    if options.role == Role::Server && !distribution.takes_servers() {
        return Err(PlanError::SecondServer.into());
    }
    let layout = distribution.layout();
    let kubectl = Kubectl::admin(distribution);
    let nodes: List<Node> = kubectl.raw("/api/v1/nodes").await?;
    let nodes = nodes.items;
    let address = if let Some(address) = &options.address {
        address.clone()
    } else {
        let name = facts::read()
            .await?
            .node_name
            .ok_or(InstallError::NoNodeName)?;
        nodes
            .iter()
            .find(|n| n.metadata.name.as_ref() == Some(&name))
            .and_then(internal_ip)
            .ok_or_else(|| InstallError::NoInternalIp(name.clone()))?
    };
    let token = match options.role {
        Role::Worker => {
            let ttl = format!("{}s", options.ttl.as_secs());
            let out = cmd::run(
                &apply::release_binary(distribution),
                &[
                    "token",
                    "create",
                    "--ttl",
                    &ttl,
                    "--description",
                    "zeatend node add",
                ],
                None,
            )
            .await?;
            Token::Bootstrap(out.trim().to_owned())
        }
        Role::Server => {
            let text = String::from_utf8(files::read_secret(Path::new(layout.server_token_file))?)
                .map_err(|_e| InstallError::NotText(layout.server_token_file.into()))?;
            Token::Server(text.trim().to_owned())
        }
    };
    let record = crate::installer::read_record(distribution.into()).await?;
    // A server turns off what this one turns off. A worker deploys no packaged components.
    let disable = match options.role {
        Role::Server => {
            let path = format!("{}/config.yaml", layout.config_dir);
            let config = std::fs::read_to_string(&path).map_err(InstallError::io(&path))?;
            cluster::disabled_components(distribution.into(), &config)?
        }
        Role::Worker => Vec::new(),
    };
    let text = instructions(&Instructions {
        distribution,
        server: &ports::api_server_url(&address, distribution.join_port()),
        role: options.role,
        token: &token,
        version: record.as_ref().and_then(|r| r.version.as_deref()),
        node_names: &nodes
            .iter()
            .filter_map(|n| n.metadata.name.as_deref())
            .collect::<Vec<_>>(),
        disable: &disable,
        ttl: options.ttl,
    });
    output::line(text.trim_end());
    Ok(())
}

fn internal_ip(node: &Node) -> Option<String> {
    node.status
        .as_ref()?
        .addresses
        .as_ref()?
        .iter()
        .find(|a| a.type_ == "InternalIP")
        .map(|a| a.address.clone())
}

/// The token a new node joins with.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    /// A bootstrap token, `K10<ca-hash>::<id>.<secret>`: it joins workers until it expires.
    Bootstrap(String),
    /// The cluster's server token: it joins servers and workers and does not expire.
    Server(String),
}

impl Token {
    fn text(&self) -> &str {
        match self {
            Self::Bootstrap(t) | Self::Server(t) => t,
        }
    }

    /// The id of a bootstrap token, which `token delete` takes.
    fn bootstrap_id(&self) -> Option<&str> {
        match self {
            Self::Bootstrap(t) => JoinToken::parse(t).ok()?.bootstrap_id(),
            Self::Server(_) => None,
        }
    }
}

/// What `node add` prints.
#[derive(Debug)]
struct Instructions<'a> {
    distribution: Rancher,
    /// The URL the new VM joins through.
    server: &'a str,
    role: Role,
    token: &'a Token,
    /// The cluster's release, as recorded.
    ///
    /// The configuration names it only when it is not this zeatend's pinned release, so the new
    /// node installs it too. Otherwise the node follows the pin, also when a newer zeatend
    /// upgrades the cluster.
    version: Option<&'a str>,
    /// The cluster's nodes, whose names the new node does not take.
    node_names: &'a [&'a str],
    /// The packaged components that a new server turns off, as this one does.
    disable: &'a [Component],
    ttl: Duration,
}

fn push(text: &mut String, line: &str) {
    text.push_str(line);
    text.push('\n');
}

/// The text `node add` prints.
fn instructions(i: &Instructions<'_>) -> String {
    let mut text = String::new();
    push(
        &mut text,
        &format!(
            "Add a {} to this cluster through {}.",
            i.role.as_str(),
            i.server
        ),
    );
    push(&mut text, "");
    push(&mut text, "Before you start");
    for advice in join::advice(i.distribution, i.role) {
        push(
            &mut text,
            &format!("  - {}", crate::i18n::en::finding(&advice)),
        );
    }
    push(&mut text, "");
    push(
        &mut text,
        "On the new VM, as root, with this zeatend copied to /usr/local/bin:",
    );
    push(&mut text, "");
    text.push_str(&commands(i));
    push(&mut text, "");
    match i.token {
        Token::Bootstrap(_) => push(
            &mut text,
            &format!(
                "The token joins workers for {}. Revoke it once the node is Ready; the node \
                 keeps working without it.",
                duration(i.ttl)
            ),
        ),
        Token::Server(_) => push(
            &mut text,
            "This is the cluster's server token: it does not expire and joins servers and \
             workers. Keep it out of chats and tickets.",
        ),
    }
    push(&mut text, "");
    let layout = i.distribution.layout();
    push(&mut text, "When the node is Ready, here:");
    push(
        &mut text,
        &format!(
            "  {} --kubeconfig {} get nodes -o wide",
            layout.kubectl, layout.admin_kubeconfig
        ),
    );
    if let Some(id) = i.token.bootstrap_id() {
        push(
            &mut text,
            &format!(
                "  {} token delete {id}   # revokes the token",
                apply::release_binary(i.distribution).display()
            ),
        );
    }
    text
}

/// `30m`, `1h` or `1h30m`.
fn duration(ttl: Duration) -> String {
    let minutes = ttl.as_secs() / 60;
    match (minutes / 60, minutes % 60) {
        (0, m) => format!("{m}m"),
        (h, 0) => format!("{h}h"),
        (h, m) => format!("{h}h{m}m"),
    }
}

/// The shell commands that write the join configuration and run zeatend.
fn commands(i: &Instructions<'_>) -> String {
    let mut lines = vec![format!("distribution: {}", i.distribution.id())];
    if let Some(version) = i.version.filter(|v| *v != i.distribution.pinned_version()) {
        lines.push(format!("{}Version: {version}", i.distribution.id()));
    }
    if !i.disable.is_empty() {
        let names: Vec<&str> = i.disable.iter().map(|c| c.name()).collect();
        lines.push(format!("disable: [{}]", names.join(", ")));
    }
    lines.extend([
        "join:".to_owned(),
        format!("  server: {}", i.server),
        format!("  role: {}", i.role.as_str()),
        format!("  token: {}", i.token.text()),
    ]);
    if !i.node_names.is_empty() {
        lines.push(format!("  nodeNames: [{}]", i.node_names.join(", ")));
    }
    let yaml = lines.join("\n");
    format!(
        "install -d -m 0700 /etc/zeatend\n\
         (umask 077; cat > /etc/zeatend/config.yaml) <<'EOF'\n\
         {yaml}\n\
         EOF\n\
         zeatend plan    # what it installs, the distance to this server, the ports and warnings\n\
         zeatend apply   # installs {} and waits until the node is Ready\n",
        i.distribution.title()
    )
}

#[cfg(test)]
mod tests {
    use zeatend_plan::config::{Document, JoinConfig};

    use super::*;

    const HASH: &str = "8a3f0e0c4b2d9a1e7f6c5b4a39281706f5e4d3c2b1a09f8e7d6c5b4a39281706";

    #[test]
    fn the_address_is_the_first_internal_ip_of_the_node() {
        let node: Node = serde_json::from_str(include_str!("../fixtures/node.json")).unwrap();
        assert_eq!(internal_ip(&node).as_deref(), Some("10.0.0.5"));
        assert_eq!(internal_ip(&Node::default()), None);
    }

    fn worker_token() -> Token {
        Token::Bootstrap(format!("K10{HASH}::abcdef.0123456789abcdef"))
    }

    /// The configuration between the heredoc markers.
    fn pasted(text: &str) -> JoinConfig {
        let yaml = text
            .split_once("<<'EOF'\n")
            .and_then(|(_, rest)| rest.split_once("EOF\n"))
            .map(|(yaml, _)| yaml)
            .unwrap();
        match Document::parse(yaml).unwrap() {
            Document::Join(config) => config,
            Document::Install(_) => panic!("not a join: {yaml}"),
        }
    }

    #[test]
    fn what_node_add_prints_for_a_worker() {
        let text = instructions(&Instructions {
            distribution: Rancher::K3s,
            server: "https://10.0.1.10:6443",
            role: Role::Worker,
            token: &worker_token(),
            version: Some("v1.37.1+k3s1"),
            node_names: &["vps-1"],
            disable: &[],
            ttl: Duration::from_hours(1),
        });
        assert_eq!(text, include_str!("../fixtures/node-add-worker.txt"));
    }

    #[test]
    fn the_pasted_configuration_joins_the_cluster_at_its_release() {
        let text = instructions(&Instructions {
            distribution: Rancher::Rke2,
            server: "https://10.0.1.10:9345",
            role: Role::Worker,
            token: &worker_token(),
            version: Some("v1.37.1+rke2r1"),
            node_names: &["vps-1", "vps-2"],
            disable: &[],
            ttl: Duration::from_mins(30),
        });
        let config = pasted(&text);
        assert_eq!(config.join.server, "https://10.0.1.10:9345");
        assert_eq!(config.join.role, Role::Worker);
        assert_eq!(config.join.node_names, ["vps-1", "vps-2"]);
        assert_eq!(config.disable, []);
        assert_eq!(config.version(), Some("v1.37.1+rke2r1"));
        assert!(text.contains("joins workers for 30m"), "{text}");
        assert!(
            text.contains("/usr/local/bin/rke2 token delete abcdef"),
            "{text}"
        );
    }

    #[test]
    fn a_server_gets_the_server_token_and_no_revoke_line() {
        let token = Token::Server(format!("K10{HASH}::server:0123456789abcdef"));
        let text = instructions(&Instructions {
            distribution: Rancher::Rke2,
            server: "https://10.0.1.10:9345",
            role: Role::Server,
            token: &token,
            version: None,
            node_names: &["vps-1"],
            disable: &[Component::Traefik, Component::MetricsServer],
            ttl: Duration::from_hours(1),
        });
        let config = pasted(&text);
        assert_eq!(config.join.role, Role::Server);
        assert_eq!(config.version(), None);
        assert_eq!(
            config.disable,
            [Component::Traefik, Component::MetricsServer]
        );
        assert!(text.contains("does not expire"), "{text}");

        assert!(!text.contains("token delete"), "{text}");
        assert!(text.contains("three or more servers"), "{text}");
    }

    #[test]
    fn a_cluster_at_the_pin_gives_the_new_node_no_version_so_it_follows_the_pin() {
        let text = instructions(&Instructions {
            distribution: Rancher::K3s,
            server: "https://10.0.1.10:6443",
            role: Role::Worker,
            token: &worker_token(),
            version: Some(Rancher::K3s.pinned_version()),
            node_names: &["vps-1"],
            disable: &[],
            ttl: Duration::from_hours(1),
        });
        assert_eq!(pasted(&text).version(), None, "{text}");
    }

    #[test]
    fn the_bootstrap_id_is_read_with_or_without_the_ca_hash() {
        assert_eq!(worker_token().bootstrap_id(), Some("abcdef"));
        assert_eq!(
            Token::Bootstrap("abcdef.0123456789abcdef".to_owned()).bootstrap_id(),
            Some("abcdef")
        );
    }
}
