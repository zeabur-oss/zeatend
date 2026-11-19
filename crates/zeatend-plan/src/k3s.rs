//! Host files written before K3s first starts.
//!
//! K3s hardened as its CIS guide says (<https://docs.k3s.io/security/hardening-guide>), to match
//! what RKE2's `cis` profile does by itself: Secrets encrypted at rest, Pod Security `restricted`
//! by default outside `kube-system`, `NetworkPolicies` that guard the system namespaces, an audit
//! log, `NodeRestriction` and `EventRateLimit`, and the kubelet and service-account flags. The
//! default level applies only to a namespace without its own Pod Security labels, as on RKE2.
//!
//! The pod network is K3s's packaged flannel with VXLAN, and K3s's embedded network policy
//! controller, from kube-router, enforces `NetworkPolicies`
//! (<https://docs.k3s.io/networking/networking-services#network-policy-controller>). Each node
//! runs its own controller in `k3s server` or `k3s agent`.
//!
//! A known gap, measured on K3s v1.36.6+k3s1: the controller covers a new Pod about one second
//! after it is Ready. A Pod in another namespace can open a connection in that second, and the
//! connection stays open after the rules arrive, because the controller accepts packets of
//! established connections. New connections after that second are refused.

use serde_json::{Value, json};

use crate::cluster::{
    AUDIT_POLICY, Distribution, HEADER, HostFile, Options, Rancher, turn_off, yaml_list,
};
use crate::config::Component;
use crate::plan::MANAGER;
use crate::tuning::HostTuning;

/// The binary; `install.sh` expects it here.
pub const BINARY: &str = "/usr/local/bin/k3s";
/// The audit log's directory. Only root may read it, so K3s does not make it itself.
pub const AUDIT_LOG_DIR: &str = "/var/lib/rancher/k3s/server/logs";
/// Manifests K3s applies on every start and keeps applied.
pub const MANIFESTS_DIR: &str = "/var/lib/rancher/k3s/server/manifests";
/// The main configuration file of a server or a worker.
pub const CONFIG_FILE: &str = "/etc/rancher/k3s/config.yaml";
/// The drop-in that sets `GOMEMLIMIT` for `k3s server`. `install.sh` writes the unit and its
/// environment file again on every install, so the limit is in a drop-in of its own.
pub const MEMORY_DROP_IN: &str = "/etc/systemd/system/k3s.service.d/zeatend-memory.conf";

const K3S: Rancher = Rancher::K3s;

/// The files to write, in the order to write them.
#[must_use]
pub fn host_files(options: &Options) -> Vec<HostFile> {
    let dir = K3S.layout().config_dir;
    vec![
        K3S.sysctl(),
        HostFile {
            path: format!("{dir}/audit-policy.yaml"),
            mode: 0o600,
            content: AUDIT_POLICY.to_owned(),
        },
        HostFile {
            path: format!("{dir}/admission.yaml"),
            mode: 0o600,
            content: ADMISSION.to_owned(),
        },
        HostFile {
            path: CONFIG_FILE.to_owned(),
            mode: 0o600,
            content: config(options),
        },
        HostFile {
            path: format!("{MANIFESTS_DIR}/zeabur-cis.yaml"),
            mode: 0o600,
            content: system_objects(&options.disable),
        },
    ]
    .into_iter()
    .chain(memory_drop_in(&options.tuning))
    .collect()
}

/// The unit drop-in with the `GOMEMLIMIT` of `tuning`, if it has one.
fn memory_drop_in(tuning: &HostTuning) -> Option<HostFile> {
    tuning.environment().map(|environment| HostFile {
        path: MEMORY_DROP_IN.to_owned(),
        mode: 0o644,
        content: format!(
            "# Written by zeatend; `zeatend apply` writes it again.\n[Service]\n{environment}"
        ),
    })
}

/// Whether the K3s configuration `config` turns flannel off (`flannel-backend: none`).
///
/// An earlier install gave such a cluster a network plugin of its own, such as Calico. A file that
/// does not parse runs flannel, as K3s would not start with it.
#[must_use]
pub fn runs_own_network(config: &str) -> bool {
    serde_saphyr::from_str::<Value>(config)
        .is_ok_and(|value| value.get("flannel-backend").and_then(Value::as_str) == Some("none"))
}

/// The files of a worker: the kernel parameters and the kubelet settings of the server.
#[must_use]
pub fn agent_files(tuning: &HostTuning) -> Vec<HostFile> {
    vec![
        K3S.sysctl(),
        HostFile {
            path: CONFIG_FILE.to_owned(),
            mode: 0o600,
            content: format!(
                "{HEADER}protect-kernel-defaults: true\n{}",
                Rancher::kubelet_args(tuning, false)
            ),
        },
    ]
}

