//! `plan` and `apply` on a node that joins an existing K3s or RKE2 cluster.

use std::path::Path;
use std::time::Duration;

use k8s_openapi::api::core::v1::Node;
use zeatend_host::kubectl::{self, Kubectl};
use zeatend_host::{facts, files, network, preflight, systemd};
use zeatend_plan::cluster::Rancher;
use zeatend_plan::config::{self, JoinConfig, Role};
use zeatend_plan::join::{self, JoinFacts, JoinPlan, RTT_SAMPLES};
use zeatend_plan::record::Record;

use crate::apply::{self, ClusterInstall};
use crate::{InstallError, output, render};

/// The join token, from the configuration or its file, checked against the role.
/// [`JoinConfig::check`] checked a token in the configuration already.
fn token(config: &JoinConfig) -> Result<String, InstallError> {
    match (&config.join.token, &config.join.token_file) {
        (Some(token), _) => Ok(token.trim().to_owned()),
        (None, Some(file)) => {
            let token = String::from_utf8(files::read_secret(Path::new(file))?)
                .map_err(|_e| InstallError::Message(format!("{file}: not text")))?;
            let token = token.trim().to_owned();
            config::check_token(&token, config.join.role)?;
            Ok(token)
        }
        (None, None) => Err(InstallError::Message(
            "join: no token or tokenFile".to_owned(),
        )),
    }
}

/// Reads the host and the way to the server.
async fn read_facts(config: &JoinConfig, token: &str) -> Result<JoinFacts, InstallError> {
    let distribution = config.rancher()?;
    let (host, port) = config::server_address(&config.join.server, distribution.join_port())?;
    let rtt_us = network::connect_times(&host, port, RTT_SAMPLES).await;
    let mut ports = Vec::new();
    for (p, _) in join::server_ports(distribution, config.join.role) {
        let open = if p == port {
            !rtt_us.is_empty()
        } else {
            network::open(&host, p).await
        };
        ports.push((p, open));
    }
    let unit = match config.join.role {
        Role::Worker => distribution.agent_unit(),
        Role::Server => distribution.server_unit(),
    };
    let mut facts = facts::read().await?;
    facts.record = crate::installer::read_record(distribution.into()).await?;
    if facts.record.is_some() {
        facts.kubernetes = apply::running_kubernetes(&own_kubectl(distribution)).await;
    }
    Ok(JoinFacts {
        facts,
        rtt_us,
        ports,
        token_pinned: config::token_pins_ca(token),
        unit_running: systemd::is_active(unit).await,
    })
}

async fn make_plan(config: &JoinConfig) -> Result<(JoinPlan, String), InstallError> {
    let token = token(config)?;
    let facts = read_facts(config, &token).await?;
    Ok((join::plan(config, &facts)?, token))
}

/// What the plan installs, for the preflight checks.
const fn node(plan: &JoinPlan) -> preflight::Node {
    match plan.role {
        Role::Server => preflight::Node::Server(plan.cluster.distribution),
        Role::Worker => preflight::Node::Agent(plan.cluster.distribution),
    }
}

/// The node's own `kubectl`, with the kubelet's credentials, which an agent also has.
pub(crate) fn own_kubectl(distribution: Rancher) -> Kubectl {
    let layout = distribution.layout();
    Kubectl::new(layout.kubectl, layout.kubelet_kubeconfig)
}

pub(crate) async fn show_plan(config: &JoinConfig, json: bool) -> Result<(), InstallError> {
    let (plan, _) = make_plan(config).await?;
    if json {
        return crate::print_json(&plan);
    }
    output::line(render::join_plan(&plan).trim_end());
    if apply::show_checks(node(&plan), config.host.require_encrypted_disk).await {
        output::line(crate::i18n::messages().apply_stops);
    }
    Ok(())
}

