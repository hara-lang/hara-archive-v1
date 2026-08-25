SHELL := /usr/bin/env bash

.DEFAULT_GOAL := help

MAVEN ?= mvn
CARGO ?= cargo
NPM ?= npm
MKDOCS ?= mkdocs
ARGS ?=

CORE_DIR := $(dir $(abspath $(lastword $(MAKEFILE_LIST))))
HARA_ROOT := $(abspath $(CORE_DIR)/..)
HARA_WORKSPACE_ROOT ?= $(abspath $(HARA_ROOT)/../..)
HARA_SPECS_REGISTRY ?= $(abspath $(HARA_ROOT)/../hara-specs-registry)
JAVA_SPECS_ARGS := -Dhara.specs.registry=$(HARA_SPECS_REGISTRY)

TRUFFLE_JAR := java/target/hara-truffle.jar
TRUFFLE_NATIVE := target/hara-truffle
RUST_MANIFEST := rust/Cargo.toml
RUST_RAW_MANIFEST := rust/raw/Cargo.toml
RUST_DEBUG := rust/target/debug/hara
RUST_RELEASE := rust/target/release/hara
WEB_DIR := rust/web
CHROME_DIR := ../../application/greenways-os/extension/hara-chrome
HARA_WWW_DIR := ../website/hara-www
HARA_BENCH_SCRIPTS := ../../../website/hara-benchmarks/scripts/hara-runtime

.PHONY: help all build-all check-all test-all clean \
        java java-offline java-headless java-build java-test java-specs java-conformance \
        rust rust-offline rust-headless rust-build rust-build-release rust-release \
        rust-test rust-test-main rust-test-library rust-test-cli \
        rust-test-integration rust-test-examples rust-test-doc rust-test-ignored rust-test-crates \
        rust-test-abi rust-test-hta rust-test-vm rust-test-compiler rust-test-conformance-groups \
        rust-layout lib-test \
        native-image native-image-run native-image-offline \
        wasm wasm-build wasm-build-all wasm-web wasm-web-build wasm-browser-build \
        wasm-test wasm-test-raw wasm-test-raw-unit wasm-test-raw-integration \
        wasm-test-node wasm-test-hta wasm-test-exceptions wasm-test-runtime \
        wasm-test-studio wasm-test-browser-sdk wasm-test-browser \
        web-install \
        chrome-install chrome-build chrome-test \
        docs-build www-build \
        runtime-benchmark vm-bb-benchmark truffle-benchmark parity-benchmark

help: ## Show the available runtime targets
	@echo 'Hara runtimes'
	@echo
	@echo '  make java [ARGS="..."]          JVM Truffle REPL'
	@echo '  make java-offline               JVM Truffle REPL without RESP'
	@echo '  make java-headless              JVM Truffle RESP server'
	@echo '  make rust [ARGS="..."]          Rust native REPL'
	@echo '  make rust-offline               Rust native REPL without RESP'
	@echo '  make rust-headless              Rust native RESP server'
	@echo '  make native-image-run           GraalVM native-image REPL'
	@echo '  make native-image-offline       GraalVM native-image REPL without RESP'
	@echo '  make wasm                       Build the raw WASM module (no terminal REPL)'
	@echo '  make wasm-web                   Build browser WASM package'
	@echo
	@echo 'Build and verification'
	@echo
	@echo '  make build-all                  Build JVM, Rust release, and raw WASM'
	@echo '  make check-all                  Run core layout and runtime test suites'
	@echo '  make java-test                  Run fast JVM tests without registry conformance'
	@echo '  make java-specs                 Run only registry-backed JVM tests'
	@echo '  make java-conformance           Run the complete JVM suite, including registry conformance'
	@echo '  make rust-test                  Run every native Rust test slice'
	@echo '  make rust-test-main             Run main crate library, CLI, integration, example, and doc slices'
	@echo '  make rust-test-crates           Run ABI, HTA codec, VM, and compiler crate slices'
	@echo '  make rust-test-ignored          Run artifact-dependent ignored Rust tests'
	@echo '  make rust-test-conformance-groups ARGS="vm whole-wasm"'
	@echo '                                  Run issue #1047 Rust failure groups'
	@echo '  make rust-test-<slice>          Run one Rust slice (see README for names)'
	@echo '  make wasm-test                  Run every raw, Node, SDK, and browser WASM test slice'
	@echo '  make wasm-test-raw              Run raw WASM unit and integration slices'
	@echo '  make wasm-test-node             Run HTA, exception, runtime, and Studio Node slices'
	@echo '  make wasm-test-<slice>          Run one WASM slice (see README for names)'
	@echo '  make rust-layout                Check Rust module layout'
	@echo '  make lib-test                   Run portable .hal library tests'
	@echo '  make chrome-build               Build the Chrome extension'
	@echo '  make chrome-test                Test the Chrome extension'
	@echo '  make docs-build                 Build documentation strictly'
	@echo '  make runtime-benchmark [ARGS="..."]'
	@echo '                                  Run the cross-runtime benchmark'
	@echo '  make vm-bb-benchmark [ARGS="..."]'
	@echo '                                  Compare compact VM tiers with bb'
	@echo '  make clean                      Remove Maven and Cargo build output'
	@echo
	@echo 'Examples'
	@echo
	@echo '  make java ARGS="eval '\''(+ 19 23)'\''"'
	@echo '  make rust ARGS="run lib/examples/hello.hal"'
	@echo '  make rust ARGS="remote 127.0.0.1:1311"'

