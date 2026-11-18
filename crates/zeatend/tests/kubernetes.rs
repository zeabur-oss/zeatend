//! Cluster tests. They need a disposable cluster that zeatend installed, `kubectl` on the path and
//! `KUBECONFIG` set to that cluster's admin kubeconfig:
//!
//! ```sh
//! cargo nextest run --locked --run-ignored only -E 'binary(kubernetes)'
//! ```

#[cfg(test)]
mod tests {
    use std::io::Write as _;
    use std::process::{Command, Output, Stdio};
    use std::thread;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    use serde_json::{Value, json};

    /// An image with `socat`, which serves and opens the test connections.
    const IMAGE: &str = "docker.io/alpine/socat:1.8.0.3";
    const PORT: u16 = 9000;

    /// Runs `kubectl` with `args` and `stdin`.
    fn kubectl(args: &[&str], stdin: &str) -> Output {
        let mut child = Command::new("kubectl")
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("kubectl runs");
        child
            .stdin
            .take()
            .expect("stdin is piped")
            .write_all(stdin.as_bytes())
            .expect("kubectl reads stdin");
        child.wait_with_output().expect("kubectl ends")
    }

    /// Runs `kubectl` and fails the test when it fails. Returns its standard output.
    fn ok(args: &[&str], stdin: &str) -> String {
        let out = kubectl(args, stdin);
        assert!(
            out.status.success(),
            "kubectl {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    /// Deletes the test's namespaces when the test ends, also when it fails.
    struct Namespaces(Vec<String>);

    impl Drop for Namespaces {
        fn drop(&mut self) {
            for ns in &self.0 {
                kubectl(&["delete", "namespace", ns, "--wait=false"], "");
            }
        }
    }

    /// A Pod that the `restricted` Pod Security level admits, running `command`.
    fn pod(name: &str, namespace: &str, command: &[&str]) -> Value {
        json!({
            "apiVersion": "v1",
            "kind": "Pod",
            "metadata": {"name": name, "namespace": namespace},
            "spec": {
                "securityContext": {
                    "runAsNonRoot": true,
                    "runAsUser": 65534,
                    "seccompProfile": {"type": "RuntimeDefault"},
                },
                "containers": [{
                    "name": "socat",
                    "image": IMAGE,
                    "command": command,
                    "securityContext": {
                        "allowPrivilegeEscalation": false,
                        "capabilities": {"drop": ["ALL"]},
                    },
                }],
            },
        })
    }

    /// A `NetworkPolicy` on every Pod in `namespace` that allows the ingress in `ingress`.
    fn policy(name: &str, namespace: &str, ingress: &Value) -> Value {
        json!({
            "apiVersion": "networking.k8s.io/v1",
            "kind": "NetworkPolicy",
            "metadata": {"name": name, "namespace": namespace},
            "spec": {"podSelector": {}, "policyTypes": ["Ingress"], "ingress": ingress},
        })
    }

    /// Whether `client` in `namespace` opens a connection to `ip` and gets its line back.
    fn echoes(namespace: &str, client: &str, ip: &str) -> bool {
        let target = format!("TCP:{ip}:{PORT},connect-timeout=2");
        let out = kubectl(
            &[
                "exec", "-i", "-n", namespace, client, "--", "socat", "-T2", "-", &target,
            ],
            "ping\n",
        );
        out.status.success() && String::from_utf8_lossy(&out.stdout).trim() == "ping"
    }

    /// Asks `check` every second until it holds or `limit` passes. Returns whether it held.
    fn eventually(limit: Duration, mut check: impl FnMut() -> bool) -> bool {
        let start = Instant::now();
        while start.elapsed() < limit {
            if check() {
                return true;
            }
            thread::sleep(Duration::from_secs(1));
        }
        false
    }

    /// A Pod in a namespace with default-deny ingress and an allow rule for its own namespace takes
    /// connections from that namespace only. The policy controller covers a new Pod about a second
    /// after it is Ready, so the test waits for the first refusal and then checks that every later
    /// connection from the other namespace is refused.
    #[test]
    #[ignore = "needs a cluster that zeatend installed; see the module documentation"]
    fn a_pod_in_another_namespace_cannot_connect_through_a_default_deny_policy() {
        let id = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("the clock is after 1970")
            .as_millis();
        let target = format!("zeatend-netpol-target-{id}");
        let other = format!("zeatend-netpol-other-{id}");
        let _cleanup = Namespaces(vec![target.clone(), other.clone()]);

        let listen = format!("TCP-LISTEN:{PORT},fork,reuseaddr");
        let namespace = |name: &str| json!({"apiVersion": "v1", "kind": "Namespace", "metadata": {"name": name}});
        let manifest = json!({
            "apiVersion": "v1",
            "kind": "List",
            "items": [
                namespace(&target),
                namespace(&other),
                policy("default-deny", &target, &json!([])),
                policy("same-namespace", &target, &json!([{"from": [{"podSelector": {}}]}])),
                pod("echo", &target, &["socat", &listen, "EXEC:cat"]),
                pod("client", &target, &["sleep", "3600"]),
                pod("client", &other, &["sleep", "3600"]),
            ],
        })
        .to_string();
        ok(&["apply", "-f", "-"], &manifest);
        for ns in [&target, &other] {
            ok(
                &[
                    "wait",
                    "-n",
                    ns,
                    "--for=condition=Ready",
                    "pod",
                    "--all",
                    "--timeout=180s",
                ],
                "",
            );
        }
        let ip = ok(
            &[
                "get",
                "pod",
                "-n",
                &target,
                "echo",
                "-o",
                "jsonpath={.status.podIP}",
            ],
            "",
        );

        assert!(
            eventually(Duration::from_secs(30), || echoes(&target, "client", &ip)),
            "a Pod in the same namespace connects"
        );
        assert!(
            eventually(Duration::from_secs(30), || !echoes(&other, "client", &ip)),
            "the policy refuses a Pod in another namespace"
        );
        for attempt in 1..=5 {
            assert!(
                !echoes(&other, "client", &ip),
                "attempt {attempt}: a Pod in another namespace connected"
            );
        }
        assert!(
            echoes(&target, "client", &ip),
            "a Pod in the same namespace still connects"
        );
    }
}
