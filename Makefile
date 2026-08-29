SHELL := /bin/sh

PREFIX ?= $(HOME)/.local
DESTDIR ?=
BINDIR ?= $(PREFIX)/bin
DATADIR ?= $(PREFIX)/share
HARA_DATADIR ?= $(DATADIR)/hara
HARA_LITE_DATADIR ?= $(DATADIR)/hara-lite

CARGO ?= cargo
MVN ?= mvn
INSTALL ?= install

RUST_MANIFEST ?= core/rust/Cargo.toml
RUST_BINARY ?= core/rust/target/release/hara
RUST_LITE_BINARY ?= core/rust/target/release/hara-lite
TRUFFLE_POM ?= core/java/pom.xml
TRUFFLE_JAR ?= core/java/target/hara-truffle.jar
TRUFFLE_NATIVE_BINARY ?= core/target/hara-truffle
HARA_SPECS_REGISTRY ?= $(abspath ../hara-specs-registry)

.PHONY: all build build-rust build-rust-lite build-truffle build-truffle-native \
  install install-rust install-rust-files \
  install-rust-lite install-rust-lite-files \
  install-truffle install-truffle-files \
  install-truffle-native install-truffle-native-files install-all \
  uninstall check-install \
  check-rust-java-parity-specs \
  test-rust-java-parity \
  test-rust-java-parity-interfaces \
  test-rust-java-parity-native \
  test-rust-java-parity-halc \
  test-rust-java-parity-hta \
  help

all: build

build: build-rust

build-rust:
	$(CARGO) build --release --manifest-path "$(RUST_MANIFEST)" --bin hara

build-rust-lite:
	$(CARGO) build --release --manifest-path "$(RUST_MANIFEST)" --no-default-features --features direct-native --bin hara-lite

build-truffle:
	$(MVN) -f "$(TRUFFLE_POM)" -Ptruffle -DskipTests package

build-truffle-native:
	./scripts/runtime/build-truffle-native

install: install-rust

install-rust: build-rust
	@$(MAKE) --no-print-directory install-rust-files

install-rust-files:
	@test -x "$(RUST_BINARY)" || { \
	  printf 'Hara Rust binary not found or not executable: %s\n' "$(RUST_BINARY)" >&2; \
	  exit 1; \
	}
	$(INSTALL) -d "$(DESTDIR)$(BINDIR)" "$(DESTDIR)$(HARA_LITE_DATADIR)/lib"
	$(INSTALL) -m 755 "$(RUST_BINARY)" "$(DESTDIR)$(BINDIR)/hara"
	$(INSTALL) -m 644 core/project.edn "$(DESTDIR)$(HARA_LITE_DATADIR)/project.edn"
	cp -R core/lib/. "$(DESTDIR)$(HARA_LITE_DATADIR)/lib/"
	@printf 'Installed Hara Rust runtime: %s\n' "$(BINDIR)/hara"
	@printf 'Installed Hara library project: %s\n' "$(HARA_LITE_DATADIR)"

install-rust-lite: build-rust-lite
	@$(MAKE) --no-print-directory install-rust-lite-files

install-rust-lite-files:
	@test -x "$(RUST_LITE_BINARY)" || { \
	  printf 'Hara Rust lite binary not found or not executable: %s\n' "$(RUST_LITE_BINARY)" >&2; \
	  exit 1; \
	}
	$(INSTALL) -d "$(DESTDIR)$(BINDIR)" "$(DESTDIR)$(HARA_LITE_DATADIR)/lib"
	$(INSTALL) -m 755 "$(RUST_LITE_BINARY)" "$(DESTDIR)$(BINDIR)/hara-lite"
	$(INSTALL) -m 644 core/project.edn "$(DESTDIR)$(HARA_LITE_DATADIR)/project.edn"
	cp -R core/lib/. "$(DESTDIR)$(HARA_LITE_DATADIR)/lib/"
	@printf 'Installed Hara Rust lite runtime: %s\n' "$(BINDIR)/hara-lite"
	@printf 'Installed Hara lite project: %s\n' "$(HARA_LITE_DATADIR)"