all: help

java-build: $(TRUFFLE_JAR) ## Build the JVM Truffle runtime

$(TRUFFLE_JAR): java/pom.xml
	$(MAVEN) -f java/pom.xml -Ptruffle $(JAVA_SPECS_ARGS) -DskipTests package

java: java-build ## Run the JVM Truffle runtime
	./hara $(ARGS)

java-offline: java-build ## Run the JVM Truffle REPL without RESP
	./hara --offline $(ARGS)

java-headless: java-build ## Run the JVM Truffle RESP server
	./hara headless $(ARGS)

java-test: ## Run the JVM Truffle test suite
	$(MAVEN) -f java/pom.xml -Ptruffle $(JAVA_SPECS_ARGS) test

java-specs: ## Run only registry-backed JVM tests
	HARA_WORKSPACE_ROOT="$(HARA_WORKSPACE_ROOT)" HARA_SPECS_REGISTRY="$(HARA_SPECS_REGISTRY)" \
	$(MAVEN) -f java/pom.xml -Ptruffle -Pconformance $(JAVA_SPECS_ARGS) \
		-Dgroups=hara.spec.RegistryConformance test

java-conformance: ## Run the complete JVM suite, including registry conformance
	HARA_WORKSPACE_ROOT="$(HARA_WORKSPACE_ROOT)" HARA_SPECS_REGISTRY="$(HARA_SPECS_REGISTRY)" \
	$(MAVEN) -f java/pom.xml -Ptruffle -Pconformance $(JAVA_SPECS_ARGS) test

rust-build: ## Build the Rust native runtime
	$(CARGO) build --manifest-path $(RUST_MANIFEST) --bin hara

rust-build-release: ## Build the optimized Rust native runtime
	$(CARGO) build --release --manifest-path $(RUST_MANIFEST) --bin hara

rust: rust-build ## Run the Rust native runtime
	$(RUST_DEBUG) $(ARGS)

rust-offline: rust-build ## Run the Rust native REPL without RESP
	$(RUST_DEBUG) --offline $(ARGS)

rust-headless: rust-build ## Run the Rust native RESP server
	$(RUST_DEBUG) headless $(ARGS)

rust-release: rust-build-release ## Run optimized Rust native
	$(RUST_RELEASE) $(ARGS)

rust-test: rust-test-main rust-test-crates ## Run every native Rust test slice

rust-test-main: rust-test-library rust-test-cli rust-test-integration rust-test-examples rust-test-doc ## Run all main hara-wasm crate test slices

rust-test-library: ## Run main hara-wasm library unit tests
	$(CARGO) test --manifest-path $(RUST_MANIFEST) -p hara-wasm --lib

rust-test-cli: ## Run unit tests embedded in every native CLI binary
	$(CARGO) test --manifest-path $(RUST_MANIFEST) -p hara-wasm --bins

rust-test-integration: ## Run main hara-wasm integration test targets
	$(CARGO) test --manifest-path $(RUST_MANIFEST) -p hara-wasm --test '*'

rust-test-examples: ## Compile and run main hara-wasm example test targets
	$(CARGO) test --manifest-path $(RUST_MANIFEST) -p hara-wasm --examples

rust-test-doc: ## Run main hara-wasm documentation tests
	$(CARGO) test --manifest-path $(RUST_MANIFEST) -p hara-wasm --doc

rust-test-ignored: ## Run ignored tests that require external runtime artifacts
	$(CARGO) test --manifest-path $(RUST_MANIFEST) -p hara-wasm -- --ignored

rust-test-crates: rust-test-abi rust-test-hta rust-test-vm rust-test-compiler ## Run every supporting Rust workspace crate

rust-test-abi: ## Run the dependency-free package ABI crate tests
	$(CARGO) test --manifest-path $(RUST_MANIFEST) -p hara-abi

rust-test-hta: ## Run the canonical HTA codec crate tests
	$(CARGO) test --manifest-path $(RUST_MANIFEST) -p hara-hta

rust-test-vm: ## Run the VM-only runtime crate tests
	$(CARGO) test --manifest-path $(RUST_MANIFEST) -p hara-vm

rust-test-compiler: ## Run the compiler facade crate tests
	$(CARGO) test --manifest-path $(RUST_MANIFEST) -p hara-compiler

