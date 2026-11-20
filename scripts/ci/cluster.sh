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

cargo build --locked -p zeatend
zeatend="$PWD/target/debug/zeatend"
config="$(mktemp -d)/config.yaml"
echo "distribution: $distribution" >"$config"

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
  cargo nextest run --locked --run-ignored only -E 'binary(kubernetes)'
fi

# A second apply changes nothing and succeeds.
sudo "$zeatend" --config "$config" apply --foreground
sudo "$zeatend" --config "$config" uninstall --yes --delete-volumes
if sudo systemctl is-active --quiet "$distribution" "$distribution-server" 2>/dev/null; then
  echo "$distribution still runs after uninstall" >&2
  exit 1
fi
