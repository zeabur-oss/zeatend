//! Cluster tests. They need a disposable cluster that zeatend installed, `kubectl` on the path and
//! `KUBECONFIG` set to that cluster's admin kubeconfig:
//!
//! ```sh
//! cargo nextest run --locked --run-ignored only -E 'binary(kubernetes)'
//! ```
//!
//! The JWT test also needs the configuration and the files that `scripts/ci/cluster.sh` writes;
//! see [`tests::a_token_of_a_trusted_issuer_signs_in_with_its_prefix`].

#[cfg(test)]
mod tests {
    use std::io::Write as _;
    use std::process::{Command, Output, Stdio};
    use std::thread;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    use serde::Deserialize;
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
        let namespaces = json!({
            "apiVersion": "v1",
            "kind": "List",
            "items": [namespace(&target), namespace(&other)],
        })
        .to_string();
        ok(&["apply", "-f", "-"], &namespaces);
        // Admission refuses a Pod until the namespace's default ServiceAccount exists, which a
        // controller creates after the namespace.
        for ns in [&target, &other] {
            ok(
                &[
                    "wait",
                    "-n",
                    ns,
                    "--for=create",
                    "serviceaccount/default",
                    "--timeout=60s",
                ],
                "",
            );
        }
        let manifest = json!({
            "apiVersion": "v1",
            "kind": "List",
            "items": [
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

    /// The test issuer: an HTTPS server on the host, which kube-apiserver reaches on every
    /// distribution, as K3s and `KubeSolo` run it in their own process and RKE2 in a pod on the
    /// host network.
    const ISSUER: &str = "https://127.0.0.1:8943";

    /// Serves the issuer's discovery document and `jwks` over HTTPS with the certificate and the
    /// PKCS #8 key, both DER, in `dir`, until the test ends.
    fn serve_issuer(dir: &str, jwks: &Value) {
        use std::io::Read as _;
        use std::sync::Arc;

        use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

        let read = |name: &str| std::fs::read(format!("{dir}/{name}")).expect("the issuer's files");
        let config = rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::aws_lc_rs::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .expect("TLS versions")
        .with_no_client_auth()
        .with_single_cert(
            vec![CertificateDer::from(read("server.der"))],
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(read("server.key.der"))),
        )
        .expect("the issuer's certificate and key");
        let config = Arc::new(config);
        let discovery = json!({
            "issuer": ISSUER,
            "jwks_uri": format!("{ISSUER}/jwks"),
            "response_types_supported": ["id_token"],
            "subject_types_supported": ["public"],
            "id_token_signing_alg_values_supported": ["ES256"],
        })
        .to_string();
        let jwks = jwks.to_string();
        let listener = std::net::TcpListener::bind("127.0.0.1:8943").expect("port 8943 is free");
        thread::spawn(move || {
            for tcp in listener.incoming().flatten() {
                let Ok(tls) = rustls::ServerConnection::new(Arc::clone(&config)) else {
                    continue;
                };
                let mut stream = rustls::StreamOwned::new(tls, tcp);
                let mut request = Vec::new();
                let mut buf = [0_u8; 1024];
                while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                    match stream.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => request.extend_from_slice(&buf[..n]),
                    }
                }
                let request = String::from_utf8_lossy(&request);
                let path = request.split_whitespace().nth(1).unwrap_or_default();
                let (status, body) = match path {
                    "/.well-known/openid-configuration" => ("200 OK", discovery.as_str()),
                    "/jwks" => ("200 OK", jwks.as_str()),
                    _ => ("404 Not Found", "{}"),
                };
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let sent = stream.write_all(response.as_bytes()).and_then(|()| {
                    stream.conn.send_close_notify();
                    stream.flush()
                });
                // kube-apiserver asks again.
                if let Err(e) = sent {
                    eprintln!("the test issuer: {e}");
                }
            }
        });
    }

    /// An ES256 key and its JWK set.
    fn signing_key() -> (aws_lc_rs::signature::EcdsaKeyPair, Value) {
        use aws_lc_rs::signature::{ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair, KeyPair as _};

        let key = EcdsaKeyPair::generate(&ECDSA_P256_SHA256_FIXED_SIGNING).expect("a P-256 key");
        // An uncompressed point: 0x04, then x and y.
        let point = key.public_key().as_ref();
        let jwks = json!({"keys": [{
            "kty": "EC", "crv": "P-256", "kid": "test", "use": "sig", "alg": "ES256",
            "x": base64url(&point[1..33]), "y": base64url(&point[33..65]),
        }]});
        (key, jwks)
    }

    fn base64url(bytes: &[u8]) -> String {
        use base64::Engine as _;
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
    }

    /// A token of the test issuer for `sub` in `groups`, for `audience`.
    fn token(
        key: &aws_lc_rs::signature::EcdsaKeyPair,
        sub: &str,
        groups: &[&str],
        audience: &str,
    ) -> String {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("the clock is after 1970")
            .as_secs();
        let header = json!({"alg": "ES256", "kid": "test", "typ": "JWT"});
        let claims = json!({
            "iss": ISSUER, "sub": sub, "aud": audience, "groups": groups,
            "iat": now, "nbf": now, "exp": now + 600,
        });
        let input = format!(
            "{}.{}",
            base64url(header.to_string().as_bytes()),
            base64url(claims.to_string().as_bytes())
        );
        let signature = key
            .sign(&aws_lc_rs::rand::SystemRandom::new(), input.as_bytes())
            .expect("signs");
        format!("{input}.{}", base64url(signature.as_ref()))
    }

