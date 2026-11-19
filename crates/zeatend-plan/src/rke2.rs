//! Host files written before RKE2 first starts.
//!
//! RKE2's `cis` profile already does most of the CIS benchmark: Secrets are encrypted at rest,
//! Pod Security `restricted` is the default outside `kube-system`, `NetworkPolicies` guard the
//! system namespaces and the service-account and kubelet flags are set
//! (<https://docs.rke2.io/security/hardening_guide>). The default applies only to a namespace
//! without its own Pod Security labels. What it leaves to the host is written here: the kernel
//! parameters it refuses to start without and an audit policy (RKE2's own logs nothing).
//!
//! Each key here is set in exactly one file, so a person's own `config.yaml.d/90-*.yaml`
//! overrides it cleanly.

use crate::PlanError;
use crate::cluster::{
    AUDIT_POLICY, Distribution, HostFile, Options, Rancher, RancherConfig, control_plane_requests,
};
use crate::tuning::HostTuning;

/// Where the tarball installs.
pub const PREFIX: &str = "/usr/local";

/// Where RKE2's deploy controller reads the manifests it applies on a server.
pub const MANIFESTS_DIR: &str = "/var/lib/rancher/rke2/server/manifests";

const RKE2: Rancher = Rancher::Rke2;

/// The files to write, in the order to write them.
///
/// # Errors
///
/// When a file cannot be serialised.
pub fn host_files(options: &Options) -> Result<Vec<HostFile>, PlanError> {
    let dir = RKE2.layout().config_dir;
    Ok(vec![
        RKE2.sysctl(),
        HostFile {
            path: format!("{dir}/audit-policy.yaml"),
            mode: 0o600,
            content: AUDIT_POLICY.to_owned(),
        },
        config(options).file(format!("{dir}/config.yaml"))?,
    ])
}

/// The files of a worker: the kernel parameters and an agent `config.yaml` with the server's
/// kubelet settings. The agent takes no server keys, so this is not the server's file.
///
/// # Errors
///
/// When a file cannot be serialised.
pub fn agent_files(tuning: &HostTuning) -> Result<Vec<HostFile>, PlanError> {
    let config = RancherConfig {
        profile: Some("cis"),
        kubelet_arg: Rancher::kubelet_args(tuning, false),
        ..RancherConfig::default()
    };
    Ok(vec![
        RKE2.sysctl(),
        config.file(format!("{}/config.yaml", RKE2.layout().config_dir))?,
    ])
}

fn config(options: &Options) -> RancherConfig {
    let mut config = RancherConfig {
        profile: Some("cis"),
        write_kubeconfig_mode: Some("0600"),
        enable_servicelb: Some(true),
        audit_policy_file: Some(format!("{}/audit-policy.yaml", RKE2.layout().config_dir)),
        kubelet_arg: Rancher::kubelet_args(&options.tuning, true),
        control_plane_resource_requests: control_plane_requests(),
        tls_san: options.tls_san.clone(),
        ..RancherConfig::default()
    };
    config.turn_off(Distribution::Rke2, &options.disable);
    config
}

