#!/bin/sh
# Deploys one notes-hub image on the VPS. Installed as /opt/notes-hub/deploy.sh
# and run only as the forced command of CI's deploy key in root's
# authorized_keys (see wiki/architecture/hub-deploy.md, "CI deploys").
#   SSH_ORIGINAL_COMMAND  the image tag: the 40-hex commit sha
#   stdin, first line     a GHCR token for this pull (CI's job token)
# The token only lives in a temporary DOCKER_CONFIG that is removed on exit.
set -eu

TAG="${SSH_ORIGINAL_COMMAND:-}"
case "$TAG" in
  *[!0-9a-f]* | "") echo "bad tag" >&2; exit 2 ;;
esac
[ "${#TAG}" -eq 40 ] || { echo "bad tag" >&2; exit 2; }
IMAGE="ghcr.io/bubblebuffer/notes-hub:$TAG"

cd /opt/notes-hub
exec 9>.deploy.lock
flock -n 9 || { echo "a deploy is already running" >&2; exit 3; }

CFG=$(mktemp -d)
trap 'rm -rf "$CFG"' EXIT
head -n 1 | DOCKER_CONFIG="$CFG" docker login ghcr.io -u bubblebuffer --password-stdin >/dev/null
DOCKER_CONFIG="$CFG" docker pull -q "$IMAGE" >/dev/null
rm -rf "$CFG"

if docker image inspect notes-hub:latest >/dev/null 2>&1; then
  docker tag notes-hub:latest notes-hub:previous
fi
docker tag "$IMAGE" notes-hub:latest
docker compose up -d --quiet-pull hub

healthy() {
  i=0
  while [ "$i" -lt 60 ]; do
    [ "$(docker inspect -f '{{.State.Health.Status}}' notes-hub 2>/dev/null)" = healthy ] && return 0
    i=$((i + 1)); sleep 2
  done
  return 1
}
if ! healthy; then
  echo "notes-hub did not become healthy; rolling back to notes-hub:previous" >&2
  docker tag notes-hub:previous notes-hub:latest
  docker compose up -d hub
  exit 4
fi

# Disk is tight: keep only notes-hub:latest and :previous.
docker images ghcr.io/bubblebuffer/notes-hub --format '{{.Repository}}:{{.Tag}}' \
  | xargs -r docker rmi >/dev/null 2>&1 || true
docker image prune -f >/dev/null
echo "deployed $TAG"
