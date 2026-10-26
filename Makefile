SHELL := /bin/bash
SITE_COUNT ?= 3
SITE_ID ?=
BASE_TEST_DIR ?= /tmp/fed-git
LAB_DIR ?= /tmp/fed-lab
GITEA_USERNAME ?= fed-git
GITEA_PASSWORD ?= password
GITEA_BASE_PORT ?= 30000
APP_BASE_PORT ?= 8080
LOG_DIR ?= logs

.PHONY: help build up down cycle demo demo-up demo-down demo-stop site farewell snoop uninstall remotes refs proposals format lint test fuzz fuzz-repro kill FORCE

help: ## Show this help message.
	@awk 'BEGIN {FS = ":.*##"; printf "Available targets:\n"} /^[a-zA-Z0-9_%.-]+:.*##/ { target = $$1; sub(/%$$/, "<id>", target); printf "  %-16s %s\n", target, $$2 }' $(MAKEFILE_LIST)

build: format ## Format and build the rust binaries.
	cargo build

up: ## bring up the demo enviroment.
	cd automation; \
	NUM_SERVERS=$(SITE_COUNT) BASE_TEST_DIR=$(BASE_TEST_DIR) LAB_DIR=$(LAB_DIR) GITEA_ADMIN_USER=$(GITEA_USERNAME) GITEA_ADMIN_PASS=$(GITEA_PASSWORD) GITEA_BASE_PORT=$(GITEA_BASE_PORT) APP_BASE_PORT=$(APP_BASE_PORT) ./setup-git.sh

down: ## Tear down the demo environment.
	cd automation; \
	NUM_SERVERS=$(SITE_COUNT) BASE_TEST_DIR=$(BASE_TEST_DIR) LAB_DIR=$(LAB_DIR) GITEA_ADMIN_USER=$(GITEA_USERNAME) GITEA_ADMIN_PASS=$(GITEA_PASSWORD) GITEA_BASE_PORT=$(GITEA_BASE_PORT) APP_BASE_PORT=$(APP_BASE_PORT) ./setup-git.sh clean
	rm -rf demo/.sites

cycle: ## Recreate the demo environment regardless if one already exists
	$(MAKE) down || true
	$(MAKE) up

demo: ## Recreate the demo environment and run all sites and UIs with logs in logs/
	$(MAKE) demo-down || true
	$(MAKE) demo-up

demo-up: ## Bring up the demo environment and run all sites and UIs with logs in logs/
	$(MAKE) demo-stop
	rm -rf "$(LOG_DIR)"
	mkdir -p "$(LOG_DIR)"
	$(MAKE) up
	@for site_id in $$(seq 1 $(SITE_COUNT)); do \
		echo "Starting site$${site_id}; logging to $(LOG_DIR)/site$${site_id}.log"; \
		setsid $(MAKE) --no-print-directory site$${site_id} > "$(LOG_DIR)/site$${site_id}.log" 2>&1 & \
		echo $$! >> "$(LOG_DIR)/demo.pids"; \
		echo "Starting ui$${site_id}; logging to $(LOG_DIR)/ui$${site_id}.log"; \
		setsid $(MAKE) --no-print-directory ui$${site_id} > "$(LOG_DIR)/ui$${site_id}.log" 2>&1 & \
		echo $$! >> "$(LOG_DIR)/demo.pids"; \
	done

demo-down: ## Stop demo sites/UIs, tear down the environment, and clear logs
	$(MAKE) demo-stop
	$(MAKE) down || true
	rm -rf "$(LOG_DIR)"

demo-stop:
	@if [ -f "$(LOG_DIR)/demo.pids" ]; then \
		while read -r pid; do \
			if [ -n "$${pid}" ]; then \
				kill -- -$${pid} >/dev/null 2>&1 || kill $${pid} >/dev/null 2>&1 || true; \
			fi; \
		done < "$(LOG_DIR)/demo.pids"; \
		sleep 1; \
		while read -r pid; do \
			if [ -n "$${pid}" ]; then \
				kill -9 -- -$${pid} >/dev/null 2>&1 || kill -9 $${pid} >/dev/null 2>&1 || true; \
			fi; \
		done < "$(LOG_DIR)/demo.pids"; \
	fi

site%: ## Run a site by index, for example: make site1
	@$(MAKE) site SITE_ID=$*

