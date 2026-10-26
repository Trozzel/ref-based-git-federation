SOURCE_SITE_ID ?= site-1
TARGET_SITE_ID ?= site-2
SITE_COUNT     ?= 3
override PROPOSAL_ID_SEED := 00001
BRANCH         ?= main
TARGET_FILE    ?= stats.txt
REPO_BASE_PATH ?= /tmp/fed-git
DEMO_DIR       := $(dir $(abspath $(lastword $(MAKEFILE_LIST))))
REPO_PATH      ?= ${REPO_BASE_PATH}/${SOURCE_SITE_ID}
PROPOSAL_ID_DIR ?= ${DEMO_DIR}.sites
PROPOSAL_ID_FILE ?= ${PROPOSAL_ID_DIR}/${SOURCE_SITE_ID}.proposal_id
PROPOSAL_ID_EFFECTIVE = $(if $(wildcard $(PROPOSAL_ID_FILE)),$(shell cat "$(PROPOSAL_ID_FILE)"),$(PROPOSAL_ID_SEED))


.PHONY: help vars append overwrite bump-proposal-id new push validate-repo remotes refs

help: ## Show this help text.
	@printf '%s\n' 'Demo proposal targets'
	@printf '%s\n' ''
	@printf '%s\n' 'Usage:'
	@printf '%s\n' '  make -f demo/Makefile <target> [VAR=value ...]'
	@printf '%s\n' ''
	@printf '%s\n' 'Targets:'
	@awk 'BEGIN { FS = ":.*## "; } /^[A-Za-z0-9_.%\/-]+:.*## / { printf "  %-17s %s\n", $$1, $$2 }' "$(lastword $(MAKEFILE_LIST))"
	@printf '%s\n' ''
	@printf '%s\n' 'Variables:'
	@$(MAKE) --no-print-directory -f "$(lastword $(MAKEFILE_LIST))" vars
	@printf '%s\n' ''
	@printf '%s\n' 'Proposal ids are stored per source site in demo/.sites/<source_site>.proposal_id.'

vars: ## Show current variable values.
	@printf '  %-17s %s\n' 'SOURCE_SITE_ID' '${SOURCE_SITE_ID}'
	@printf '  %-17s %s\n' 'TARGET_SITE_ID' '${TARGET_SITE_ID}'
	@printf '  %-17s %s\n' 'SITE_COUNT' '${SITE_COUNT}'
	@printf '  %-17s %s\n' 'PROPOSAL_ID' '${PROPOSAL_ID_EFFECTIVE}'
	@printf '  %-17s %s\n' 'BRANCH' '${BRANCH}'
	@printf '  %-17s %s\n' 'TARGET_FILE' '${TARGET_FILE}'
	@printf '  %-17s %s\n' 'REPO_BASE_PATH' '${REPO_BASE_PATH}'
	@printf '  %-17s %s\n' 'REPO_PATH' '${REPO_PATH}'
	@printf '  %-17s %s\n' 'PROPOSAL_ID_DIR' '${PROPOSAL_ID_DIR}'
	@printf '  %-17s %s\n' 'PROPOSAL_ID_FILE' '${PROPOSAL_ID_FILE}'

