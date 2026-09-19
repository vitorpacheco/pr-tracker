BINARY  := pr-tracker
PKG     := github.com/vitorpacheco/pr-tracker
VERSION ?= $(shell git describe --tags --always --dirty 2>/dev/null || echo dev)
LDFLAGS := -s -w -X main.version=$(VERSION)
PREFIX  ?= $(HOME)/.local
DIST    := dist

PLATFORMS := linux/amd64 linux/arm64 darwin/amd64 darwin/arm64 windows/amd64 windows/arm64
GOVULNCHECK := golang.org/x/vuln/cmd/govulncheck@latest

.DEFAULT_GOAL := help

.PHONY: help
help: ## Lista os comandos disponíveis
	@awk 'BEGIN {FS = ":.*## "} /^[a-zA-Z_-]+:.*## / {printf "  \033[36m%-10s\033[0m %s\n", $$1, $$2}' $(MAKEFILE_LIST)

.PHONY: build
build: ## Compila o binário ./pr-tracker
	go build -trimpath -ldflags "$(LDFLAGS)" -o $(BINARY) .

.PHONY: run
run: ## Executa a interface (use ARGS="doctor" para subcomandos)
	go run . $(ARGS)

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
	gofmt -w .

.PHONY: lint
lint: ## Verifica formatação e roda go vet
	@test -z "$$(gofmt -l .)" || (echo "arquivos não formatados:"; gofmt -l .; exit 1)
	go vet ./...

.PHONY: check
check: lint test ## Lint + testes (use antes de commitar)

.PHONY: vuln
vuln: ## Procura vulnerabilidades conhecidas nas dependências (govulncheck)
	go run $(GOVULNCHECK) ./...

.PHONY: tidy
tidy: ## Atualiza go.mod/go.sum
	go mod tidy

.PHONY: install
install: build ## Instala em $(PREFIX)/bin (padrão ~/.local/bin)
	install -Dm755 $(BINARY) $(PREFIX)/bin/$(BINARY)

.PHONY: uninstall
uninstall: ## Remove de $(PREFIX)/bin
	rm -f $(PREFIX)/bin/$(BINARY)

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

.PHONY: doctor
doctor: build ## Verifica CLIs, autenticação e configuração
	./$(BINARY) doctor

.PHONY: clean
clean: ## Remove artefatos de build
	rm -rf $(BINARY) $(DIST) coverage.out
