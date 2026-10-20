.DEFAULT_GOAL := run

QA_ROOT ?= /private/tmp/onecad-rust-desktop
FIXTURE ?= /private/tmp/onecad-rust-fixtures/constrained-plate.onecad
DOCUMENT ?= $(if $(wildcard $(QA_ROOT)/documents/$(notdir $(FIXTURE))),$(QA_ROOT)/documents/$(notdir $(FIXTURE)),$(FIXTURE))
WORKER_BUILD_DIR ?= $(CURDIR)/worker/build-native
WORKER ?= $(WORKER_BUILD_DIR)/onecad-worker
OCCT_ROOT ?= $(HOME)/.onecad-occt/8.0.1

.PHONY: run worker fixture

run: fixture
	TMPDIR="$(QA_ROOT)/temporary" \
	ONECAD_LOG_DIR="$(QA_ROOT)/logs" \
	ONECAD_LIBRARY_ROOT="$(QA_ROOT)/data/library" \
	ONECAD_WORKER_PATH="$(WORKER)" \
	cargo run -p onecad-ui -- --qa-root "$(QA_ROOT)" --open "$(DOCUMENT)"

worker:
	ONECAD_OCCT_ROOT="$(OCCT_ROOT)" ONECAD_WORKER_BUILD_DIR="$(WORKER_BUILD_DIR)" \
	./scripts/build-worker.sh Release

fixture: worker
	@if [ ! -f "$(DOCUMENT)" ] && [ "$(DOCUMENT)" != "$(FIXTURE)" ]; then \
		printf 'Missing DOCUMENT: %s\n' "$(DOCUMENT)" >&2; exit 1; \
	fi
	@if [ ! -f "$(DOCUMENT)" ]; then \
		ONECAD_REQUIRE_WORKER=1 ONECAD_WORKER_PATH="$(WORKER)" \
		ONECAD_NATIVE_PLATE_FIXTURE="$(FIXTURE)" \
		cargo test -p onecad-app --test native_plate_workflow synthetic_plate_native_workflow -- --nocapture; \
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