/// The command and its arguments that add the `etcd` user the cis profile runs etcd as. It must
/// be a local user in /etc/passwd.
pub const ETCD_USER: (&str, &[&str]) = (
    "useradd",
    &[
        "-r",
        "-c",
        "etcd user",
        "-s",
        "/sbin/nologin",
        "-M",
        "etcd",
        "-U",
    ],
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cluster::Distribution;
    use crate::tuning::MemoryTier;

    fn tuning() -> HostTuning {
        HostTuning::new(Distribution::Rke2, MemoryTier::Standard)
    }

    fn options(tls_san: Vec<String>) -> Options {
        Options {
            tls_san,
            tuning: tuning(),
            own_network: false,
            disable: Vec::new(),
        }
    }

    #[test]
    fn kubelet_serves_only_the_allowed_ciphers_and_closes_idle_streams() {
        let options = options(Vec::new());
        let server = host_files(&options).unwrap();
        for content in [
            &server[2].content,
            &agent_files(&tuning()).unwrap()[1].content,
        ] {
            let parsed: serde_json::Value = serde_saphyr::from_str(content).unwrap();
            let args: Vec<&str> = parsed["kubelet-arg"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|a| a.as_str())
                .collect();
            assert!(
                args.contains(&"streaming-connection-idle-timeout=5m"),
                "{args:?}"
            );
            let ciphers = args
                .iter()
                .find_map(|a| a.strip_prefix("tls-cipher-suites="))
                .unwrap();
            assert!(
                ciphers.split(',').all(|c| c.contains("_ECDHE_")),
                "{ciphers}"
            );
            assert!(ciphers.contains("TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256"));
        }
    }

    #[test]
    fn the_server_files_parse_as_yaml() {
        let options = options(vec!["k8s.example.com".to_owned()]);
        let files = host_files(&options).unwrap();
        for file in files.iter().filter(|f| {
            !f.path.starts_with(MANIFESTS_DIR)
                && std::path::Path::new(&f.path)
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("yaml"))
        }) {
            let value: serde_json::Value = serde_saphyr::from_str(&file.content).unwrap();
            assert!(value.is_object(), "{}", file.path);
        }
        let config: serde_json::Value = serde_saphyr::from_str(&files[2].content).unwrap();
        assert_eq!(config["tls-san"], serde_json::json!(["k8s.example.com"]));
        assert_eq!(config["enable-servicelb"], true);
        assert_eq!(config["write-kubeconfig-mode"], "0600");
    }

    #[test]
    fn storage_is_left_to_the_zeabur_catalog() {
        let files = host_files(&options(Vec::new())).unwrap();
        assert!(files.iter().all(|f| !f.path.starts_with(MANIFESTS_DIR)));
        assert!(
            agent_files(&tuning())
                .unwrap()
                .iter()
                .all(|f| !f.path.starts_with(MANIFESTS_DIR))
        );
        // RKE2's default ingress controller, Traefik, runs.
        assert!(
            !config(&options(Vec::new()))
                .file(String::new())
                .unwrap()
                .content
                .contains("ingress-controller")
        );
    }

    #[test]
    fn agents_keep_the_kubelet_settings_without_server_keys() {
        let server = host_files(&options(Vec::new())).unwrap();
        let agent = agent_files(&tuning()).unwrap();
        assert_eq!(agent[0], server[0]);
        let config: serde_json::Value = serde_saphyr::from_str(&agent[1].content).unwrap();
        assert_eq!(config["profile"], "cis");
        assert!(agent[1].content.contains("pod-max-pids=4096"));
        assert!(!agent[1].content.contains("audit-policy-file"));
    }

    #[test]
    fn workers_reserve_the_os_and_servers_also_ask_less_for_etcd() {
        let server = host_files(&options(Vec::new())).unwrap();
        let server: serde_json::Value = serde_saphyr::from_str(&server[2].content).unwrap();
        assert_eq!(
            server["control-plane-resource-requests"],
            serde_json::json!(["etcd-memory=256Mi", "kube-controller-manager-memory=128Mi"])
        );
        let agent: serde_json::Value =
            serde_saphyr::from_str(&agent_files(&tuning()).unwrap()[1].content).unwrap();
        let args = agent["kubelet-arg"].as_array().unwrap();
        assert!(args.contains(&serde_json::json!("system-reserved=memory=350Mi")));
        assert!(args.contains(&serde_json::json!(
            "eviction-hard=memory.available<200Mi,nodefs.available<5%,imagefs.available<5%"
        )));
        assert!(
            !args
                .iter()
                .any(|a| a.as_str().unwrap().starts_with("kube-reserved="))
        );
        assert!(agent.get("control-plane-resource-requests").is_none());
    }

    #[test]
    fn disable_turns_off_the_ingress_controller_and_metrics_server() {
        let parsed: serde_json::Value = serde_saphyr::from_str(
            &config(&Options {
                disable: vec![
                    crate::config::Component::Traefik,
                    crate::config::Component::MetricsServer,
                ],
                ..options(Vec::new())
            })
            .file(String::new())
            .unwrap()
            .content,
        )
        .unwrap();
        assert_eq!(parsed["ingress-controller"], serde_json::json!(["none"]));
        assert_eq!(
            parsed["disable"],
            serde_json::json!(["rke2-metrics-server"])
        );
        let plain: serde_json::Value = serde_saphyr::from_str(
            &config(&options(Vec::new()))
                .file(String::new())
                .unwrap()
                .content,
        )
        .unwrap();
        assert!(plain.get("ingress-controller").is_none());
        assert!(plain.get("disable").is_none());
    }
}
