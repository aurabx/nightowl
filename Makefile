# Phantom — DICOM service tester (Tauri + React + Rust)
#
# Usage: `make help` lists every target.
# Targets are documented inline with `## <description>` after the colon;
# the `help` target greps them out so this file stays self-documenting.

SHELL := /bin/bash
.DEFAULT_GOAL := help

# --- paths -------------------------------------------------------------

# Cargo workspace root. Members: `src-tauri` (Tauri desktop app) and
# `nightowl-cli` (standalone CLI). `--workspace` runs the action across
# both members in one invocation.
CARGO_WORKSPACE_ARGS := --workspace

# Default source image for icon regeneration; override with
#   make icons ICON_SRC=path/to/source.png
ICON_SRC ?= /tmp/phantom-source.png

# Override DCMTK target host/port when smoke-testing against a peer that
# is not this app, e.g.
#   make echoscu AET=PHANTOM HOST=192.168.1.20 PORT=11112
AET  ?= PHANTOM
AEC  ?= TESTSCU
HOST ?= localhost
PORT ?= 11112


# --- meta --------------------------------------------------------------

.PHONY: help
help: ## Show this help.
	@printf "\nPhantom — available make targets:\n\n"
	@awk 'BEGIN {FS = ":.*?## "} /^[a-zA-Z_-]+:.*?## / { printf "  \033[1;36m%-18s\033[0m %s\n", $$1, $$2 }' $(MAKEFILE_LIST)
	@printf "\nOverridable variables: AET=$(AET) AEC=$(AEC) HOST=$(HOST) PORT=$(PORT) ICON_SRC=$(ICON_SRC)\n\n"


# --- dependencies ------------------------------------------------------

.PHONY: install
install: ## Install npm dependencies.
	npm install

node_modules: package.json package-lock.json
	npm install
	@touch node_modules


# --- run ---------------------------------------------------------------

.PHONY: dev
dev: node_modules ## Run the desktop app in dev mode (hot reload).
	npm run tauri dev

.PHONY: web
web: node_modules ## Run the frontend dev server only (no Tauri shell).
	npm run dev


# --- build -------------------------------------------------------------

.PHONY: build
build: node_modules ## Build the release bundle (Phantom.app).
	npm run tauri build

.PHONY: build-web
build-web: node_modules ## Type-check + bundle the frontend only.
	npm run build


# --- quality gates -----------------------------------------------------

.PHONY: check
check: check-rust check-web ## Compile-check both halves.

.PHONY: check-rust
check-rust: ## cargo check across the workspace.
	cargo check $(CARGO_WORKSPACE_ARGS)

.PHONY: check-web
check-web: node_modules ## tsc + vite type-check (no emit).
	npx tsc -b

.PHONY: test
test: test-rust ## Run all tests.

.PHONY: test-rust
test-rust: ## Run Rust unit + doc + CLI integration tests.
	cargo test $(CARGO_WORKSPACE_ARGS)

.PHONY: lint
lint: lint-rust ## Run linters.

.PHONY: lint-rust
lint-rust: ## cargo clippy with warnings as errors, across the workspace.
	cargo clippy $(CARGO_WORKSPACE_ARGS) --all-targets -- -D warnings

.PHONY: fmt
fmt: ## Auto-format Rust source.
	cargo fmt $(CARGO_WORKSPACE_ARGS)

.PHONY: fmt-check
fmt-check: ## Verify Rust formatting (no rewrite).
	cargo fmt $(CARGO_WORKSPACE_ARGS) -- --check


# --- icons -------------------------------------------------------------

.PHONY: icons
icons: node_modules ## Regenerate the Tauri icon set from ICON_SRC (override the var to point elsewhere).
	@if [ ! -f "$(ICON_SRC)" ]; then \
		echo "ERROR: $(ICON_SRC) not found."; \
		echo "Run 'make icons-placeholder' to write a default 1024x1024 source,"; \
		echo "or set ICON_SRC=path/to/your.png."; \
		exit 1; \
	fi
	npx tauri icon $(ICON_SRC)

define ICON_PLACEHOLDER_PY
import struct, zlib, sys

def write_png(path, width, height, rgb):
    sig = b"\x89PNG\r\n\x1a\n"
    ihdr = struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)
    def chunk(tag, data):
        return struct.pack(">I", len(data)) + tag + data + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)
    raw = b"".join(b"\x00" + bytes(rgb) * width for _ in range(height))
    blob = sig + chunk(b"IHDR", ihdr) + chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b"")
    with open(path, "wb") as f:
        f.write(blob)

path = sys.argv[1]
write_png(path, 1024, 1024, [15, 23, 42])
print(f"wrote {path}")
endef
export ICON_PLACEHOLDER_PY

.PHONY: icons-placeholder
icons-placeholder: ## Write a solid-colour 1024x1024 PNG to ICON_SRC.
	@python3 -c "$$ICON_PLACEHOLDER_PY" $(ICON_SRC)


# --- DICOM smoke tests (DCMTK) -----------------------------------------
#
# These require DCMTK to be installed: `brew install dcmtk`. They are
# advisory — the listed milestone makes each one return success.

.PHONY: echoscu
echoscu: ## C-ECHO smoke against AET@HOST:PORT (works from M3).
	echoscu -v -aec $(AET) -aet $(AEC) $(HOST) $(PORT)

.PHONY: findscu
findscu: ## C-FIND STUDY-level query (works from M4).
	findscu -v -P -k QueryRetrieveLevel=STUDY -k PatientID -k StudyInstanceUID \
		-aec $(AET) -aet $(AEC) $(HOST) $(PORT)

.PHONY: storescu
storescu: ## C-STORE one file (FILE=path.dcm) to AET@HOST:PORT (works from M5).
	@if [ -z "$(FILE)" ]; then echo "Usage: make storescu FILE=path/to/sample.dcm"; exit 2; fi
	storescu -v -aec $(AET) -aet $(AEC) $(HOST) $(PORT) $(FILE)


# --- cleanup -----------------------------------------------------------

.PHONY: kill-dev
kill-dev: ## Force-kill any lingering dev processes and free relevant ports.
	@for proc in phantom vite tauri cargo-tauri node; do \
		pkill -9 -f $$proc 2>/dev/null || true; \
	done
	@for port in 5173 11112 11113; do \
		pids=$$(lsof -ti :$$port 2>/dev/null); \
		if [ -n "$$pids" ]; then \
			echo "killing $$pids on port $$port"; \
			kill -9 $$pids 2>/dev/null || true; \
		fi; \
	done
	@sleep 1
	@for port in 5173 11112 11113; do \
		if lsof -i :$$port 2>/dev/null | grep -q LISTEN; then \
			echo "WARNING: port $$port still in use"; \
		else \
			echo "port $$port clear"; \
		fi; \
	done

.PHONY: clean
clean: ## Remove build artifacts (target/, dist/) but keep node_modules.
	rm -rf target dist src-tauri/gen src-tauri/target

.PHONY: distclean
distclean: clean ## clean + also drop node_modules and lockfile-cached state.
	rm -rf node_modules

.PHONY: reset-config
reset-config: ## Delete the persisted app config (forces defaults on next launch).
	rm -f "$$HOME/Library/Application Support/cloud.aurabox.phantom/config.json"
	@echo "Config reset. Next launch will use defaults."
