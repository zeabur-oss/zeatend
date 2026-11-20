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
