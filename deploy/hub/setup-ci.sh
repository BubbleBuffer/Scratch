#!/usr/bin/env bash
# One-time setup for .github/workflows/hub-image.yml, run from Git Bash on the
# dev PC with gh signed in as BubbleBuffer and the `vps` ssh alias working
# (Windows OpenSSH, for the agent keyring). It creates two keys and prints no
# private key material:
#   - a read-only deploy key on BubbleBuffer/BubblePaw-base, as notes-app's
#     BUBBLEPAW_DEPLOY_KEY secret
#   - a VPS key that can only run /opt/notes-hub/deploy.sh, as VPS_DEPLOY_KEY,
#     plus the VPS host key as VPS_KNOWN_HOSTS
# Re-running adds fresh keys; remove the old ones from the deploy keys list and
# root's authorized_keys (comment notes-app-ci-deploy).
set -euo pipefail
cd "$(dirname "$0")"
SSH=/c/Windows/System32/OpenSSH/ssh.exe
SCP=/c/Windows/System32/OpenSSH/scp.exe
REPO=BubbleBuffer/notes-app
VPS_IP=${VPS_IP:?set VPS_IP to the VPS public IP}
VPS_USER=${VPS_USER:-root}

K=$(mktemp -d)
trap 'rm -rf "$K"' EXIT
ssh-keygen -q -t ed25519 -N "" -C notes-app-ci-bubblepaw-read -f "$K/bp"
ssh-keygen -q -t ed25519 -N "" -C notes-app-ci-deploy -f "$K/vps"

gh repo deploy-key add "$K/bp.pub" -R BubbleBuffer/BubblePaw-base -t "notes-app CI (read-only)"
gh secret set BUBBLEPAW_DEPLOY_KEY -R "$REPO" < "$K/bp"
gh secret set VPS_DEPLOY_KEY -R "$REPO" < "$K/vps"
"$SSH" vps "ssh-keyscan -t ed25519 localhost 2>/dev/null | sed 's/^localhost/$VPS_IP/'" \
  | gh secret set VPS_KNOWN_HOSTS -R "$REPO"

"$SCP" -q deploy.sh vps:/opt/notes-hub/deploy.sh
"$SSH" vps "chmod 755 /opt/notes-hub/deploy.sh && command -v flock >/dev/null && \
  printf '%s\n' 'restrict,command=\"/opt/notes-hub/deploy.sh\" $(cat "$K/vps.pub")' >> /root/.ssh/authorized_keys && \
  tail -n 1 /root/.ssh/authorized_keys | cut -c1-60"
gh variable set VPS_HOST -R "$REPO" -b "$VPS_IP"
gh variable set VPS_USER -R "$REPO" -b "$VPS_USER"
gh secret list -R "$REPO"
gh variable list -R "$REPO"
echo "done: push to the release branch to build and deploy"
