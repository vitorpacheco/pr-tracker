BINARY  := pr-tracker
VERSION ?= $(shell git describe --tags --always --dirty 2>/dev/null || echo dev)
PREFIX  ?= $(HOME)/.local
DIST    := dist
CARGO_AUDIT_VERSION := 0.22.2

# The packaged platform: the host by default, as the window links native
# graphics libraries and is built on each platform's own runner.
OS   ?= $(shell uname -s | tr '[:upper:]' '[:lower:]' | sed -e 's/mingw.*/windows/' -e 's/msys.*/windows/')
ARCH ?= $(shell uname -m | sed -e 's/x86_64/amd64/' -e 's/aarch64/arm64/')
EXT  := $(if $(filter windows,$(OS)),.exe,)

export PR_TRACKER_VERSION := $(VERSION)

.DEFAULT_GOAL := help

##@ Ajuda

.PHONY: help
help: ## Lista os comandos disponíveis por categoria
	@awk 'BEGIN {FS = ":.*## "} /^##@ / {printf "\n\033[1m%s\033[0m\n", substr($$0, 5)} /^[a-zA-Z0-9_-]+:.*## / {printf "  \033[36m%-18s\033[0m %s\n", $$1, $$2}' $(MAKEFILE_LIST)

##@ Execução e desenvolvimento

.PHONY: run
run: ## Abre a janela (use ARGS="doctor" para subcomandos, ARGS="--demo" para dados fictícios)
	cargo run -p pr-tracker -- $(ARGS)

.PHONY: doctor
doctor: ## Verifica CLIs, autenticação e configuração
	cargo run -p pr-tracker -- doctor

.PHONY: snapshot
snapshot: ## Renderiza a janela com dados fictícios em dist/preview (sem abrir janela)
	@mkdir -p $(DIST)/preview
	cargo run -p pr-tracker -- --demo --snap $(DIST)/preview/window.png $(ARGS)

##@ Build e distribuição

.PHONY: build
build: ## Compila o binário ./pr-tracker
	cargo build --release -p pr-tracker
	cp target/release/$(BINARY)$(EXT) $(BINARY)$(EXT)

.PHONY: package
package: ## Empacota o binário desta plataforma (.tar.gz/.zip) em ./dist
	cargo build --release -p pr-tracker
	@set -e; name=$(BINARY)_$(VERSION)_$(OS)_$(ARCH); \
	rm -rf $(DIST)/$$name && mkdir -p $(DIST)/$$name; \
	cp target/release/$(BINARY)$(EXT) README.md $(DIST)/$$name/; \
	if [ $(OS) = linux ]; then cp packaging/linux/* $(DIST)/$$name/; fi; \
	if [ $(OS) = windows ]; then (cd $(DIST) && 7z a -bso0 $$name.zip $$name); else tar -C $(DIST) -czf $(DIST)/$$name.tar.gz $$name; fi; \
	rm -rf $(DIST)/$$name; \
	echo "→ $(DIST)/$$name"

.PHONY: checksums
checksums: ## Gera dist/checksums.txt para os pacotes em ./dist
	@cd $(DIST) && if command -v sha256sum >/dev/null; then sha256sum *.tar.gz *.zip; else shasum -a 256 *.tar.gz *.zip; fi > checksums.txt

##@ Testes e qualidade

.PHONY: test
test: ## Roda os testes
	cargo test --workspace

.PHONY: fmt
fmt: ## Formata o código
	cargo fmt --all

.PHONY: lint
lint: ## Verifica formatação e roda clippy
	cargo fmt --all -- --check
	cargo clippy --workspace --all-targets -- -D warnings

.PHONY: check
check: lint test ## Lint + testes (use antes de commitar)

.PHONY: vuln
vuln: ## Procura vulnerabilidades conhecidas nas dependências (cargo-audit)
	cargo install cargo-audit --locked --version $(CARGO_AUDIT_VERSION)
	cargo audit

##@ Instalação e limpeza

.PHONY: install
install: build ## Instala em $(PREFIX)/bin (padrão ~/.local/bin)
	install -Dm755 $(BINARY)$(EXT) $(PREFIX)/bin/$(BINARY)$(EXT)

.PHONY: uninstall
uninstall: ## Remove de $(PREFIX)/bin
	rm -f $(PREFIX)/bin/$(BINARY)$(EXT)

.PHONY: clean
clean: ## Remove artefatos de build
	cargo clean
	rm -rf $(BINARY) $(BINARY).exe $(DIST)
