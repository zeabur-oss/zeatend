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
    AUDIT_POLICY, HEADER, HostFile, Options, Rancher, control_plane_requests, yaml_list,
};

/// Where the tarball installs.
pub const PREFIX: &str = "/usr/local";

/// Where RKE2's deploy controller reads the manifests it applies on a server.
pub const MANIFESTS_DIR: &str = "/var/lib/rancher/rke2/server/manifests";

/// Where the local-path-provisioner of [`LOCAL_PATH`] keeps volumes.
pub const STORAGE_DIR: &str = "/var/lib/zeatend/storage";

/// rancher's local-path-provisioner as the default storage class: RKE2 ships none. Its volumes
/// are under [`STORAGE_DIR`].
pub const LOCAL_PATH: &str = include_str!("local-path-storage.yaml");

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
        HostFile {
            path: format!("{MANIFESTS_DIR}/zeatend-local-path.yaml"),
            mode: 0o600,
            content: LOCAL_PATH.to_owned(),
        },
    ]
}

/// The files of a worker: the kernel parameters and an agent `config.yaml` with the server's
/// kubelet settings. The agent takes no server keys, so this is not the server's file.
#[must_use]
pub fn agent_files() -> Vec<HostFile> {
    vec![
        RKE2.sysctl(),
        HostFile {
            path: format!("{}/config.yaml", RKE2.layout().config_dir),
            mode: 0o600,
            content: format!("{HEADER}profile: cis\n{}", RKE2.kubelet_args(false)),
        },
    ]
}

fn config(options: &Options) -> String {
    let mut out = String::from(HEADER);
    out.push_str(
        "profile: cis\n\
         write-kubeconfig-mode: \"0600\"\n\
         # No packaged ingress; ServiceLB gives the one you install ports 80 and 443.\n\
         ingress-controller: none\n\
         enable-servicelb: true\n\
         audit-policy-file: /etc/rancher/rke2/audit-policy.yaml\n",
    );
    out.push_str(&RKE2.kubelet_args(true));
    out.push_str(&control_plane_requests());
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

    #[test]
    fn kubelet_serves_only_the_allowed_ciphers_and_closes_idle_streams() {
        let options = Options { tls_san: vec![] };
        let server = host_files(&options);
        for content in [&server[2].content, &agent_files()[1].content] {
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
        let options = Options {
            tls_san: vec!["k8s.example.com".to_owned()],
        };
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
    fn a_server_gets_local_path_as_the_default_class() {
        let files = host_files(&Options {
            tls_san: Vec::new(),
        });
        let manifest = files
            .iter()
            .find(|f| f.path == "/var/lib/rancher/rke2/server/manifests/zeatend-local-path.yaml")
            .unwrap();
        let objects: Vec<serde_json::Value> = manifest
            .content
            .split("\n---\n")
            .map(|doc| serde_saphyr::from_str(doc).unwrap())
            .collect();
        let class = objects
            .iter()
            .find(|o| o["kind"] == "StorageClass")
            .unwrap();
        assert_eq!(class["metadata"]["name"], "local-path");
        assert_eq!(
            class["metadata"]["annotations"]["storageclass.kubernetes.io/is-default-class"],
            "true"
        );
        assert_eq!(class["volumeBindingMode"], "WaitForFirstConsumer");
        assert_eq!(class["reclaimPolicy"], "Delete");
        let namespace = objects.iter().find(|o| o["kind"] == "Namespace").unwrap();
        assert_eq!(
            namespace["metadata"]["labels"]["app.kubernetes.io/managed-by"],
            "zeatend"
        );
        let paths_config = objects.iter().find(|o| o["kind"] == "ConfigMap").unwrap();
        let paths: serde_json::Value =
            serde_json::from_str(paths_config["data"]["config.json"].as_str().unwrap()).unwrap();
        assert_eq!(
            paths["nodePathMap"][0]["paths"],
            serde_json::json!([STORAGE_DIR])
        );
        assert!(!manifest.content.contains("zeabur"));
        assert!(
            agent_files()
                .iter()
                .all(|f| !f.path.starts_with(MANIFESTS_DIR))
        );
        assert!(
            config(&Options {
                tls_san: Vec::new()
            })
            .contains("ingress-controller: none\n")
        );
    }

    #[test]
    fn agents_keep_the_kubelet_settings_without_server_keys() {
        let server = host_files(&Options { tls_san: vec![] });
        let agent = agent_files();
        assert_eq!(agent[0], server[0]);
        assert!(agent[1].content.contains("profile: cis\n"));
        assert!(agent[1].content.contains("pod-max-pids=4096"));
        assert!(!agent[1].content.contains("audit-policy-file"));
    }

    #[test]
    fn workers_reserve_the_os_and_servers_also_ask_less_for_etcd() {
        let server = host_files(&Options { tls_san: vec![] });
        let server: serde_json::Value = serde_saphyr::from_str(&server[2].content).unwrap();
        assert_eq!(
            server["control-plane-resource-requests"],
            serde_json::json!(["etcd-memory=256Mi", "kube-controller-manager-memory=128Mi"])
        );
        let agent: serde_json::Value = serde_saphyr::from_str(&agent_files()[1].content).unwrap();
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
}