install-truffle: build-truffle
	@$(MAKE) --no-print-directory install-truffle-files

install-truffle-files:
	@test -r "$(TRUFFLE_JAR)" || { \
	  printf 'Hara Truffle JAR not found or not readable: %s\n' "$(TRUFFLE_JAR)" >&2; \
	  exit 1; \
	}
	$(INSTALL) -d "$(DESTDIR)$(BINDIR)" "$(DESTDIR)$(HARA_DATADIR)"
	$(INSTALL) -m 644 "$(TRUFFLE_JAR)" "$(DESTDIR)$(HARA_DATADIR)/hara-truffle.jar"
	@{ \
	  printf '%s\n' '#!/bin/sh'; \
	  printf '%s\n' 'set -eu'; \
	  printf '%s\n' 'JAR="$${HARA_RUNTIME_JAR:-$(HARA_DATADIR)/hara-truffle.jar}"'; \
	  printf '%s\n' 'JAVA="$${HARA_JAVA:-java}"'; \
	  printf '%s\n' 'if [ ! -r "$$JAR" ]; then'; \
	  printf '%s\n' '  printf "Hara runtime JAR not found: %s\\n" "$$JAR" >&2'; \
	  printf '%s\n' '  exit 2'; \
	  printf '%s\n' 'fi'; \
	  printf '%s\n' 'if ! command -v "$$JAVA" >/dev/null 2>&1; then'; \
	  printf '%s\n' '  printf "Java runtime not found: %s\\n" "$$JAVA" >&2'; \
	  printf '%s\n' '  exit 2'; \
	  printf '%s\n' 'fi'; \
	  printf '%s\n' 'exec "$$JAVA" -jar "$$JAR" "$$@"'; \
	} > "$(DESTDIR)$(BINDIR)/hara-truffle"
	chmod 755 "$(DESTDIR)$(BINDIR)/hara-truffle"
	@printf 'Installed Hara Truffle launcher: %s\n' "$(BINDIR)/hara-truffle"
	@printf 'Installed Hara Truffle runtime: %s\n' "$(HARA_DATADIR)/hara-truffle.jar"

install-truffle-native: build-truffle-native
	@$(MAKE) --no-print-directory install-truffle-native-files

install-truffle-native-files:
	@test -x "$(TRUFFLE_NATIVE_BINARY)" || { \
	  printf 'Hara Truffle native image not found or not executable: %s\n' "$(TRUFFLE_NATIVE_BINARY)" >&2; \
	  exit 1; \
	}
	$(INSTALL) -d "$(DESTDIR)$(BINDIR)"
	$(INSTALL) -m 755 "$(TRUFFLE_NATIVE_BINARY)" "$(DESTDIR)$(BINDIR)/hara-truffle-native"
	@printf 'Installed Hara Truffle native image: %s\n' "$(BINDIR)/hara-truffle-native"

install-all: install-rust install-truffle

uninstall:
	rm -f "$(DESTDIR)$(BINDIR)/hara" \
	      "$(DESTDIR)$(BINDIR)/hara-lite" \
	      "$(DESTDIR)$(BINDIR)/hara-truffle" \
	      "$(DESTDIR)$(BINDIR)/hara-truffle-native" \
	      "$(DESTDIR)$(HARA_DATADIR)/hara-truffle.jar"
	rm -rf "$(DESTDIR)$(HARA_LITE_DATADIR)"
	@rmdir "$(DESTDIR)$(HARA_DATADIR)" 2>/dev/null || true
	@rmdir "$(DESTDIR)$(BINDIR)" 2>/dev/null || true

check-install:
	sh scripts/runtime/test-make-install.sh

check-rust-java-parity-specs:
	@test -d "$(HARA_SPECS_REGISTRY)" && { \
	  test -f "$(HARA_SPECS_REGISTRY)/spec-manifest.json" || \
	  test -f "$(HARA_SPECS_REGISTRY)/registry-index.json"; \
	} || { \
	  printf 'Hara specs registry is unavailable: %s\\n' "$(HARA_SPECS_REGISTRY)" >&2; \
	  printf 'Set HARA_SPECS_REGISTRY to a valid hara-specs-registry checkout.\\n' >&2; \
	  exit 2; \
	}

