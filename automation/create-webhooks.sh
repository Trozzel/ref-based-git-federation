WEBHOOK_SECRET="mys3cr3t"

create_webhook() {
  local port="$1"
  local repo_name="$2"
  local hook_url="$3"
  local gitea_admin_user="$4"
  local gitea_admin_pass="$5"

  curl -fsS -X POST "http://localhost:${port}/api/v1/repos/${gitea_admin_user}/${repo_name}/hooks" \
    -u "${gitea_admin_user}:${gitea_admin_pass}" \
    -H "Content-Type: application/json" \
    -d "{
      \"type\": \"gitea\",
      \"active\": true,
      \"branch_filter\": \"*\",
      \"events\": [\"push\"],
      \"config\": {
        \"url\": \"${hook_url}/webhook/gitea\",
        \"content_type\": \"json\",
        \"http_method\": \"post\",
        \"secret\": \"${WEBHOOK_SECRET}\"
      }
    }"
}

create_webhook $1 $2 $3 $4 $5
