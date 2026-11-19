//! `plan` and `apply` for `KubeSolo`: one node, with the distribution's own `CoreDNS` and
//! local-path storage. Each step checks what is already there, so `apply` runs again safely.

use std::fs::{self, Permissions};
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;
use zeatend_host::kubectl::{self, Kubectl};
use zeatend_host::preflight::Node;
use zeatend_host::{archive, cmd, download, facts, files, systemd};
use zeatend_plan::config::Config;
use zeatend_plan::kubesolo::{ADMIN_KUBECONFIG, BINARY, KUBECONFIG, KUBECTL, STATE_DIR, UNIT};
use zeatend_plan::plan::{self, KubeSoloPlan, Plan, Release};
use zeatend_plan::record::Record;
use zeatend_plan::upgrade::release_tag;

use crate::apply::{
    changed_files, check_host, owe_restart, restart_owed, restarted, show_checks, write_record,
};
use crate::{InstallError, output, render};

/// The `KubeSolo` release of a plan.
fn release(plan: &Plan) -> Result<&KubeSoloPlan, InstallError> {
    match &plan.release {
        Release::KubeSolo(solo) => Ok(solo),
        Release::Cluster(_) => Err(InstallError::Message(
            "the plan installs K3s or RKE2, not KubeSolo".to_owned(),
        )),
    }
}

/// The plan, with the install record read from the host.
async fn make_plan(config: &Config) -> Result<Plan, InstallError> {
    let mut facts = facts::read().await?;
    facts.record = crate::installer::read_record(config.distribution).await?;
    if facts.record.is_some() {
        facts.kubernetes = crate::apply::running_kubernetes(&kubectl()).await;
    }
    let mut plan = plan::plan(config, &facts)?;
    let outdated = binary_outdated(Path::new(BINARY), &release(&plan)?.version).await;
    let changed = changed_files(plan.host_files.iter());
    if let Some(changes) = &mut plan.changes {
        changes.set_host_files(changed);
        changes.restart |= outdated || restart_owed(STATE_DIR);
    }
    Ok(plan)
}

pub(crate) async fn show_plan(config: &Config, json: bool) -> Result<(), InstallError> {
    let plan = make_plan(config).await?;
    if json {
        return crate::print_json(&plan);
    }
    output::line(render::plan(&plan).trim_end());
    if show_checks(Node::KubeSolo, config.host.require_encrypted_disk).await {
        output::line(crate::i18n::messages().apply_stops);
    }
    Ok(())
}

/// `KubeSolo`'s admin `kubectl`.
/// zeatend's kubectl with the admin kubeconfig. Before zeatend's first `apply` on a host that
/// zeabur-installer installed, the installer's kubectl.
pub(crate) fn kubectl() -> Kubectl {
    let installer = Path::new(zeatend_plan::installer::KUBESOLO_KUBECTL);
    if !Path::new(KUBECTL).exists() && installer.exists() {
        return Kubectl::new(installer, ADMIN_KUBECONFIG);
    }
    Kubectl::new(KUBECTL, ADMIN_KUBECONFIG)
}

pub(crate) async fn apply(config: &Config) -> Result<(), InstallError> {
    let plan = make_plan(config).await?;
    output::line(render::plan(&plan).trim_end());
    check_host(Node::KubeSolo, config.host.require_encrypted_disk).await?;

    install_kubesolo(&plan).await?;
    let kubectl = kubectl();
    kubectl.wait_ready(Duration::from_mins(10)).await?;
    kubectl::poll(
        Duration::from_mins(10),
        "the node to be Ready",
        async || {
            kubectl
                .wait(&["node", "--all", "--for=condition=Ready", "--timeout=10s"])
                .await
                .is_ok()
        },
    )
    .await?;
    for (namespace, deployment) in [
        ("kube-system", "coredns"),
        ("local-path-storage", "local-path-provisioner"),
    ] {
        wait_available(&kubectl, namespace, deployment).await?;
    }
    write_record(
        Path::new(STATE_DIR),
        &Record::new(plan.distribution, plan.release.version()),
    )?;
    let name = plan.node_name.clone();
    write_kubeconfig(&name)?;

    output::step("Done");
    output::line(&done_text(&name, &facts::addresses().await));
    Ok(())
}

/// What to run on the person's own computer.
fn done_text(name: &str, addresses: &[String]) -> String {
    let host = addresses.first().map_or("<this server>", String::as_str);
    format!(
        "This server runs KubeSolo. On your own computer:\n\
         \n  ssh <you>@{host} sudo cat {KUBECONFIG} > ~/.kube/{name}.yaml\n  \
         KUBECONFIG=~/.kube/{name}.yaml kubectl get nodes\n\n\
         Whoever has this file is cluster admin; keep it like an SSH key. If your computer \
         reaches the server at another address, change `server:` in the file and add the \
         address to host.tlsSan."
    )
}

