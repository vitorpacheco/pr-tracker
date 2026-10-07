GO_SOURCES := $(shell git ls-files --cached --others --exclude-standard '*.go')
BINARY  := pr-tracker
PKG     := github.com/vitorpacheco/pr-tracker
VERSION ?= $(shell git describe --tags --always --dirty 2>/dev/null || echo dev)
LDFLAGS := -s -w -X main.version=$(VERSION)
PREFIX  ?= $(HOME)/.local
DIST    := dist

PLATFORMS := linux/amd64 linux/arm64 darwin/amd64 darwin/arm64 windows/amd64 windows/arm64
GOVULNCHECK := golang.org/x/vuln/cmd/govulncheck@latest

# Native desktop builds are separate from the CGO-disabled CLI release loop.
WAILS_VERSION := v2.16.0
DESKTOP_TAGS ?= gui,desktop,production,webkit2_41

.DEFAULT_GOAL := help

##@ Ajuda

.PHONY: help
help: ## Lista os comandos disponíveis por categoria
	@awk 'BEGIN {FS = ":.*## "} /^##@ / {printf "\n\033[1m%s\033[0m\n", substr($$0, 5)} /^[a-zA-Z0-9_-]+:.*## / {printf "  \033[36m%-18s\033[0m %s\n", $$1, $$2}' $(MAKEFILE_LIST)

##@ Execução e desenvolvimento

.PHONY: run
run: ## Executa a interface (use ARGS="doctor" para subcomandos)
	go run . $(ARGS)

.PHONY: doctor
doctor: build ## Verifica CLIs, autenticação e configuração
	./$(BINARY) doctor

.PHONY: desktop-run
desktop-run: desktop-build ## Compila e executa a GUI nativa
	./$(DIST)/pr-tracker-desktop

.PHONY: desktop-dev
desktop-dev: ## Abre preview da GUI no navegador (use ?demo=1 para dados fictícios)
	cd desktop/frontend && npm run dev

##@ Build e distribuição

.PHONY: build
build: ## Compila o binário ./pr-tracker
	go build -trimpath -ldflags "$(LDFLAGS)" -o $(BINARY) .

.PHONY: desktop-build
desktop-build: ## Compila a GUI nativa em dist/pr-tracker-desktop (Linux com WebKitGTK 4.1)
	cd desktop/frontend && npm run build
	go build -trimpath -tags $(DESKTOP_TAGS) -o $(DIST)/pr-tracker-desktop ./desktop

.PHONY: dist
dist: ## Compila para Linux, macOS e Windows em ./dist (binários)
	@mkdir -p $(DIST)
	@for p in $(PLATFORMS); do \
		os=$${p%/*}; arch=$${p#*/}; ext=; [ $$os = windows ] && ext=.exe; \
		out=$(DIST)/$(BINARY)-$$os-$$arch$$ext; \
		echo "→ $$out"; \
		GOOS=$$os GOARCH=$$arch CGO_ENABLED=0 go build -trimpath -ldflags "$(LDFLAGS)" -o $$out . || exit 1; \
	done

.PHONY: package
package: dist ## Empacota os binários (.tar.gz/.zip) e gera checksums.txt para release
	@set -e; cd $(DIST); \
	for p in $(PLATFORMS); do \
		os=$${p%/*}; arch=$${p#*/}; ext=; [ $$os = windows ] && ext=.exe; \
		name=$(BINARY)_$(VERSION)_$${os}_$${arch}; \
		rm -rf $$name && mkdir $$name; \
		cp $(BINARY)-$$os-$$arch$$ext $$name/$(BINARY)$$ext; \
		cp ../README.md $$name/; \
		if [ $$os = windows ]; then zip -qr $$name.zip $$name; else tar -czf $$name.tar.gz $$name; fi; \
		rm -rf $$name; \
		echo "→ $(DIST)/$$name"; \
	done; \
	if command -v sha256sum >/dev/null; then sha256sum *.tar.gz *.zip; else shasum -a 256 *.tar.gz *.zip; fi > checksums.txt

##@ Testes e qualidade

.PHONY: test
test: ## Roda os testes
	go test ./...

.PHONY: cover
cover: ## Roda os testes com relatório de cobertura
	go test -coverprofile=coverage.out ./...
	go tool cover -func=coverage.out | tail -1

.PHONY: vet
vet: ## Roda go vet
	go vet ./...

.PHONY: fmt
fmt: ## Formata o código
	gofmt -w $(GO_SOURCES)

.PHONY: lint
lint: ## Verifica formatação e roda go vet
	@test -z "$$(gofmt -l $(GO_SOURCES))" || (echo "arquivos não formatados:"; gofmt -l $(GO_SOURCES); exit 1)
	go vet ./...

.PHONY: check
check: lint test ## Lint + testes (use antes de commitar)

.PHONY: vuln
vuln: ## Procura vulnerabilidades conhecidas nas dependências (govulncheck)
	go run $(GOVULNCHECK) ./...

.PHONY: desktop-check
desktop-check: ## Verifica TypeScript/Svelte e testes do frontend
	cd desktop/frontend && npm run check && npm test

.PHONY: desktop-e2e
desktop-e2e: ## Testa os layouts desktop em Chromium (CHROMIUM_PATH opcional)
	cd desktop/frontend && npm run test:e2e

##@ Dependências e bindings

.PHONY: tidy
tidy: ## Atualiza go.mod/go.sum
	go mod tidy

.PHONY: desktop-install
desktop-install: ## Instala dependências do frontend desktop
	cd desktop/frontend && npm ci

.PHONY: desktop-bindings
desktop-bindings: ## Regenera contratos Go/TypeScript (requer assets do frontend)
	cd desktop && go run github.com/wailsapp/wails/v2/cmd/wails@$(WAILS_VERSION) generate module -tags gui,webkit2_41

##@ Instalação e limpeza

.PHONY: install
install: build ## Instala em $(PREFIX)/bin (padrão ~/.local/bin)
	install -Dm755 $(BINARY) $(PREFIX)/bin/$(BINARY)

.PHONY: uninstall
uninstall: ## Remove de $(PREFIX)/bin
	rm -f $(PREFIX)/bin/$(BINARY)

.PHONY: clean
clean: ## Remove artefatos de build
	rm -rf $(BINARY) $(DIST) coverage.out