test-rust-java-parity: check-rust-java-parity-specs
	@$(MAKE) --no-print-directory \
	  HARA_SPECS_REGISTRY="$(HARA_SPECS_REGISTRY)" \
	  test-rust-java-parity-interfaces
	@$(MAKE) --no-print-directory \
	  HARA_SPECS_REGISTRY="$(HARA_SPECS_REGISTRY)" \
	  test-rust-java-parity-native
	@$(MAKE) --no-print-directory \
	  HARA_SPECS_REGISTRY="$(HARA_SPECS_REGISTRY)" \
	  test-rust-java-parity-halc
	@$(MAKE) --no-print-directory \
	  HARA_SPECS_REGISTRY="$(HARA_SPECS_REGISTRY)" \
	  test-rust-java-parity-hta

test-rust-java-parity-interfaces: check-rust-java-parity-specs
	HARA_SPECS_REGISTRY="$(HARA_SPECS_REGISTRY)" \
	  $(CARGO) test --locked --manifest-path "$(RUST_MANIFEST)" --test protocol_declarations
	$(MVN) -B -Ptruffle,conformance \
	  -Dhara.specs.registry="$(HARA_SPECS_REGISTRY)" \
	  -Dtest=HaraDeclarationCoverageTest test --file "$(TRUFFLE_POM)"

test-rust-java-parity-native: check-rust-java-parity-specs
	HARA_SPECS_REGISTRY="$(HARA_SPECS_REGISTRY)" \
	  $(CARGO) test --locked --manifest-path "$(RUST_MANIFEST)" \
	  native_behavioral_conformance_tests:: --lib -- --nocapture
	$(MVN) -B -Ptruffle,conformance \
	  -Dhara.specs.registry="$(HARA_SPECS_REGISTRY)" \
	  -Dtest=NativeMethodParityTest,NativeBehavioralConformanceTest \
	  test --file "$(TRUFFLE_POM)"

test-rust-java-parity-halc: check-rust-java-parity-specs
	HARA_SPECS_REGISTRY="$(HARA_SPECS_REGISTRY)" \
	  $(CARGO) test --locked --manifest-path "$(RUST_MANIFEST)" --lib \
	  kernel::halc::tests::registry_golden_matches_rust_encoding -- --nocapture
	$(MVN) -B -Ptruffle,conformance \
	  -Dhara.specs.registry="$(HARA_SPECS_REGISTRY)" \
	  -Dtest=HalcArtifactTest#registryGoldenMatchesJavaEncoding \
	  test --file "$(TRUFFLE_POM)"

test-rust-java-parity-hta: check-rust-java-parity-specs
	HARA_SPECS_REGISTRY="$(HARA_SPECS_REGISTRY)" \
	  $(CARGO) test --locked --manifest-path "$(RUST_MANIFEST)" \
	  --test hta_portable registry_golden_vector_matches_rust_encoding
	$(MVN) -B -Ptruffle,conformance \
	  -Dhara.specs.registry="$(HARA_SPECS_REGISTRY)" \
	  -Dtest=HtaValueCodecTest#registryGoldenVectorMatchesJavaEncoding \
	  test --file "$(TRUFFLE_POM)"

help:
	@printf '%s\n' \
	  'make install            Build and install the Rust CLI as hara' \
	  'make install-rust-lite  Build and install the dependency-light CLI as hara-lite' \
	  'make install-truffle    Build and install the JVM/Truffle launcher' \
	  'make install-truffle-native' \
	  '                        Build and install the GraalVM native image' \
	  'make install-all        Install the Rust CLI and JVM/Truffle launcher' \
	  'make uninstall          Remove files installed by these targets' \
	  'make check-install      Exercise staged install and uninstall flows' \
	  'make test-rust-java-parity' \
	  '                        Run Rust-Java protocol, native, HALC, and HTA parity' \
	  '' \
	  'Variables: PREFIX, DESTDIR, BINDIR, DATADIR, HARA_DATADIR, HARA_LITE_DATADIR,' \
	  '           HARA_SPECS_REGISTRY'