pub(crate) async fn apply(config: &JoinConfig) -> Result<(), InstallError> {
    let (plan, token) = make_plan(config).await?;
    output::line(render::join_plan(&plan).trim_end());
    if let Some(blocker) = plan.blockers.first() {
        return Err(InstallError::Message(blocker.text.clone()));
    }
    apply::check_host(node(&plan), config.host.require_encrypted_disk).await?;

    let distribution = plan.cluster.distribution;
    let layout = distribution.layout();
    let changed = files::write(
        Path::new(layout.join_token_file),
        0o600,
        format!("{token}\n").as_bytes(),
    )?;
    let install = ClusterInstall {
        cluster: &plan.cluster,
        files: &plan.host_files,
        server: plan.role == Role::Server,
        changed,
    };
    let restarted = apply::install_cluster(&install).await?;

    let name = plan.node_name.clone();
    let messages = crate::i18n::messages();
    output::step(&(messages.waiting_for_node)(&name));
    wait_node_ready(
        &own_kubectl(distribution),
        &name,
        restarted.then(|| install.unit()),
    )
    .await?;
    apply::write_record(
        Path::new(apply::STATE_DIR),
        &Record::new(distribution.into(), &plan.cluster.version),
    )?;

    output::step(messages.done);
    output::line(&(messages.joined)(&crate::i18n::Joined {
        node: &name,
        server: plan.role == Role::Server,
        kubectl: &format!(
            "sudo {} --kubeconfig {}",
            layout.kubectl, layout.admin_kubeconfig
        ),
    }));
    Ok(())
}

/// Waits for node `name` to be Ready. After a restart of `restarted`, only a Ready condition that
/// the new kubelet reported counts: a heartbeat from before the unit's main process started is the
/// old kubelet's. The kubelet's own credentials may read its Node object, so this works on an
/// agent, which has no admin kubeconfig.
pub(crate) async fn wait_node_ready(
    kubectl: &Kubectl,
    name: &str,
    restarted: Option<&str>,
) -> Result<(), InstallError> {
    let since = match restarted {
        Some(unit) => Some(systemd::main_started(unit).await?),
        None => None,
    };
    kubectl::poll(
        Duration::from_mins(10),
        &format!("node {name} to be Ready"),
        async || {
            let node: Option<Node> = kubectl.get("node", name, None).await?;
            Ok::<_, InstallError>(node.as_ref().is_some_and(|n| ready_since(n, since)))
        },
    )
    .await
}

/// Whether the node reports Ready, with a heartbeat at or after `since` (seconds since the Unix
/// epoch) when given.
pub(crate) fn ready_since(node: &Node, since: Option<i64>) -> bool {
    node.status
        .iter()
        .flat_map(|s| s.conditions.iter().flatten())
        .any(|c| {
            c.type_ == "Ready"
                && c.status == "True"
                && since.is_none_or(|since| {
                    c.last_heartbeat_time
                        .as_ref()
                        .is_some_and(|beat| beat.0.as_second() >= since)
                })
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A node as `kubectl get node -o json` prints it, trimmed.
    const NODE: &str = include_str!("../fixtures/node.json");

    #[test]
    fn ready_only_after_the_restart() {
        let node: Node = serde_json::from_str(NODE).unwrap();
        let at = 1_792_742_430; // 2026-10-23T08:00:30Z
        assert!(ready_since(&node, None));
        assert!(ready_since(&node, Some(at)));
        assert!(!ready_since(&node, Some(at + 1)));
        let not_ready: Node = serde_json::from_str(&NODE.replace(
            r#""status": "True", "type": "Ready""#,
            r#""status": "Unknown", "type": "Ready""#,
        ))
        .unwrap();
        assert!(!ready_since(&not_ready, None));
        assert!(!ready_since(&Node::default(), None));
    }

    #[test]
    fn a_token_file_is_read_and_checked() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("token");
        let hash = "a".repeat(64);
        let token_text = format!("K10{hash}::abcdef.0123456789abcdef");
        std::fs::write(&path, format!("{token_text}\n")).unwrap();
        std::fs::set_permissions(
            &path,
            <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o600),
        )
        .unwrap();
        let config = JoinConfig::parse(&format!(
            "join:\n  server: https://10.0.0.5:6443\n  tokenFile: {}\n",
            path.display()
        ))
        .unwrap();
        assert_eq!(token(&config).unwrap(), token_text);
    }
}
