#!/bin/bash
set -euo pipefail

SCRIPT_DIR=$(cd -- "$(dirname -- "$0")" &>/dev/null && pwd)
NUM_SERVERS="${NUM_SERVERS:-3}"
BASE_TEST_DIR="${BASE_TEST_DIR:-/tmp/fed-git}"
LAB_DIR="${LAB_DIR:-/tmp/fed-lab}"
GITEA_ADMIN_USER="${GITEA_ADMIN_USER:-fed-git}"
GITEA_ADMIN_PASS="${GITEA_ADMIN_PASS:-password}"
GITEA_BASE_PORT="${GITEA_BASE_PORT:-30000}"
GITEA_BASE_SSH_PORT="${GITEA_BASE_SSH_PORT:-2220}"
APP_BASE_PORT="${APP_BASE_PORT:-8080}"
COMPOSE_FILE="${COMPOSE_FILE:-${SCRIPT_DIR}/docker-compose.generated.yaml}"
LEGACY_COMPOSE_FILE="${SCRIPT_DIR}/docker-compose.yaml"

remove_dir() {
  local dir=$1

  if [ ! -e "${dir}" ]; then
    return 0
  fi

  if rm -rf "${dir}" 2>/dev/null; then
    return 0
  fi

  if command -v docker >/dev/null 2>&1; then
    docker run --rm -v "${dir}:/target" alpine:3.22 \
      sh -c 'rm -rf /target/* /target/.[!.]* /target/..?*' >/dev/null 2>&1 || true
  fi

  rm -rf "${dir}"
}

cleanup() {
  echo "Cleaning up Docker containers and local directories..."
  if [ -f "${COMPOSE_FILE}" ]; then
    docker compose -f "${COMPOSE_FILE}" down --remove-orphans || true
  elif [ -f "${LEGACY_COMPOSE_FILE}" ]; then
    docker compose -f "${LEGACY_COMPOSE_FILE}" down --remove-orphans || true
  fi
  remove_dir "${BASE_TEST_DIR}"
  remove_dir "${LAB_DIR}"
  rm -f "${COMPOSE_FILE}"
}

wait_for_gitea() {
  local port=$1
  local max_attempts=${2:-30}
  local delay=${3:-2}
  echo "Waiting for Gitea on port ${port} to become ready..."
  for attempt in $(seq 1 "${max_attempts}"); do
    if curl -sf "http://localhost:${port}/api/v1/version" >/dev/null; then
      echo "Gitea on port ${port} is ready."
      return 0
    fi
    sleep "${delay}"
  done
  echo "Timed out waiting for Gitea on port ${port}." >&2
  return 1
}

http_port_for_site() {
  local site_id=$1
  echo $((GITEA_BASE_PORT + site_id))
}

app_port_for_site() {
  local site_id=$1
  echo $((APP_BASE_PORT + site_id))
}

repo_url_for_site() {
  local site_id=$1
  local http_port
  http_port=$(http_port_for_site "${site_id}")
  echo "http://${GITEA_ADMIN_USER}:${GITEA_ADMIN_PASS}@localhost:${http_port}/${GITEA_ADMIN_USER}/site-${site_id}.git"
}

compose() {
  docker compose -f "${COMPOSE_FILE}" "$@"
}

generate_compose() {
  NUM_SERVERS="${NUM_SERVERS}" \
    LAB_DIR="${LAB_DIR}" \
    GITEA_BASE_PORT="${GITEA_BASE_PORT}" \
    GITEA_BASE_SSH_PORT="${GITEA_BASE_SSH_PORT}" \
    "${SCRIPT_DIR}/generate-docker-compose.sh" >"${COMPOSE_FILE}"
}

if [ $# -gt 0 ]; then
  if [ "$1" == "clean" ]; then
    echo "Performing cleanup..."
    cleanup
    exit 0
  else
    echo "Usage: $0 [clean]"
    exit 1
  fi
fi

cleanup

echo "Generating Compose file at ${COMPOSE_FILE} for ${NUM_SERVERS} site(s)."
generate_compose

echo "Creating local testing directories under ${BASE_TEST_DIR}."
mkdir -p "${BASE_TEST_DIR}"
mkdir -p "${LAB_DIR}"
for i in $(seq 1 "${NUM_SERVERS}"); do
  mkdir -p "${BASE_TEST_DIR}/site-${i}"
  mkdir -p "${LAB_DIR}/gitea-${i}"
done

compose up -d

echo "Waiting until Gitea servers are ready."
for i in $(seq 1 "${NUM_SERVERS}"); do
  wait_for_gitea "$(http_port_for_site "${i}")"
done

echo "Creating admin users in each Gitea instance."
for i in $(seq 1 "${NUM_SERVERS}"); do
  docker exec -u 1000:1000 "gitea-${i}" /usr/local/bin/gitea admin user create \
    --admin \
    --username "${GITEA_ADMIN_USER}" \
    --password "${GITEA_ADMIN_PASS}" \
    --must-change-password=false \
    --email test@email.com
done

sleep 3

echo "Preparing repositories in each Gitea instance."
for i in $(seq 1 "${NUM_SERVERS}"); do
  curl -fsS -X POST "http://localhost:$(http_port_for_site "${i}")/api/v1/user/repos" \
    -u "${GITEA_ADMIN_USER}:${GITEA_ADMIN_PASS}" \
    -H "Content-Type: application/json" \
    -d "{\"name\": \"site-${i}\"}" >/dev/null
done

echo "Initializing local git repositories and pushing to Gitea instances."
for i in $(seq 1 "${NUM_SERVERS}"); do
  cd "${BASE_TEST_DIR}/site-${i}"
  echo "Working in directory: ${BASE_TEST_DIR}/site-${i}"
  git init -b main
  git config user.name "${GITEA_ADMIN_USER}"
  git config user.email test@email.com
  echo ".federation" >.gitignore
  echo "site-${i} initial state" >stats.txt
  git add .gitignore stats.txt
  git commit -m "first commit"
  git remote add origin "$(repo_url_for_site "${i}")"
  git push -u origin main
done

echo "Creating webhooks in each Gitea instance."
cd "${SCRIPT_DIR}"
for i in $(seq 1 "${NUM_SERVERS}"); do
  ./create-webhooks.sh \
    "$(http_port_for_site "${i}")" \
    "site-${i}" \
    "http://host.docker.internal:$(app_port_for_site "${i}")" \
    "${GITEA_ADMIN_USER}" \
    "${GITEA_ADMIN_PASS}"
done
