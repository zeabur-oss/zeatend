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

use crate::apply::{changed_files, check_host, kept_version, read_record, show_checks};
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

/// The plan, with the install record read from the cluster when it runs.
fn make_plan(config: &Config) -> Result<Plan, InstallError> {
    let running = systemd::is_active(UNIT);
    let mut facts = facts::read()?;
    facts.installer = kept_version(Path::new(STATE_DIR));
    if running {
        facts.record = read_record(&kubectl())?;
    }
    let mut plan = plan::plan(config, &facts)?;
    let outdated = binary_outdated(Path::new(BINARY), &release(&plan)?.version);
    let changed = changed_files(plan.host_files.iter());
    if let Some(changes) = &mut plan.changes {
        changes.set_host_files(changed);
        changes.restart |= outdated;
    }
    Ok(plan)
}

pub(crate) fn show_plan(config: &Config, json: bool) -> Result<(), InstallError> {
    let plan = make_plan(config)?;
    if json {
        return crate::print_json(&plan);
    }
    output::line(render::plan(&plan).trim_end());
    if show_checks(Node::KubeSolo, config.host.require_encrypted_disk) {
        output::line("apply stops until what failed is fixed.");
    }
    Ok(())
}

fn kubectl() -> Kubectl {
    Kubectl::new(KUBECTL, ADMIN_KUBECONFIG)
}

pub(crate) async fn apply(config: &Config) -> Result<(), InstallError> {
    let mut plan = make_plan(config)?;
    let running = systemd::is_active(UNIT);
    output::line(render::plan(&plan).trim_end());
    check_host(Node::KubeSolo, config.host.require_encrypted_disk)?;

    install_kubesolo(&plan).await?;
    let kubectl = kubectl();
    kubectl.wait_ready(Duration::from_mins(10))?;
    kubectl::poll(Duration::from_mins(10), "the node to be Ready", || {
        kubectl
            .wait(&["node", "--all", "--for=condition=Ready", "--timeout=10s"])
            .is_ok()
    })?;
    for (namespace, deployment) in [
        ("kube-system", "coredns"),
        ("local-path-storage", "local-path-provisioner"),
    ] {
        wait_available(&kubectl, namespace, deployment)?;
    }
    // A stopped cluster could not be read while planning: plan again now that it answers.
    if !running && read_record(&kubectl)?.is_some() {
        plan = make_plan(config)?;
        output::line(render::changes(plan.changes.as_ref(), UNIT).trim_end());
    }
    crate::apply::record(&kubectl, &plan)?;
    let name = plan.node_name.clone();
    write_kubeconfig(&name)?;

    output::step("Done");
    output::line(&done_text(&name, &facts::addresses()));
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

fn wait_available(kubectl: &Kubectl, namespace: &str, name: &str) -> Result<(), InstallError> {
    let target = format!("deployment/{name}");
    kubectl::poll(
        Duration::from_mins(10),
        &format!("{namespace}/{name}"),
        || {
            kubectl
                .wait(&[
                    "--namespace",
                    namespace,
                    &target,
                    "--for=condition=Available",
                    "--timeout=10s",
                ])
                .is_ok()
        },
    )?;
    Ok(())
}

/// Downloads `KubeSolo` and kubectl, writes the files and starts or restarts `KubeSolo`.
async fn install_kubesolo(plan: &Plan) -> Result<(), InstallError> {
    let solo = release(plan)?;
    output::step(&format!("KubeSolo {} ({UNIT})", solo.version));
    let agent = download::agent();
    let kubectl = Path::new(KUBECTL);
    output::err(&format!("    {}", solo.kubectl.url));
    download::fetch(
        &agent,
        &solo.kubectl.url,
        kubectl,
        solo.kubectl.sha256.as_deref(),
    )
    .await?;
    fs::set_permissions(kubectl, Permissions::from_mode(0o755)).map_err(io(kubectl))?;

    let mut changed = false;
    for file in &plan.host_files {
        changed |= files::write(Path::new(&file.path), file.mode, file.content.as_bytes())?;
    }
    crate::apply::load_apparmor_profile(plan.host_files.iter())?;
    if binary_outdated(Path::new(BINARY), &solo.version) {
        let download = &solo.download;
        let dir = PathBuf::from(STATE_DIR)
            .join("downloads")
            .join(&solo.version);
        output::err(&format!("    {}", download.url));
        download::fetch(
            &agent,
            &download.url,
            &dir.join(&download.file),
            download.sha256.as_deref(),
        )
        .await?;
        let unpacked = dir.join("unpacked");
        archive::unpack_tar_gz(&dir.join(&download.file), &unpacked)?;
        let source = unpacked.join("kubesolo");
        files::install(&source, Path::new(BINARY), 0o755)?;
        fs::remove_dir_all(&unpacked).map_err(io(&unpacked))?;
        changed = true;
    }
    if changed {
        systemd::daemon_reload()?;
    }
    if !systemd::is_active(UNIT) {
        output::err(&format!(
            "    starting {UNIT}; the first start takes a few minutes"
        ));
        systemd::enable_now(UNIT)?;
    } else if changed {
        output::err(&format!("    restarting {UNIT} for the new configuration"));
        systemd::restart(UNIT)?;
    }
    Ok(())
}

/// Whether the binary at `path` is missing or does not report `version`.
fn binary_outdated(path: &Path, version: &str) -> bool {
    !cmd::run(path, &["--version"], None)
        .unwrap_or_default()
        .contains(version)
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
    let config = zeatend_plan::kubesolo::kubeconfig(&admin, name, None)?;
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

    #[test]
    fn a_missing_binary_is_outdated() {
        let dir = tempfile::tempdir().unwrap();
        assert!(binary_outdated(&dir.path().join("kubesolo"), "v1.0.0"));
    }
}
