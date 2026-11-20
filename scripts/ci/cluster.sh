#!/usr/bin/env bash
# Installs a cluster on this host with zeatend, as a person would, runs the cluster tests on it,
# applies again and uninstalls. It changes the host: run it on a disposable machine only.
#
#   scripts/ci/cluster.sh <k3s|rke2|kubesolo>
set -euo pipefail
cd "$(dirname "$0")/../.."
source scripts/lib.sh

distribution="${1:?usage: cluster.sh <k3s|rke2|kubesolo>}"
case "$distribution" in
  k3s) admin=/etc/rancher/k3s/k3s.yaml ;;
  rke2) admin=/etc/rancher/rke2/rke2.yaml ;;
  kubesolo) admin=/var/lib/kubesolo/pki/admin/admin.kubeconfig ;;
  *) echo "unknown distribution $distribution" >&2; exit 2 ;;
esac
unit="$distribution"
[[ "$distribution" == rke2 ]] && unit=rke2-server

# On failure, print what the cluster was doing, so the log explains it without a rerun.
dump() {
  echo "::group::$unit journal"
  sudo journalctl -u "$unit" --no-pager -o short-precise | tail -n 2000 || true
  echo "::endgroup::"
  if sudo test -f "$admin"; then
    echo "::group::pods, deployments and events"
    sudo "$(command -v kubectl)" --kubeconfig "$admin" --request-timeout=10s get nodes,pods,deployments -A -o wide || true
    sudo "$(command -v kubectl)" --kubeconfig "$admin" --request-timeout=10s get events -A --sort-by=.lastTimestamp || true
    echo "::endgroup::"
  fi
}
on_exit() {
  local code=$?
  ((code == 0)) || dump
  exit "$code"
}
trap on_exit EXIT

cargo build --locked -p zeatend
zeatend="$PWD/target/debug/zeatend"
config="$(mktemp -d)/config.yaml"

# A test JWT issuer: crates/zeatend/tests/kubernetes.rs serves it on 127.0.0.1:8943 with this
# certificate, and the API server trusts it through apiServer.jwt.
issuer="$(mktemp -d)"
openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes -days 1 \
  -subj /CN=zeatend-test-issuer-ca -keyout "$issuer/ca.key" -out "$issuer/ca.pem" 2>/dev/null
openssl req -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes -subj /CN=127.0.0.1 \
  -keyout "$issuer/server.key" -out "$issuer/server.csr" 2>/dev/null
openssl x509 -req -in "$issuer/server.csr" -CA "$issuer/ca.pem" -CAkey "$issuer/ca.key" \
  -CAcreateserial -days 1 -out "$issuer/server.pem" \
  -extfile <(printf 'subjectAltName=IP:127.0.0.1\nextendedKeyUsage=serverAuth\n') 2>/dev/null
openssl x509 -in "$issuer/server.pem" -outform DER -out "$issuer/server.der"
openssl pkcs8 -topk8 -nocrypt -in "$issuer/server.key" -outform DER -out "$issuer/server.key.der"
export ZEATEND_TEST_ISSUER_DIR="$issuer" ZEATEND_TEST_DISTRIBUTION="$distribution"

# Writes the configuration with the issuer for the audiences in $1, such as `[kubernetes]`.
write_config() {
  {
    echo "distribution: $distribution"
    echo "apiServer:"
    echo "  jwt:"
    echo "    - issuer: https://127.0.0.1:8943"
    echo "      certificateAuthority: |"
    sed 's/^/        /' "$issuer/ca.pem"
    echo "      audiences: $1"
    echo "      usernamePrefix: \"zeabur:\""
    echo "      groupsClaim: groups"
    echo "      groupsPrefix: \"zeabur:\""
  } >"$config"
}
jwt_test() {
  ZEATEND_TEST_AUDIENCE="$1" cargo nextest run --locked --run-ignored only \
    -E 'binary(kubernetes) and test(trusted_issuer)'
}
write_config "[kubernetes]"

sudo "$zeatend" --config "$config" plan
sudo "$zeatend" --config "$config" apply --foreground

kubeconfig="$(mktemp)"
sudo install -m 0600 -o "$(id -u)" "$admin" "$kubeconfig"
export KUBECONFIG="$kubeconfig"
kubectl get nodes -o wide
# Exit 1 is a warning, such as a runner's small disk; 2 and 3 fail.
status=0
sudo "$zeatend" --config "$config" doctor || status=$?
((status <= 1)) || { echo "zeatend doctor exited $status" >&2; exit 1; }
# KubeSolo enforces no NetworkPolicy (crates/zeatend-plan/src/kubesolo.rs), so it has no
# isolation to test.
if [[ "$distribution" != kubesolo ]]; then
  cargo nextest run --locked --run-ignored only -E 'binary(kubernetes) and not test(trusted_issuer)'
fi
jwt_test kubernetes

# A second apply changes nothing and succeeds.
sudo "$zeatend" --config "$config" apply --foreground

# A changed issuer list is written again: the issuer's tokens are now for another audience.
write_config "[zeatend-test]"
sudo "$zeatend" --config "$config" plan
sudo "$zeatend" --config "$config" apply --foreground
jwt_test zeatend-test
sudo "$zeatend" --config "$config" uninstall --yes --delete-volumes
if sudo systemctl is-active --quiet "$distribution" "$distribution-server" 2>/dev/null; then
  echo "$distribution still runs after uninstall" >&2
  exit 1
fi