fn config(options: &Options) -> String {
    let dir = K3S.layout().config_dir;
    // An existing cluster keeps its network: changing it would cut off every pod.
    let network = if options.own_network {
        "# Kept from the earlier install: the cluster runs a network plugin of its own.\n\
         flannel-backend: none\n\
         disable-network-policy: true\n"
    } else {
        ""
    };
    let mut out = format!(
        "{HEADER}protect-kernel-defaults: true\n\
         secrets-encryption: true\n\
         write-kubeconfig-mode: \"0600\"\n\
         {network}\
         kube-apiserver-arg:\n  \
           - enable-admission-plugins=NodeRestriction,EventRateLimit\n  \
           - admission-control-config-file={dir}/admission.yaml\n  \
           - audit-policy-file={dir}/audit-policy.yaml\n  \
           - audit-log-path={AUDIT_LOG_DIR}/audit.log\n  \
           - audit-log-maxage=30\n  \
           - audit-log-maxbackup=10\n  \
           - audit-log-maxsize=100\n  \
           - service-account-extend-token-expiration=false\n\
         kube-controller-manager-arg:\n  \
           - terminated-pod-gc-threshold=1000\n"
    );
    out.push_str(&Rancher::kubelet_args(&options.tuning, true));
    // ServiceLB stays and gives an ingress ports 80 and 443.
    turn_off(&mut out, Distribution::K3s, &options.disable);
    yaml_list(&mut out, "tls-san", &options.tls_san);
    out
}

/// Pod Security `restricted` for every namespace without its own labels, `kube-system` exempt, as
/// RKE2's `cis` profile writes it; and a cluster-wide limit on Events, which a flood of them from
/// one workload would otherwise turn into load on the datastore.
const ADMISSION: &str = "# Written by zeatend.
apiVersion: apiserver.config.k8s.io/v1
kind: AdmissionConfiguration
plugins:
  - name: PodSecurity
    configuration:
      apiVersion: pod-security.admission.config.k8s.io/v1
      kind: PodSecurityConfiguration
      defaults:
        enforce: restricted
        enforce-version: latest
        audit: restricted
        audit-version: latest
        warn: restricted
        warn-version: latest
      exemptions:
        usernames: []
        runtimeClasses: []
        namespaces: [kube-system]
  - name: EventRateLimit
    configuration:
      apiVersion: eventratelimit.admission.k8s.io/v1alpha1
      kind: Configuration
      limits:
        - type: Server
          qps: 5000
          burst: 20000
";

/// The port K3s's packaged metrics-server serves on (`--secure-port`).
const METRICS_SERVER_PORT: u16 = 10250;

/// What RKE2's `cis` profile creates in the system namespaces and K3s does not: a policy that
/// lets in only the namespace's own pods, `CoreDNS` reachable from every pod, Traefik's entry
/// points reachable unless `disable` turns Traefik off, and default service accounts that mount no
/// token. K3s applies the file on every start, as RKE2 re-creates its own.
///
/// The API server reaches metrics-server from the server's host, which is not a pod of
/// `kube-system`. On another node, kube-router refuses that traffic, so `kubectl top` and the
/// Horizontal Pod Autoscaler stop. metrics-server's port is open, as `CoreDNS`'s is: metrics-server
/// asks the API server to authenticate and authorise each request.
fn system_objects(disable: &[Component]) -> String {
    let mut objects: Vec<Value> = ["kube-system", "kube-public", "default"]
        .into_iter()
        .map(|namespace| {
            policy(
                "default-network-policy",
                namespace,
                &json!({}),
                &json!([{"from": [{"podSelector": {}}]}]),
            )
        })
        .collect();
    objects.push(policy(
        "default-network-dns-policy",
        "kube-system",
        &json!({"matchLabels": {"k8s-app": "kube-dns"}}),
        &json!([{"ports": [
            {"port": 53, "protocol": "TCP"},
            {"port": 53, "protocol": "UDP"},
        ]}]),
    ));
    objects.push(policy(
        "default-network-metrics-server-policy",
        "kube-system",
        &json!({"matchLabels": {"k8s-app": "metrics-server"}}),
        &json!([{"ports": [{"port": METRICS_SERVER_PORT, "protocol": "TCP"}]}]),
    ));
    if !disable.contains(&Component::Traefik) {
        // As RKE2's `default-network-traefik-policy`.
        objects.push(policy(
            "default-network-traefik-policy",
            "kube-system",
            &json!({"matchLabels": {"app.kubernetes.io/name": "traefik"}}),
            &json!([{"ports": [
                {"port": "web", "protocol": "TCP"},
                {"port": "websecure", "protocol": "TCP"},
            ]}]),
        ));
    }
    objects.extend(
        ["kube-system", "kube-public", "default", "kube-node-lease"]
            .into_iter()
            .map(|namespace| {
                json!({
                    "apiVersion": "v1",
                    "kind": "ServiceAccount",
                    "metadata": {"name": "default", "namespace": namespace},
                    "automountServiceAccountToken": false,
                })
            }),
    );
    let mut out = String::from("# Written by zeatend; K3s applies it on every start.\n");
    for object in objects {
        out.push_str("---\n");
        out.push_str(&serde_json::to_string(&object).unwrap_or_default());
        out.push('\n');
    }
    out
}

