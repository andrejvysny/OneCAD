.DEFAULT_GOAL := run

QA_ROOT ?= /private/tmp/onecad-rust-desktop
FIXTURE ?= /private/tmp/onecad-rust-fixtures/constrained-plate.onecad
# Empty opens the start page; `make qa` opens the constrained-plate sample.
DOCUMENT ?=
QA_DOCUMENT ?= $(if $(wildcard $(QA_ROOT)/documents/$(notdir $(FIXTURE))),$(QA_ROOT)/documents/$(notdir $(FIXTURE)),$(FIXTURE))
WORKER_BUILD_DIR ?= $(CURDIR)/worker/build-native
WORKER ?= $(WORKER_BUILD_DIR)/onecad-worker
OCCT_ROOT ?= $(HOME)/.onecad-occt/8.0.1
TARGET_DIR ?= $(if $(CARGO_TARGET_DIR),$(CARGO_TARGET_DIR),$(CURDIR)/target)
APP_BINARY ?= $(TARGET_DIR)/debug/onecad-ui

.PHONY: build run dev qa worker fixture

build: worker
	cargo build --target-dir "$(TARGET_DIR)" -p onecad-ui

dev: build
	$(MAKE) run

qa: build fixture
	$(MAKE) run DOCUMENT="$(QA_DOCUMENT)"

run:
	@test -x "$(APP_BINARY)" || { printf 'Missing app: %s. Run make build first.\n' "$(APP_BINARY)" >&2; exit 1; }
	@test -x "$(WORKER)" || { printf 'Missing worker: %s. Run make build first.\n' "$(WORKER)" >&2; exit 1; }
	@test -z "$(DOCUMENT)" || test -f "$(DOCUMENT)" || { printf 'Missing document: %s. Run make qa or set DOCUMENT.\n' "$(DOCUMENT)" >&2; exit 1; }
	TMPDIR="$(QA_ROOT)/temporary" \
	ONECAD_LOG_DIR="$(QA_ROOT)/logs" \
	ONECAD_LIBRARY_ROOT="$(QA_ROOT)/data/library" \
	ONECAD_WORKER_PATH="$(WORKER)" \
	"$(APP_BINARY)" --qa-root "$(QA_ROOT)" $(if $(DOCUMENT),--open "$(DOCUMENT)")

worker:
	ONECAD_OCCT_ROOT="$(OCCT_ROOT)" ONECAD_WORKER_BUILD_DIR="$(WORKER_BUILD_DIR)" \
	./scripts/build-worker.sh Release

fixture: worker
	@if [ ! -f "$(QA_DOCUMENT)" ] && [ "$(QA_DOCUMENT)" != "$(FIXTURE)" ]; then \
		printf 'Missing QA_DOCUMENT: %s\n' "$(QA_DOCUMENT)" >&2; exit 1; \
	fi
	@if [ ! -f "$(QA_DOCUMENT)" ]; then \
		ONECAD_REQUIRE_WORKER=1 ONECAD_WORKER_PATH="$(WORKER)" \
		ONECAD_NATIVE_PLATE_FIXTURE="$(FIXTURE)" \
		cargo test --target-dir "$(TARGET_DIR)" -p onecad-app --test native_plate_workflow synthetic_plate_native_workflow -- --nocapture; \
	fi

.PHONY: check-tooling test-tooling package
check-tooling:
	cargo xtask check-rust-layers
	cargo xtask verify-modeling-coverage
	cargo xtask verify-modeling-contracts
	cargo xtask check-hygiene
	cargo xtask bake-cad-icons --check

test-tooling:
	cargo test -p xtask

package:
	./scripts/package-macos.sh
