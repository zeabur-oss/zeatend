//! `apply` for `KubeSolo`: one node, with the distribution's own `CoreDNS` and local-path
//! storage. Each step checks what is already there, so `apply` runs again safely.

use std::fs::{self, Permissions};
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use zeatend_host::kubectl::Kubectl;
use zeatend_host::{HostError, archive, download, files};
use zeatend_plan::cluster::{Distribution, Layout};
use zeatend_plan::config::Config;
use zeatend_plan::kubesolo::{KUBECONFIG, Kubeconfig};
use zeatend_plan::plan::{KubeSoloPlan, Plan};
use zeatend_plan::precondition::Change;
use zeatend_plan::record::Record;
use zeatend_plan::telemetry::Step;

use crate::apply::{Converge, binary_outdated, file_sha256, write_record};
use crate::{InstallError, output};

const LAYOUT: &Layout = Distribution::KubeSolo.layout();
const UNIT: &str = LAYOUT.server_unit;
const BINARY: &str = LAYOUT.binary;
const STATE_DIR: &str = LAYOUT.state_dir;
const KUBECTL: &str = LAYOUT.kubectl;
const ADMIN_KUBECONFIG: &str = LAYOUT.admin_kubeconfig;

/// Adds the kubeconfig `run` writes from the admin kubeconfig to the plan's preconditions.
pub(crate) fn add_kubeconfig(plan: &mut Plan) -> Result<(), InstallError> {
    let kubeconfig = kubeconfig_yaml(&plan.node_name)?;
    plan.preconditions.generated.insert(
        KUBECONFIG.to_owned(),
        Change {
            found: file_sha256(Path::new(KUBECONFIG))?,
            leaves: kubeconfig.map(|yaml| download::sha256(yaml.as_bytes())),
        },
    );
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

/// Installs `KubeSolo` and waits until it is ready.
pub(crate) async fn run(
    config: &Config,
    plan: &Plan,
    solo: &KubeSoloPlan,
) -> Result<(), InstallError> {
    let restarted = install(plan, solo).await?;
    let messages = crate::i18n::messages();
    crate::telemetry::reached(Step::Node);
    output::step(&(messages.waiting_for_node)(&plan.node_name));
    let kubectl = kubectl();
    kubectl.wait_ready(Duration::from_mins(10)).await?;
    let name = plan.node_name.clone();
    crate::join::wait_node_ready(&kubectl, &name, restarted.then_some(UNIT)).await?;
    for (namespace, deployment) in [
        ("kube-system", "coredns"),
        ("local-path-storage", "local-path-provisioner"),
    ] {
        kubectl
            .rollout(
                namespace,
                &format!("deployment/{deployment}"),
                Duration::from_mins(10),
            )
            .await?;
    }
    write_record(
        Path::new(STATE_DIR),
        &Record::new(plan.distribution, plan.release.version()),
    )?;
    let yaml = kubeconfig_yaml(&name)?.ok_or_else(|| {
        InstallError::io(Path::new(ADMIN_KUBECONFIG))(std::io::ErrorKind::NotFound.into())
    })?;
    files::write(Path::new(KUBECONFIG), 0o600, yaml.as_bytes())?;

    output::step(messages.done);
    let host = crate::apply::reachable_host(&config.host.tls_san).await;
    output::line(&(messages.ready)(&crate::i18n::Ready {
        cluster: &format!("KubeSolo {}", plan.release.version()),
        node: &name,
        kubectl: &format!("sudo {KUBECTL} --kubeconfig {ADMIN_KUBECONFIG}"),
        kubeconfig: KUBECONFIG,
        host: host.as_deref(),
        rewrite: None,
        add_nodes: false,
    }));
    Ok(())
}

/// Downloads `KubeSolo` and kubectl, writes the files and starts or restarts `KubeSolo`. Returns
/// whether it started or restarted the unit.
async fn install(plan: &Plan, solo: &KubeSoloPlan) -> Result<bool, InstallError> {
    crate::telemetry::reached(Step::Install);
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
    fs::set_permissions(kubectl, Permissions::from_mode(0o755))
        .map_err(InstallError::io(kubectl))?;

    let outdated = binary_outdated(Path::new(BINARY), &solo.version).await;
    let converge = Converge {
        unit: UNIT,
        state_dir: STATE_DIR,
        files: &plan.host_files,
        changed: outdated,
    };
    converge
        .run(async || {
            if !outdated {
                return Ok(());
            }
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
            files::install(&unpacked.join("kubesolo"), Path::new(BINARY), 0o755)?;
            fs::remove_dir_all(&unpacked).map_err(InstallError::io(&unpacked))?;
            Ok(())
        })
        .await
}

/// The kubeconfig `apply` writes to [`KUBECONFIG`] from the admin kubeconfig; `None` before
/// `KubeSolo` made that.
fn kubeconfig_yaml(name: &str) -> Result<Option<String>, InstallError> {
    let path = Path::new(ADMIN_KUBECONFIG);
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(InstallError::io(path)(e)),
    };
    let admin: Kubeconfig = serde_saphyr::from_str(&text).map_err(|e| HostError::Parse {
        what: ADMIN_KUBECONFIG.to_owned(),
        reason: e.to_string(),
    })?;
    let config = zeatend_plan::kubesolo::kubeconfig(&admin, name)?;
    let yaml = serde_saphyr::to_string(&config).map_err(|e| InstallError::Serialize {
        what: KUBECONFIG,
        reason: e.to_string(),
    })?;
    Ok(Some(yaml))
}