fn policy(name: &str, namespace: &str, pods: &Value, ingress: &Value) -> Value {
    json!({
        "apiVersion": "networking.k8s.io/v1",
        "kind": "NetworkPolicy",
        "metadata": {
            "name": name,
            "namespace": namespace,
            "labels": {"app.kubernetes.io/managed-by": MANAGER},
        },
        "spec": {"podSelector": pods, "policyTypes": ["Ingress"], "ingress": ingress},
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cluster::Distribution;
    use crate::tuning::MemoryTier;

    fn tuning(tier: MemoryTier) -> HostTuning {
        HostTuning::new(Distribution::K3s, tier)
    }

    fn options() -> Options {
        Options {
            tls_san: vec!["k8s.example.com".to_owned()],
            tuning: tuning(MemoryTier::Small),
            own_network: false,
            disable: Vec::new(),
        }
    }

    fn config_of(files: &[HostFile]) -> Value {
        yaml(
            &files
                .iter()
                .find(|f| f.path == CONFIG_FILE)
                .unwrap()
                .content,
        )
    }

    fn yaml(content: &str) -> Value {
        serde_saphyr::from_str(content).unwrap()
    }

    #[test]
    fn the_server_is_hardened_as_the_k3s_cis_guide_says() {
        let config = config_of(&host_files(&options()));
        assert_eq!(config["protect-kernel-defaults"], true);
        assert_eq!(config["secrets-encryption"], true);
        let apiserver = config["kube-apiserver-arg"].as_array().unwrap();
        for arg in [
            "enable-admission-plugins=NodeRestriction,EventRateLimit",
            "admission-control-config-file=/etc/rancher/k3s/admission.yaml",
            "audit-log-path=/var/lib/rancher/k3s/server/logs/audit.log",
            "service-account-extend-token-expiration=false",
        ] {
            assert!(apiserver.contains(&json!(arg)), "{arg}");
        }
        // K3s's own defaults: Traefik, metrics-server, local-path and ServiceLB all run.
        assert!(config.get("disable").is_none());
        assert_eq!(config["tls-san"], json!(["k8s.example.com"]));
    }

    #[test]
    fn pods_are_restricted_outside_kube_system() {
        let admission = yaml(ADMISSION);
        let psa = &admission["plugins"][0]["configuration"];
        assert_eq!(psa["defaults"]["enforce"], "restricted");
        assert_eq!(psa["exemptions"]["namespaces"], json!(["kube-system"]));
        assert_eq!(psa["exemptions"]["usernames"], json!([]));
    }

    #[test]
    fn system_namespaces_let_in_only_their_own_pods_and_dns() {
        let files = host_files(&options());
        let manifest = files
            .iter()
            .find(|f| f.path.starts_with(MANIFESTS_DIR))
            .unwrap();
        let objects: Vec<Value> = manifest
            .content
            .split("---\n")
            .skip(1)
            .map(|d| serde_json::from_str(d).unwrap())
            .collect();
        let policy = |namespace: &str, name: &str| {
            objects
                .iter()
                .find(|o| {
                    o["kind"] == "NetworkPolicy"
                        && o["metadata"]["namespace"] == namespace
                        && o["metadata"]["name"] == name
                })
                .cloned()
        };
        for namespace in ["kube-system", "kube-public", "default"] {
            let p = policy(namespace, "default-network-policy").unwrap();
            assert_eq!(
                p["spec"]["ingress"],
                json!([{"from": [{"podSelector": {}}]}])
            );
        }
        let dns = policy("kube-system", "default-network-dns-policy").unwrap();
        assert_eq!(
            dns["spec"]["podSelector"]["matchLabels"]["k8s-app"],
            "kube-dns"
        );
        assert!(dns["spec"]["ingress"][0].get("from").is_none());
        let metrics = policy("kube-system", "default-network-metrics-server-policy").unwrap();
        assert_eq!(
            metrics["spec"]["podSelector"]["matchLabels"]["k8s-app"],
            "metrics-server"
        );
        assert_eq!(
            metrics["spec"]["ingress"],
            json!([{"ports": [{"port": 10250, "protocol": "TCP"}]}]),
            "the API server reaches it from another node's host"
        );
        assert!(
            objects
                .iter()
                .filter(|o| o["kind"] == "ServiceAccount")
                .all(|o| o["automountServiceAccountToken"] == false)
        );
    }

    #[test]
    fn agents_get_the_kubelet_settings_and_no_server_keys() {
        let files = agent_files(&tuning(MemoryTier::Standard));
        assert_eq!(files[0], K3S.sysctl());
        let config = yaml(&files[1].content);
        assert_eq!(config["protect-kernel-defaults"], true);
        assert!(
            config["kubelet-arg"]
                .as_array()
                .unwrap()
                .contains(&json!("pod-max-pids=4096"))
        );
        let args = config["kubelet-arg"].as_array().unwrap();
        assert!(args.contains(&json!("system-reserved=memory=350Mi")));
        assert!(args.contains(&json!("eviction-soft=memory.available<400Mi")));
        assert!(
            !args
                .iter()
                .any(|a| a.as_str().unwrap().starts_with("kube-reserved="))
        );
        assert!(config.get("secrets-encryption").is_none());
        assert!(config.get("kube-apiserver-arg").is_none());
    }

    #[test]
    fn a_new_cluster_runs_flannel_and_the_policy_controller() {
        let config = config_of(&host_files(&options()));
        assert!(config.get("flannel-backend").is_none());
        assert!(config.get("disable-network-policy").is_none());
        assert!(!runs_own_network(&serde_json::to_string(&config).unwrap()));
    }

    #[test]
    fn a_cluster_with_its_own_network_keeps_it() {
        let earlier =
            "protect-kernel-defaults: true\nflannel-backend: none\ndisable-network-policy: true\n";
        assert!(runs_own_network(earlier));
        assert!(!runs_own_network("flannel-backend: vxlan\n"));
        assert!(!runs_own_network("not: [yaml"));
        let config = config_of(&host_files(&Options {
            own_network: true,
            ..options()
        }));
        assert_eq!(config["flannel-backend"], "none");
        assert_eq!(config["disable-network-policy"], true);
    }

    #[test]
    fn the_server_gets_its_reserves_and_gomemlimit_by_tier() {
        for (tier, kube, soft, limit) in [
            (
                MemoryTier::Small,
                "kube-reserved=memory=100Mi",
                false,
                "600MiB",
            ),
            (
                MemoryTier::Standard,
                "kube-reserved=memory=1530Mi",
                true,
                "1530MiB",
            ),
        ] {
            let files = host_files(&Options {
                tuning: tuning(tier),
                ..options()
            });
            let config = config_of(&files);
            let args = config["kubelet-arg"].as_array().unwrap();
            assert!(
                args.contains(&json!("system-reserved=memory=350Mi")),
                "{tier:?}"
            );
            assert!(args.contains(&json!(kube)), "{tier:?}");
            assert_eq!(
                args.iter()
                    .any(|a| a.as_str().unwrap().starts_with("eviction-soft=")),
                soft,
                "{tier:?}"
            );
            let drop_in = files.iter().find(|f| f.path == MEMORY_DROP_IN).unwrap();
            assert!(
                drop_in
                    .content
                    .contains(&format!("[Service]\nEnvironment=GOMEMLIMIT={limit}\n")),
                "{tier:?}"
            );
            assert!(
                agent_files(&tuning(tier))
                    .iter()
                    .all(|f| f.path != MEMORY_DROP_IN)
            );
        }
    }

    #[test]
    fn disable_turns_off_the_components_and_traefiks_policy() {
        let traefik_policy = |files: &[HostFile]| {
            files
                .iter()
                .any(|f| f.content.contains("default-network-traefik-policy"))
        };
        assert!(traefik_policy(&host_files(&options())));
        let files = host_files(&Options {
            disable: Component::ALL.to_vec(),
            ..options()
        });
        assert_eq!(
            config_of(&files)["disable"],
            json!(["traefik", "metrics-server", "local-storage"])
        );
        assert!(!traefik_policy(&files));
    }
}