validate-repo: ## Verify REPO_PATH and mutable paths are constrained.
	@repo_path=$$(cd "${REPO_PATH}" 2>/dev/null && pwd -P) || { echo "Error: REPO_PATH must exist: ${REPO_PATH}" >&2; exit 1; }; \
	repo_base_parent=$$(cd "$$(dirname "${REPO_BASE_PATH}")" 2>/dev/null && pwd -P) || { echo "Error: cannot resolve REPO_BASE_PATH parent: ${REPO_BASE_PATH}" >&2; exit 1; }; \
	repo_base_path="$${repo_base_parent}/$$(basename "${REPO_BASE_PATH}")"; \
	case "$${repo_path}" in \
		"$${repo_base_path}"/*) ;; \
		*) echo "Error: REPO_PATH must be under $${repo_base_path}/<site_name>: $${repo_path}" >&2; exit 1;; \
	esac; \
	case "$${repo_path#"$${repo_base_path}/"}" in \
		''|*/*) echo "Error: REPO_PATH must be exactly $${repo_base_path}/<site_name>: $${repo_path}" >&2; exit 1;; \
	esac; \
	toplevel=$$(git -C "$${repo_path}" rev-parse --show-toplevel 2>/dev/null) || { echo "Error: REPO_PATH is not a git repo: $${repo_path}" >&2; exit 1; }; \
	if [ "$${toplevel}" != "$${repo_path}" ]; then echo "Error: REPO_PATH must be the repo root: $${repo_path}" >&2; exit 1; fi; \
	case "${TARGET_FILE}" in \
		''|/*|..|../*|*/..|*/../*) echo "Error: TARGET_FILE must stay inside REPO_PATH: ${TARGET_FILE}" >&2; exit 1;; \
	esac; \
	target_path="$${repo_path}/${TARGET_FILE}"; \
	target_parent=$$(dirname "$${target_path}"); \
	if [ -e "$${target_parent}" ]; then \
		parent_real=$$(cd "$${target_parent}" 2>/dev/null && pwd -P) || { echo "Error: cannot resolve TARGET_FILE parent: $${target_parent}" >&2; exit 1; }; \
		case "$${parent_real}" in "$${repo_path}"|"$${repo_path}"/*) ;; *) echo "Error: TARGET_FILE parent resolves outside REPO_PATH: ${TARGET_FILE}" >&2; exit 1;; esac; \
	fi; \
	if [ -L "$${target_path}" ]; then echo "Error: TARGET_FILE must not be a symlink: ${TARGET_FILE}" >&2; exit 1; fi; \
	case "${SOURCE_SITE_ID}" in \
		''|/*|..|../*|*/..|*/../*) echo "Error: SOURCE_SITE_ID must be a single site name: ${SOURCE_SITE_ID}" >&2; exit 1;; \
	esac; \
	proposal_id_dir=$$(mkdir -p "${PROPOSAL_ID_DIR}" && cd "${PROPOSAL_ID_DIR}" && pwd -P) || { echo "Error: cannot create PROPOSAL_ID_DIR: ${PROPOSAL_ID_DIR}" >&2; exit 1; }; \
	case "$${proposal_id_dir}" in "${DEMO_DIR}.sites") ;; *) echo "Error: PROPOSAL_ID_DIR must resolve to ${DEMO_DIR}.sites: $${proposal_id_dir}" >&2; exit 1;; esac; \
	case "${PROPOSAL_ID_FILE}" in \
		"$${proposal_id_dir}/${SOURCE_SITE_ID}.proposal_id") ;; \
		*) echo "Error: PROPOSAL_ID_FILE must be $${proposal_id_dir}/${SOURCE_SITE_ID}.proposal_id: ${PROPOSAL_ID_FILE}" >&2; exit 1;; \
	esac

new: bump-proposal-id ## Bump the source-site proposal id and create a proposal branch.
	proposal_id=$$(cat "${PROPOSAL_ID_FILE}"); \
	proposal_ref=proposals/${TARGET_SITE_ID}/$${proposal_id}; \
	git -C "${REPO_PATH}" checkout -b "$${proposal_ref}" "${TARGET_SITE_ID}/${BRANCH}";

append: new ## Create a proposal branch, append to TARGET_FILE, commit, and push.
	timestamp=$(shell date -u +"%Y-%m-%dT%H:%M:%SZ"); \
	proposal_id=$$(cat "${PROPOSAL_ID_FILE}"); \
	proposal_ref=proposals/${TARGET_SITE_ID}/$${proposal_id}; \
	echo "foo - $${timestamp}" >> "${REPO_PATH}/${TARGET_FILE}"; \
	git -C "${REPO_PATH}" add -- "${TARGET_FILE}"; \
	git -C "${REPO_PATH}" commit -m "added foo to ${TARGET_FILE}"; \
	git -C "${REPO_PATH}" push origin "$${proposal_ref}"; \
	git -C "${REPO_PATH}" show;

overwrite conflict: new ## Create a proposal branch, overwrite TARGET_FILE, commit, and push.
	timestamp=$(shell date -u +"%Y-%m-%dT%H:%M:%SZ"); \
	proposal_id=$$(cat "${PROPOSAL_ID_FILE}"); \
	proposal_ref=proposals/${TARGET_SITE_ID}/$${proposal_id}; \
	echo "foo - $${timestamp}" > "${REPO_PATH}/${TARGET_FILE}"; \
	git -C "${REPO_PATH}" add -- "${TARGET_FILE}"; \
	git -C "${REPO_PATH}" commit -m "overwrote ${TARGET_FILE}"; \
	git -C "${REPO_PATH}" push origin "$${proposal_ref}"; \
	git -C "${REPO_PATH}" show;

bump-proposal-id: validate-repo ## Increment the source-site proposal id counter.
	counter_file="${PROPOSAL_ID_FILE}"; \
	last_id="${PROPOSAL_ID_EFFECTIVE}"; \
	case "$${last_id}" in ''|*[!0-9]*) echo "invalid proposal id: $${last_id}" >&2; exit 1;; esac; \
	width=$${#last_id}; \
	if [ "$${width}" -lt 5 ]; then width=5; fi; \
	proposal_id=$$(awk -v id="$${last_id}" -v width="$${width}" 'BEGIN { printf "%0*d", width, id + 1 }'); \
	printf '%s\n' "$${proposal_id}" > "$${counter_file}"

push: validate-repo ## Push the current source-site proposal ref.
	proposal_id=$$(cat "${PROPOSAL_ID_FILE}"); \
	proposal_ref=proposals/${TARGET_SITE_ID}/$${proposal_id}; \
	git -C "${REPO_PATH}" push origin "$${proposal_ref}"

remotes: ## Show git remotes for each demo site repo.
	@for site_id in $$(seq 1 $(SITE_COUNT)); do \
		echo site-$${site_id}; \
		git -C "$(REPO_BASE_PATH)/site-$${site_id}" remote -v; \
	done

refs: ## Show git refs for each demo site repo.
	@for site_id in $$(seq 1 $(SITE_COUNT)); do \
		echo site-$${site_id}; \
		git -C "$(REPO_BASE_PATH)/site-$${site_id}" show-ref; \
	done
