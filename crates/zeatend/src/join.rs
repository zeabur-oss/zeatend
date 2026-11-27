//! `plan` and `apply` on a node that joins an existing K3s or RKE2 cluster.

use std::path::Path;
use std::time::Duration;

use k8s_openapi::api::core::v1::Node;
use zeatend_host::kubectl::{self, Kubectl};
use zeatend_host::{facts, files, longhorn, network, systemd};
use zeatend_plan::cluster::{HostFile, Rancher};
use zeatend_plan::config::{self, JoinConfig, Role};
use zeatend_plan::finding::Status;
use zeatend_plan::join::{self, JoinFacts, JoinPlan, RTT_SAMPLES};
use zeatend_plan::precondition::Preconditions;
use zeatend_plan::record::Record;
use zeatend_plan::steps::Step;
use zeatend_plan::telemetry::{Action, Command};
use zeatend_plan::token::JoinToken;

use crate::apply::{self, ClusterInstall};
use crate::{InstallError, output, render, telemetry};

/// The join token, from the configuration or its file, checked against the role.
/// [`JoinConfig::check`] checked a token in the configuration already.
fn token(config: &JoinConfig) -> Result<String, InstallError> {
    match (&config.join.token, &config.join.token_file) {
        (Some(token), _) => Ok(token.trim().to_owned()),
        (None, Some(file)) => {
            let token = String::from_utf8(files::read_secret(Path::new(file))?)
                .map_err(|_e| InstallError::NotText(file.into()))?;
            let token = token.trim().to_owned();
            config::check_token(&token, config.join.role)?;
            Ok(token)
        }
        (None, None) => Err(InstallError::NoToken),
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
    let unit = distribution.distribution().layout().unit(config.join.role);
    let mut facts = facts::read(config.network.interface.as_deref()).await?;
    if config.storage.longhorn {
        facts.longhorn = Some(longhorn::read().await);
    }
    crate::registry::read(
        &mut facts,
        distribution.into(),
        config.join.role,
        &config.registry_hosts,
    )?;
    facts.record = crate::installer::read_record(distribution.into()).await?;
    if facts.record.is_some() {
        facts.kubernetes = apply::running_kubernetes(&own_kubectl(distribution)).await;
    }
    Ok(JoinFacts {
        facts,
        rtt_us,
        ports,
        token_pinned: JoinToken::parse(token).is_ok_and(|t| t.pins_ca()),
        unit_running: systemd::is_active(unit).await,
    })
}

async fn make_plan(config: &JoinConfig) -> Result<(JoinPlan, String), InstallError> {
    let token = token(config)?;
    let facts = read_facts(config, &token).await?;
    let mut plan = join::plan(config, &facts)?;
    let distribution = plan.cluster.distribution;
    plan.preconditions = apply::preconditions(
        &apply::written_files(&plan.host_files, plan.longhorn.as_ref()),
        &plan.removed_files,
        Path::new(distribution.distribution().layout().binary),
        plan.facts.record.as_ref(),
        &Record::new(distribution.into(), &plan.cluster.version),
    )
    .await?;
    Ok((plan, token))
}

/// The node's own `kubectl`, with the kubelet's credentials, which an agent also has.
pub(crate) fn own_kubectl(distribution: Rancher) -> Kubectl {
    Kubectl::new(
        distribution.distribution().layout().kubectl,
        distribution.paths().kubelet_kubeconfig,
    )
}

pub(crate) async fn show_plan(config: &JoinConfig, json: bool) -> Result<(), InstallError> {
    let run = telemetry::run(config.distribution, config.join.role, Action::Join);
    let made = make_plan(config).await;
    telemetry::planned(run.clone(), &made);
    let (plan, _) = made?;
    if json {
        return crate::print_json(&plan);
    }
    output::line(render::join_plan(&plan).trim_end());
    output::step(crate::i18n::messages().checking_host);
    let failed = apply::show_checks(
        config.distribution,
        plan.role,
        config.host.require_encrypted_disk,
        config.network.interface.as_deref(),
    )
    .await;
    if !failed.is_empty() {
        telemetry::checks_failed(run, Command::Plan, &failed);
        output::line(crate::i18n::messages().apply_stops);
    }
    Ok(())
}

/// Joins this node. With `reviewed`, the preconditions of a reviewed plan, it stops unless the
/// plan made now still meets them.
pub(crate) async fn apply(
    config: &JoinConfig,
    reviewed: Option<&Preconditions>,
) -> Result<(), InstallError> {
    let (plan, token) = make_plan(config).await?;
    output::line(render::join_plan(&plan).trim_end());
    apply::check_reviewed(reviewed, &plan.preconditions)?;
    if let Some(blocker) = plan.findings.iter().find(|c| c.status == Status::Fail) {
        return Err(InstallError::Blocked(blocker.finding.clone()));
    }
    let run = telemetry::run(config.distribution, plan.role, Action::Join);
    let mut steps = apply::Steps::new(Step::join(plan.longhorn.is_some()));
    steps.start(Step::CheckHost)?;
    apply::check_host(
        config.distribution,
        plan.role,
        config.host.require_encrypted_disk,
        config.network.interface.as_deref(),
        &run,
    )
    .await?;
    telemetry::started(run);
    apply::prepare_longhorn(plan.longhorn.as_ref(), &mut steps).await?;

    let distribution = plan.cluster.distribution;
    let layout = distribution.distribution().layout();
    let mut host_files = plan.host_files.clone();
    host_files.push(HostFile {
        path: distribution.paths().join_token_file.to_owned(),
        mode: 0o600,
        content: format!("{token}\n"),
    });
    let install = ClusterInstall {
        cluster: &plan.cluster,
        files: &host_files,
        removed: &plan.removed_files,
        server: plan.role == Role::Server,
    };
    let restarted = apply::install_cluster(&install, &mut steps).await?;

    let name = plan.node_name.clone();
    let messages = crate::i18n::messages();
    steps.start(Step::WaitReady)?;
    output::note(&(messages.waiting_for_node)(&name));
    wait_node_ready(
        &own_kubectl(distribution),
        &name,
        restarted.then(|| install.unit()),
    )
    .await?;
    if let Some(iface) = &plan.canal_iface {
        apply::roll_canal(&Kubectl::admin(distribution.distribution()), iface).await?;
    }
    steps.start(Step::WriteRecord)?;
    apply::write_record(
        Path::new(layout.state_dir),
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