    /// The cluster of the admin kubeconfig.
    #[derive(Deserialize)]
    struct KubeConfig {
        clusters: Vec<NamedCluster>,
    }

    #[derive(Deserialize)]
    struct NamedCluster {
        cluster: Cluster,
    }

    #[derive(Deserialize)]
    struct Cluster {
        server: String,
        #[serde(rename = "certificate-authority-data")]
        certificate_authority_data: String,
    }

    /// `kubectl auth whoami -o json`.
    #[derive(Deserialize)]
    struct SelfSubjectReview {
        status: ReviewStatus,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct ReviewStatus {
        user_info: UserInfo,
    }

    #[derive(Deserialize)]
    struct UserInfo {
        username: String,
        #[serde(default)]
        groups: Vec<String>,
    }

    /// The cluster of the admin kubeconfig.
    fn admin_cluster() -> Cluster {
        let admin: KubeConfig = serde_json::from_str(&ok(
            &["config", "view", "--raw", "--minify", "-o", "json"],
            "",
        ))
        .expect("a kubeconfig");
        admin
            .clusters
            .into_iter()
            .next()
            .expect("a cluster")
            .cluster
    }

    /// The status line of a request for `/version` with no credentials. kubectl would ask for a
    /// password instead of sending one, so `openssl s_client` sends it.
    fn anonymous_status() -> String {
        let server = admin_cluster().server;
        let address = server.trim_start_matches("https://").trim_end_matches('/');
        let mut child = Command::new("openssl")
            .args(["s_client", "-quiet", "-connect", address])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("openssl runs");
        child
            .stdin
            .take()
            .expect("stdin is piped")
            .write_all(b"GET /version HTTP/1.1\r\nHost: kubernetes\r\nConnection: close\r\n\r\n")
            .expect("openssl reads stdin");
        let out = child.wait_with_output().expect("openssl ends");
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .next()
            .unwrap_or_default()
            .to_owned()
    }

    /// Runs `kubectl` against the admin kubeconfig's cluster with `token` as the only credential.
    fn kubectl_as(token: &str, args: &[&str]) -> Output {
        let cluster = admin_cluster();
        let config = json!({
            "apiVersion": "v1",
            "kind": "Config",
            "clusters": [{"name": "c", "cluster": {
                "server": cluster.server,
                "certificate-authority-data": cluster.certificate_authority_data,
            }}],
            "users": [{"name": "u", "user": {"token": token}}],
            "contexts": [{"name": "c", "context": {"cluster": "c", "user": "u"}}],
            "current-context": "c",
        });
        let mut file = tempfile::NamedTempFile::new().expect("a temporary file");
        file.write_all(config.to_string().as_bytes())
            .expect("writes the kubeconfig");
        let path = file.path().to_str().expect("a UTF-8 path").to_owned();
        let mut full = vec!["--kubeconfig", path.as_str()];
        full.extend_from_slice(args);
        Command::new("kubectl")
            .args(&full)
            .env_remove("KUBECONFIG")
            .output()
            .expect("kubectl runs")
    }

    /// The API server trusts the issuer of `apiServer.jwt` that `scripts/ci/cluster.sh` sets: a
    /// token it signs for `ZEATEND_TEST_AUDIENCE` signs in as `zeabur:<sub>` in `zeabur:<group>`,
    /// and one for another audience does not. `ZEATEND_TEST_ISSUER_DIR` holds the issuer's
    /// certificate and key. On K3s and RKE2, anonymous requests stay refused.
    #[test]
    #[ignore = "needs a cluster that zeatend installed with a test issuer; see scripts/ci/cluster.sh"]
    fn a_token_of_a_trusted_issuer_signs_in_with_its_prefix() {
        let env = |name: &str| std::env::var(name).unwrap_or_else(|_| panic!("set {name}"));
        let audience = env("ZEATEND_TEST_AUDIENCE");
        let (key, jwks) = signing_key();
        serve_issuer(&env("ZEATEND_TEST_ISSUER_DIR"), &jwks);

        let alice = token(&key, "alice", &["admins"], &audience);
        let mut last = String::new();
        // kube-apiserver fetches the keys again until it gets them.
        let signed_in = eventually(Duration::from_secs(180), || {
            let out = kubectl_as(&alice, &["auth", "whoami", "-o", "json"]);
            last = String::from_utf8_lossy(&out.stderr).into_owned();
            out.status.success()
                && serde_json::from_slice::<SelfSubjectReview>(&out.stdout).is_ok_and(|review| {
                    let user = review.status.user_info;
                    user.username == "zeabur:alice"
                        && user.groups.iter().any(|g| g == "zeabur:admins")
                })
        });
        assert!(signed_in, "the token signs in as zeabur:alice: {last}");

        let other = token(&key, "alice", &[], "not-the-audience");
        let out = kubectl_as(&other, &["auth", "whoami"]);
        assert!(
            !out.status.success(),
            "a token for another audience is refused"
        );

        if env("ZEATEND_TEST_DISTRIBUTION") != "kubesolo" {
            let status = anonymous_status();
            assert!(
                status.starts_with("HTTP/1.1 401"),
                "an anonymous request is refused: {status}"
            );
        }
    }
}
