//! The TCP ports of a cluster and the URL of its API server.

/// HTTP, taken on every node by the ingress.
pub const HTTP: u16 = 80;
/// HTTPS, taken on every node by the ingress.
pub const HTTPS: u16 = 443;
/// The Kubernetes API server, which K3s also joins nodes through.
pub const API_SERVER: u16 = 6443;
/// RKE2's supervisor, which joins nodes.
pub const SUPERVISOR: u16 = 9345;
/// The kubelet, and K3s's metrics-server.
pub const KUBELET: u16 = 10250;
/// etcd, for clients.
pub const ETCD_CLIENT: u16 = 2379;
/// etcd, for its peers.
pub const ETCD_PEER: u16 = 2380;
/// `KubeSolo`'s webhook.
pub const KUBESOLO_WEBHOOK: u16 = 10443;

/// `https://<host>:<port>`, with an IPv6 address in brackets.
#[must_use]
pub fn api_server_url(host: &str, port: u16) -> String {
    if host.contains(':') {
        format!("https://[{host}]:{port}")
    } else {
        format!("https://{host}:{port}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_ipv6_host_is_in_brackets() {
        assert_eq!(
            api_server_url("fd00::5", SUPERVISOR),
            "https://[fd00::5]:9345"
        );
        assert_eq!(
            api_server_url("10.0.0.5", API_SERVER),
            "https://10.0.0.5:6443"
        );
        assert_eq!(
            api_server_url("k8s.example.com", API_SERVER),
            "https://k8s.example.com:6443"
        );
    }
}