site: format
	@if [ -z "$(SITE_ID)" ]; then echo "Usage: make site SITE_ID=<n> or make site<n>"; exit 1; fi
	@repo_url="http://$(GITEA_USERNAME):$(GITEA_PASSWORD)@localhost:$$(( $(GITEA_BASE_PORT) + $(SITE_ID) ))/$(GITEA_USERNAME)/site-$(SITE_ID).git"; \
	RUST_LOG=info cargo run -- --url=0.0.0.0 --port=$$(( $(APP_BASE_PORT) + $(SITE_ID) )) --site-name=site-$(SITE_ID) --repo-path $(BASE_TEST_DIR)/site-$(SITE_ID) --repo-url "$${repo_url}" --publish-interval 15

farewell%: ## Send a farewell from a site by index, for example: make farewell1.
	@$(MAKE) farewell SITE_ID=$*

farewell: format
	@if [ -z "$(SITE_ID)" ]; then echo "Usage: make farewell SITE_ID=<n> or make farewell<n>"; exit 1; fi
	@repo_url="http://$(GITEA_USERNAME):$(GITEA_PASSWORD)@localhost:$$(( $(GITEA_BASE_PORT) + $(SITE_ID) ))/$(GITEA_USERNAME)/site-$(SITE_ID).git"; \
	RUST_LOG=info cargo run -- --farewell --site-name=site-$(SITE_ID) --repo-path $(BASE_TEST_DIR)/site-$(SITE_ID) --repo-url "$${repo_url}"

snoop: ## Snoop the nats bus
	RUST_LOG=info cargo run -- --snoop --site-name=snoop --repo-path $(BASE_TEST_DIR)/snoop

FORCE:

ui%: FORCE ## Run the frontend for a site, for example: make ui1
	@num='$*'; \
	case "$$num" in \
		''|*[!0-9]*) \
			echo "Usage: make ui<number>" >&2; \
			exit 1 ;; \
		*) \
			export site_id="$${num}"; \
			cd frontend; \
			npm install; \
			npm run dev -- --api-port=$$(( $(APP_BASE_PORT) + $${site_id} )) --port $$((5172 + $${site_id})); \
			;; \
	esac

uninstall%: FORCE ## Remove webhooks for a site by suffix, for example: make uninstall1
	@num='$*'; \
	case "$$num" in \
		''|*[!0-9]*) \
			echo "Usage: make uninstall<number>" >&2; \
			exit 1 ;; \
		*) \
			$(MAKE) --no-print-directory uninstall SITE_ID="$${num}"; \
			;; \
	esac

uninstall: FORCE
	@case "${SITE_ID}" in \
		''|*[!0-9]*) \
			echo "Error: SITE_ID must be set to a numeric site id. Use 'make uninstall<number>' or 'SITE_ID=<site_id> make uninstall'." >&2; \
			exit 1 ;; \
		*) \
			cd automation; \
			./remove-webhooks.sh $$((30000 + ${SITE_ID})) site-${SITE_ID} "http://host.docker.internal:808${SITE_ID}" fed-git password; \
			;; \
	esac

remotes: ## Show git remotes for all sites
	@for site_id in $$(seq 1 $(SITE_COUNT)); do \
		echo site-$${site_id}; \
		git -C $(BASE_TEST_DIR)/site-$${site_id} remote -v; \
	done

refs: ## Show git refs for all sites
	@for site_id in $$(seq 1 $(SITE_COUNT)); do \
		echo site-$${site_id}; \
		git -C $(BASE_TEST_DIR)/site-$${site_id} show-ref; \
	done

proposals: ## Show list of proposals
	@$(MAKE) refs | grep -e fed -e ^site | grep -B1 /fed/ | awk '{ if (NF == 2) print $$2; else print $$0 }'

format: ## Format code
	cargo +nightly fmt --all

lint: format ## lint code
	cargo clippy --all-targets --all-features -- -D warnings

test: lint ## run tests / coverage
	cargo llvm-cov --tests --html --open

fuzz: ## Run the http_endpoints fuzz target.
	cd fuzz && cargo fuzz run http_endpoints

fuzz-repro: ## Reproduce a fuzz artifact with ARTIFACT=<path>.
	cd fuzz && cargo fuzz run http_endpoints artifacts/http_endpoints/$(ARTIFACT)

kill: ## kill any orphaned sites run via: make site<n>
	pkill -f 'target/debug/[r]ef-based-fed' || true
