BINARY  := pr-tracker
PKG     := github.com/vitorpacheco/pr-tracker
VERSION ?= $(shell git describe --tags --always --dirty 2>/dev/null || echo dev)
LDFLAGS := -s -w -X main.version=$(VERSION)
PREFIX  ?= $(HOME)/.local
DIST    := dist

PLATFORMS := linux/amd64 linux/arm64 darwin/amd64 darwin/arm64 windows/amd64

.DEFAULT_GOAL := help

.PHONY: help
help: ## Lista os comandos disponíveis
	@awk 'BEGIN {FS = ":.*## "} /^[a-zA-Z_-]+:.*## / {printf "  \033[36m%-10s\033[0m %s\n", $$1, $$2}' $(MAKEFILE_LIST)

.PHONY: build
build: ## Compila o binário ./pr-tracker
	go build -ldflags "$(LDFLAGS)" -o $(BINARY) .

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
dist: ## Compila para Linux, macOS e Windows em ./dist
	@mkdir -p $(DIST)
	@for p in $(PLATFORMS); do \
		os=$${p%/*}; arch=$${p#*/}; ext=; [ $$os = windows ] && ext=.exe; \
		out=$(DIST)/$(BINARY)-$$os-$$arch$$ext; \
		echo "→ $$out"; \
		GOOS=$$os GOARCH=$$arch CGO_ENABLED=0 go build -ldflags "$(LDFLAGS)" -o $$out . || exit 1; \
	done

.PHONY: doctor
doctor: build ## Verifica CLIs, autenticação e configuração
	./$(BINARY) doctor

.PHONY: clean
clean: ## Remove artefatos de build
	rm -rf $(BINARY) $(DIST) coverage.out
