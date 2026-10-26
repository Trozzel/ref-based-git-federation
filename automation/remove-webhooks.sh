#!/usr/bin/env bash

set -euo pipefail

remove_webhook() {
  local port="$1"
  local repo_name="$2"
  local hook_url="$3"
  local gitea_admin_user="$4"
  local gitea_admin_pass="$5"
  local full_hook_url="${hook_url}/webhook/gitea"
  local hooks_response
  local hook_id

  hooks_response="$(curl -fsS "http://localhost:${port}/api/v1/repos/${gitea_admin_user}/${repo_name}/hooks" \
    -u "${gitea_admin_user}:${gitea_admin_pass}")"

  if command -v jq >/dev/null 2>&1; then
    hook_id="$(printf '%s' "${hooks_response}" | jq -r --arg url "${full_hook_url}" '.[] | select(.config.url == $url) | .id' | head -n 1)"
  else
    hook_id="$(printf '%s' "${hooks_response}" | grep -B 10 "\"url\":\"${full_hook_url}\"" | grep -o '"id":[0-9]\+' | head -n 1 | cut -d: -f2)"
  fi

  if [[ -z "${hook_id}" || "${hook_id}" == "null" ]]; then
    echo "No webhook found for ${full_hook_url} in ${gitea_admin_user}/${repo_name} on port ${port}" >&2
    return 1
  fi

  curl -fsS -X DELETE "http://localhost:${port}/api/v1/repos/${gitea_admin_user}/${repo_name}/hooks/${hook_id}" \
    -u "${gitea_admin_user}:${gitea_admin_pass}"
}

remove_webhook "$1" "$2" "$3" "$4" "$5"