async fn wait_available(
    kubectl: &Kubectl,
    namespace: &str,
    name: &str,
) -> Result<(), InstallError> {
    let target = format!("deployment/{name}");
    kubectl::poll(
        Duration::from_mins(10),
        &format!("{namespace}/{name}"),
        async || {
            kubectl
                .wait(&[
                    "--namespace",
                    namespace,
                    &target,
                    "--for=condition=Available",
                    "--timeout=10s",
                ])
                .await
                .is_ok()
        },
    )
    .await?;
    Ok(())
}

/// Downloads `KubeSolo` and kubectl, writes the files and starts or restarts `KubeSolo`.
async fn install_kubesolo(plan: &Plan) -> Result<(), InstallError> {
    let solo = release(plan)?;
    output::step(&format!("KubeSolo {} ({UNIT})", solo.version));
    let agent = download::agent();
    let kubectl = Path::new(KUBECTL);
    output::note(&solo.kubectl.url);
    download::fetch(
        &agent,
        &solo.kubectl.url,
        kubectl,
        solo.kubectl.sha256.as_deref(),
        &mut output::Meter,
    )
    .await?;
    fs::set_permissions(kubectl, Permissions::from_mode(0o755)).map_err(io(kubectl))?;

    let outdated = binary_outdated(Path::new(BINARY), &solo.version).await;
    let changed =
        outdated || restart_owed(STATE_DIR) || !changed_files(plan.host_files.iter()).is_empty();
    if changed {
        owe_restart(STATE_DIR)?;
    }
    for file in &plan.host_files {
        files::write(Path::new(&file.path), file.mode, file.content.as_bytes())?;
    }
    crate::apply::load_apparmor_profile(plan.host_files.iter()).await?;
    if outdated {
        let download = &solo.download;
        let dir = PathBuf::from(STATE_DIR)
            .join("downloads")
            .join(&solo.version);
        output::note(&download.url);
        download::fetch(
            &agent,
            &download.url,
            &dir.join(&download.file),
            download.sha256.as_deref(),
            &mut output::Meter,
        )
        .await?;
        let unpacked = dir.join("unpacked");
        archive::unpack_tar_gz(&dir.join(&download.file), &unpacked)?;
        let source = unpacked.join("kubesolo");
        files::install(&source, Path::new(BINARY), 0o755)?;
        fs::remove_dir_all(&unpacked).map_err(io(&unpacked))?;
    }
    if changed {
        systemd::daemon_reload().await?;
    }
    if !systemd::is_active(UNIT).await {
        output::note(&format!(
            "starting {UNIT}; the first start takes a few minutes"
        ));
        systemd::enable_now(UNIT).await?;
    } else if changed {
        output::note(&format!("restarting {UNIT} for the new configuration"));
        systemd::restart(UNIT).await?;
    }
    restarted(STATE_DIR)?;
    Ok(())
}

/// Whether the binary at `path` is missing or reports another release than `version`.
async fn binary_outdated(path: &Path, version: &str) -> bool {
    release_tag(&cmd::version_output(path).await).as_deref() != Some(version)
}

fn io(path: &Path) -> impl Fn(std::io::Error) -> InstallError {
    let path = path.display().to_string();
    move |e| InstallError::Message(format!("{path}: {e}"))
}

/// Writes the kubeconfig for the person's computer, readable by root only.
fn write_kubeconfig(name: &str) -> Result<(), InstallError> {
    let path = Path::new(ADMIN_KUBECONFIG);
    let text = fs::read_to_string(path).map_err(io(path))?;
    let admin: Value = serde_saphyr::from_str(&text)
        .map_err(|e| InstallError::Message(format!("{ADMIN_KUBECONFIG}: {e}")))?;
    let config = zeatend_plan::kubesolo::kubeconfig(&admin, name)?;
    let yaml =
        serde_saphyr::to_string(&config).map_err(|e| InstallError::Message(e.to_string()))?;
    files::write(Path::new(KUBECONFIG), 0o600, yaml.as_bytes())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn done_text_names_the_address_and_the_kubeconfig() {
        let text = done_text("solo", &["203.0.113.7".to_owned()]);
        assert!(
            text.contains(&format!("ssh <you>@203.0.113.7 sudo cat {KUBECONFIG}")),
            "{text}"
        );
        assert!(text.contains("~/.kube/solo.yaml"), "{text}");
        assert!(done_text("solo", &[]).contains("<this server>"));
    }

    #[tokio::test]
    async fn a_missing_binary_is_outdated() {
        let dir = tempfile::tempdir().unwrap();
        assert!(binary_outdated(&dir.path().join("kubesolo"), "v1.0.0").await);
    }
}