rust-test-conformance-groups: ## Run issue #1047 Rust failure groups
	../scripts/runtime/run-rust-conformance-groups $(if $(strip $(ARGS)),$(ARGS),all)

rust-layout: ## Check the Rust module layout
	bash rust/scripts/check-layout.sh

lib-test: ## Run portable .hal library tests
	../scripts/runtime/run-lib-tests

native-image: $(TRUFFLE_NATIVE) ## Build the GraalVM native-image runtime

$(TRUFFLE_NATIVE): java/pom.xml ../scripts/runtime/build-truffle-native
	../scripts/runtime/build-truffle-native

native-image-run: native-image ## Run the GraalVM native-image runtime
	$(TRUFFLE_NATIVE) $(ARGS)

native-image-offline: native-image ## Run native-image without RESP
	$(TRUFFLE_NATIVE) --offline $(ARGS)

wasm: wasm-build ## Build the raw WASM runtime

wasm-build: ## Build the raw WASM module
	../scripts/runtime/build-hara-wasm-raw

wasm-build-all: ## Build core, VM, and tracing raw WASM variants
	../scripts/runtime/build-hara-wasm-raw all

wasm-web: wasm-web-build ## Build the browser WASM runtime

wasm-web-build: ## Build the browser WASM package
	../scripts/runtime/build-hara-wasm-web

wasm-browser-build: ## Build VM and full browser SDK WASM variants
	$(NPM) --prefix $(WEB_DIR) run build:browser:wasm

wasm-test: wasm-test-raw wasm-test-node wasm-test-browser-sdk wasm-test-browser ## Run every WASM test slice

wasm-test-raw: wasm-test-raw-unit wasm-test-raw-integration ## Run raw WASM crate unit and integration tests

wasm-test-raw-unit: ## Run raw WASM crate unit tests
	$(CARGO) test --manifest-path $(RUST_RAW_MANIFEST) --lib

wasm-test-raw-integration: ## Run raw WASM crate integration test targets
	$(CARGO) test --manifest-path $(RUST_RAW_MANIFEST) --test '*'

wasm-test-node: wasm-test-hta wasm-test-exceptions wasm-test-runtime wasm-test-studio ## Run all Node-hosted WASM slices

wasm-test-hta: ## Run HTA codec and sandbox Node tests
	$(NPM) --prefix $(WEB_DIR) run test:hta

wasm-test-exceptions: wasm-build-all ## Run raw WASM exception conformance in Node
	$(NPM) --prefix $(WEB_DIR) run test:exceptions

wasm-test-runtime: wasm-web-build ## Run portable code through browser WASM in Node
	$(NPM) --prefix $(WEB_DIR) run test:wasm-runtime

wasm-test-studio: wasm-build ## Run Studio Node tests against current raw WASM
	$(NPM) --prefix $(WEB_DIR) run test:studio

wasm-test-browser-sdk: ## Build and run the publishable browser SDK Node tests
	$(NPM) --prefix $(WEB_DIR) run test:browser-sdk

wasm-test-browser: wasm-build-all wasm-test-browser-sdk wasm-web-build ## Run the Playwright browser WASM suite
	$(NPM) --prefix $(WEB_DIR) run test:browser

web-install: ## Install browser-loader and Studio dependencies
	$(NPM) --prefix $(WEB_DIR) ci

chrome-install: ## Install Chrome extension dependencies
	$(NPM) --prefix $(CHROME_DIR) ci

chrome-build: ## Build the Chrome extension
	$(NPM) --prefix $(CHROME_DIR) run build

chrome-test: ## Test the Chrome extension
	$(NPM) --prefix $(CHROME_DIR) test

docs-build: ## Build the hara-www documentation site
	npm run build --prefix $(HARA_WWW_DIR)

runtime-benchmark: ## Run the shared cross-runtime benchmark corpus
	$(HARA_BENCH_SCRIPTS)/run-runtime-benchmarks $(ARGS)

vm-bb-benchmark: ## Compare bytecode/native trace VM tiers with babashka
	$(HARA_BENCH_SCRIPTS)/run-vm-bb-benchmark $(ARGS)

truffle-benchmark: ## Run the JVM Truffle benchmark
	$(HARA_BENCH_SCRIPTS)/run-truffle-benchmark $(ARGS)

parity-benchmark: ## Benchmark Rust WASM and Truffle parity
	$(HARA_BENCH_SCRIPTS)/run-wasm-truffle-parity-benchmark $(ARGS)

build-all: java-build rust-build-release wasm-build ## Build all portable runtime artifacts

test-all: java-test rust-test ## Run Java and Rust test suites

check-all: rust-layout java-test rust-test wasm-test-raw lib-test wasm-test-hta wasm-test-studio ## Run core repository checks

clean: ## Remove generated build output
	$(MAVEN) -f java/pom.xml clean
	$(CARGO) clean --manifest-path $(RUST_MANIFEST)
	$(CARGO) clean --manifest-path $(RUST_RAW_MANIFEST)
