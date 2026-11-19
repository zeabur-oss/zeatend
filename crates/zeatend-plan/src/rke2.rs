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

use crate::cluster::{
    AUDIT_POLICY, HEADER, HostFile, Options, Rancher, control_plane_requests, disable_list,
    yaml_list,
};
use crate::config::Component;
use crate::tuning::HostTuning;

/// Where the tarball installs.
pub const PREFIX: &str = "/usr/local";

/// Where RKE2's deploy controller reads the manifests it applies on a server.
pub const MANIFESTS_DIR: &str = "/var/lib/rancher/rke2/server/manifests";

const RKE2: Rancher = Rancher::Rke2;

/// The files to write, in the order to write them.
#[must_use]
pub fn host_files(options: &Options) -> Vec<HostFile> {
    let dir = RKE2.layout().config_dir;
    vec![
        RKE2.sysctl(),
        HostFile {
            path: format!("{dir}/audit-policy.yaml"),
            mode: 0o600,
            content: AUDIT_POLICY.to_owned(),
        },
        HostFile {
            path: format!("{dir}/config.yaml"),
            mode: 0o600,
            content: config(options),
        },
    ]
}

/// The files of a worker: the kernel parameters and an agent `config.yaml` with the server's
/// kubelet settings. The agent takes no server keys, so this is not the server's file.
#[must_use]
pub fn agent_files(tuning: &HostTuning) -> Vec<HostFile> {
    vec![
        RKE2.sysctl(),
        HostFile {
            path: format!("{}/config.yaml", RKE2.layout().config_dir),
            mode: 0o600,
            content: format!(
                "{HEADER}profile: cis\n{}",
                Rancher::kubelet_args(tuning, false)
            ),
        },
    ]
}

fn config(options: &Options) -> String {
    let mut out = String::from(HEADER);
    out.push_str(
        "profile: cis\n\
         write-kubeconfig-mode: \"0600\"\n\
         enable-servicelb: true\n\
         audit-policy-file: /etc/rancher/rke2/audit-policy.yaml\n",
    );
    out.push_str(&Rancher::kubelet_args(&options.tuning, true));
    out.push_str(&control_plane_requests());
    // RKE2 takes Traefik off with `ingress-controller`, not with `disable`.
    let traefik = Component::Traefik.name();
    if options.off.contains(&Component::Traefik) || options.disable.iter().any(|i| i == traefik) {
        out.push_str("ingress-controller: none\n");
    }
    let off: Vec<&str> = options
        .off
        .iter()
        .filter_map(|c| match c {
            Component::MetricsServer => Some("rke2-metrics-server"),
            Component::Traefik | Component::LocalStorage => None,
        })
        .collect();
    let kept: Vec<String> = options
        .disable
        .iter()
        .filter(|i| *i != traefik)
        .cloned()
        .collect();
    disable_list(&mut out, &off, &kept);
    yaml_list(&mut out, "tls-san", &options.tls_san);
    out
}

/// The `etcd` user the cis profile runs etcd as. It must be a local user in /etc/passwd.
pub const ETCD_USER: &[&str] = &[
    "useradd",
    "-r",
    "-c",
    "etcd user",
    "-s",
    "/sbin/nologin",
    "-M",
    "etcd",
    "-U",
];

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
            off: Vec::new(),
            disable: Vec::new(),
        }
    }

    #[test]
    fn components_an_earlier_install_turned_off_stay_off() {
        let config = |disable: Vec<String>| -> serde_json::Value {
            let files = host_files(&Options {
                disable,
                ..options(Vec::new())
            });
            let file = files
                .iter()
                .find(|f| f.path == "/etc/rancher/rke2/config.yaml")
                .unwrap();
            serde_saphyr::from_str(&file.content).unwrap()
        };
        assert!(config(Vec::new()).get("disable").is_none());
        assert_eq!(
            config(vec!["rke2-metrics-server".to_owned()])["disable"],
            serde_json::json!(["rke2-metrics-server"])
        );
    }

    #[test]
    fn kubelet_serves_only_the_allowed_ciphers_and_closes_idle_streams() {
        let options = options(Vec::new());
        let server = host_files(&options);
        for content in [&server[2].content, &agent_files(&tuning())[1].content] {
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
        let files = host_files(&options);
        for file in files.iter().filter(|f| {
            !f.path.starts_with(MANIFESTS_DIR)
                && std::path::Path::new(&f.path)
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("yaml"))
        }) {
            let value: serde_json::Value = serde_saphyr::from_str(&file.content).unwrap();
            assert!(value.is_object(), "{}", file.path);
        }
        assert!(files[2].content.contains("tls-san:\n  - k8s.example.com\n"));
        assert!(files[2].content.contains("enable-servicelb: true\n"));
    }

    #[test]
    fn storage_is_left_to_the_zeabur_catalog() {
        let files = host_files(&options(Vec::new()));
        assert!(files.iter().all(|f| !f.path.starts_with(MANIFESTS_DIR)));
        assert!(
            agent_files(&tuning())
                .iter()
                .all(|f| !f.path.starts_with(MANIFESTS_DIR))
        );
        // RKE2's default ingress controller, Traefik, runs.
        assert!(!config(&options(Vec::new())).contains("ingress-controller"));
    }

    #[test]
    fn agents_keep_the_kubelet_settings_without_server_keys() {
        let server = host_files(&options(Vec::new()));
        let agent = agent_files(&tuning());
        assert_eq!(agent[0], server[0]);
        assert!(agent[1].content.contains("profile: cis\n"));
        assert!(agent[1].content.contains("pod-max-pids=4096"));
        assert!(!agent[1].content.contains("audit-policy-file"));
    }

    #[test]
    fn workers_reserve_the_os_and_servers_also_ask_less_for_etcd() {
        let server = host_files(&options(Vec::new()));
        let server: serde_json::Value = serde_saphyr::from_str(&server[2].content).unwrap();
        assert_eq!(
            server["control-plane-resource-requests"],
            serde_json::json!(["etcd-memory=256Mi", "kube-controller-manager-memory=128Mi"])
        );
        let agent: serde_json::Value =
            serde_saphyr::from_str(&agent_files(&tuning())[1].content).unwrap();
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
    fn the_configuration_turns_off_the_components_it_names() {
        let config = |off: Vec<Component>, disable: Vec<String>| -> serde_json::Value {
            serde_saphyr::from_str(&config(&Options {
                off,
                disable,
                ..options(Vec::new())
            }))
            .unwrap()
        };
        let off = config(
            vec![Component::Traefik, Component::MetricsServer],
            Vec::new(),
        );
        assert_eq!(off["ingress-controller"], "none");
        assert_eq!(off["disable"], serde_json::json!(["rke2-metrics-server"]));
        // An earlier install with `ingress-controller: none` keeps it.
        let kept = crate::cluster::disabled_components("ingress-controller: none\n");
        assert_eq!(kept, ["traefik"]);
        let earlier = config(Vec::new(), kept);
        assert_eq!(earlier["ingress-controller"], "none");
        assert!(earlier.get("disable").is_none());
    }
}
