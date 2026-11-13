# Local development with podman-compose.
#
# Note: podman-compose 1.6.0 does NOT rebuild images on `up` when they already
# exist. The `up`/`up-fresh` targets handle rebuilding explicitly.
#
# The app's entrypoint runs DB migrations before serving, so Postgres must be
# up before the app starts; see the postgresdb healthcheck.

COMPOSE    := podman-compose
COMPOSE_F  := -f Local.DockerCompose.yaml
APP_SVC    := app

.PHONY: help up up-fresh build down logs ps migrate migrate-local

help: ## Show this help
	@grep -E '^[a-zA-Z_-]+:.*?## ' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[36m%-14s\033[0m %s\n", $$1, $$2}'

up: ## Build (cache) and start the full stack, recreating changed containers
	$(COMPOSE) $(COMPOSE_F) up -d --build --force-recreate

up-fresh: ## Start stack with a full no-cache rebuild of the app image
	$(COMPOSE) $(COMPOSE_F) build --no-cache $(APP_SVC)
	$(COMPOSE) $(COMPOSE_F) up -d --force-recreate

build: ## Build the app image (cached)
	$(COMPOSE) $(COMPOSE_F) build $(APP_SVC)

down: ## Stop and remove the stack containers
	$(COMPOSE) $(COMPOSE_F) down

logs: ## Tail logs for the app
	$(COMPOSE) $(COMPOSE_F) logs -f $(APP_SVC)

ps: ## List stack containers
	$(COMPOSE) $(COMPOSE_F) ps

migrate: ## Migrations run automatically from the app entrypoint; nothing to do
	@echo "Migrations run on app startup. Use 'make up' (or 'make migrate-local')."

migrate-local: ## Run migrations against a local postgres (uses .env vars)
	cd migrations && cargo run
