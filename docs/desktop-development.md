# Desenvolvimento da GUI

A aplicação desktop usa Wails 2.16, Svelte 5 e TypeScript. A CLI/TUI continua no
executável `pr-tracker`; a GUI é um executável separado, `pr-tracker-desktop`.
Ambas usam a configuração TOML, os providers e o cache SQLite existentes.

## Executar

Requisitos: Go conforme `go.mod`, Node 24 ou superior, npm e as dependências
nativas do Wails. No Linux, o alvo padrão usa GTK 3 e WebKitGTK 4.1. O ambiente
utilizado para o primeiro build possui GTK 3.24.52 e WebKitGTK 2.52.6.

```sh
make desktop-install
make desktop-check
make desktop-build
./dist/pr-tracker-desktop
```

Abrir o executável utiliza as configurações reais do usuário. A GUI não descobre
nem grava novas instâncias automaticamente: a tela de configurações permite
adicioná-las explicitamente. Autenticação continua sendo feita por `gh`, `glab`
e `tea`; a GUI não solicita tokens.

Para experimentar somente o frontend, com dados fictícios e sem chamadas ao
backend:

```sh
make desktop-dev
# Abrir http://127.0.0.1:5173/?demo=1
```

O modo de demonstração é indicado na interface e rejeita alterações. Abrir o
preview sem `?demo=1` informa que o bridge nativo está indisponível; dados reais
não são substituídos silenciosamente por exemplos.

Para executar um smoke test sem usar arquivos pessoais, crie uma pasta local e
aponte os caminhos antes de abrir o binário:

```sh
mkdir -p .cache/desktop-smoke/config .cache/desktop-smoke/data .cache/desktop-smoke/cache
PR_TRACKER_CONFIG="$PWD/.cache/desktop-smoke/config.toml" \
PR_TRACKER_CACHE="$PWD/.cache/desktop-smoke/cache.db" \
XDG_CONFIG_HOME="$PWD/.cache/desktop-smoke/config" \
XDG_DATA_HOME="$PWD/.cache/desktop-smoke/data" \
XDG_CACHE_HOME="$PWD/.cache/desktop-smoke/cache" \
./dist/pr-tracker-desktop
```

## Interface disponível

- Cache antes do refresh remoto, erros por instância e preservação dos dados
  anteriores quando o provider está indisponível.
- PRs e issues, busca por título/repositório/autor/referência, filtros por relação
  e instância, contagens e seleção preservada durante refresh e resize.
- Lista dedicada abaixo de 700 px, lista e inspector entre 700 e 1099 px e
  navegação lateral adicional a partir de 1100 px.
- Menu lateral recolhível pelo botão no canto superior esquerdo. Em janelas
  menores, o mesmo botão abre o menu sobre o conteúdo; `Esc` ou um clique fora
  dele fecha o menu.
- Divisórias arrastáveis entre menu, lista e inspector. A lista ocupa o espaço
  restante e adapta suas colunas à largura disponível. Com foco na divisória,
  `←`/`→` ajustam 10 px, `Shift` acelera para 40 px e `Home`/`End` levam aos
  limites. Larguras e estado recolhido são salvos localmente; reduzir a janela
  limita as larguras temporariamente, sem apagar a preferência para telas maiores.
- `j/k`, setas, `Enter`, `Esc`, `/`, `Cmd/Ctrl+K`, `r`, `v`, `n`, `a`, `m`, `w`,
  `t`, `d` e `o`. Inputs não executam atalhos de ações durante digitação.
- Descrição e conversa em Markdown sanitizado, checks, aprovações, conflitos,
  branches e totais do diff. Sem inventar aprovações obrigatórias nem arquivos
  individuais ausentes no provider.
- Rascunhos locais por item, revisão antes do envio, aprovação, merge,
  fechamento, checkout e ciclo de vida dos worktrees.
- Confirmação separada para remoção de worktree sujo, inclusive após o PR sair
  da lista por merge/fechamento. Falha remota não inicia limpeza local.
- Terminal e diff externos, navegador, pasta local e seletor nativo de clone.
- Configurações de instâncias, mapeamentos e `track_all`, diagnóstico de CLIs e
  autenticação, tema do sistema ou seleção manual claro/escuro.

## Ferramentas e terminais

Campos novos opcionais no TOML:

```toml
desktop_terminal = "kitty"

[tool_paths]
git = "/usr/bin/git"
gh = "/opt/homebrew/bin/gh"
```

`desktop_terminal` não altera o campo `terminal` usado pela TUI. Vazio detecta
kitty, foot, wezterm, GNOME Terminal, Konsole ou xterm no Linux; Terminal no macOS;
Windows Terminal, pwsh ou PowerShell no Windows. Um executável personalizado pode
ser informado; no Linux, wrappers devem aceitar `-e comando argumentos…`.

Caminhos explícitos de CLIs prevalecem sobre o `PATH`. No macOS também são
consultados `/opt/homebrew/bin` e `/usr/local/bin`. Diretórios configurados são
passados ao ambiente dos processos filhos sem modificar o `PATH` global.

## Contrato e testes

`internal/app.Session` mantém estado, gerações de configuração, refresh em curso,
ações por item, cancelamento e resultados parciais. `Application` implementa as
operações de uma configuração imutável. A TUI e `desktop/Bridge` usam a sessão;
o frontend envia chaves de itens, sem escolher repositórios arbitrários.

Os modelos e bindings em `desktop/frontend/wailsjs/` são gerados e versionados.
Após mudar modelos ou métodos exportados do bridge:

```sh
make desktop-bindings
make desktop-check
```

A geração exige que `desktop/frontend/dist` exista; `make desktop-build` cria os
assets. Os tipos usam interfaces TypeScript; datas seguem a serialização JSON
RFC 3339 do Go. Campos de provider ficam em `Item.Data`, normalizados apenas no
adapter frontend. A geração Wails atualmente avisa sobre `time.Time`, que aparece
como `any` no TypeScript; a data serializada é coberta pelo contrato Go.

```sh
make check
go test -race ./...
make desktop-check
# Usar Chromium já instalado, ou instalar o navegador do Playwright separadamente.
CHROMIUM_PATH=/usr/bin/chromium make desktop-e2e
```

Os testes de navegador geram capturas em `dist/desktop-preview/` nos três
breakpoints e nos dois temas. Testam navegação, busca, rascunho, resize, menu
recolhível, divisórias por mouse/teclado, persistência do layout, erros parciais,
o formato real do bridge e sanitização de Markdown. Os testes Go de
ações usam adapters falsos; não aprovam PRs nem publicam comentários reais.

Quando caches devem ficar no checkout, use `GOMODCACHE="$PWD/.cache/mod"`,
`GOCACHE="$PWD/.cache/go-build"` e `npm --cache ../../.cache/npm` dentro do
frontend. Para Chromium, um `TMPDIR` muito longo excede o limite de sockets Unix;
`mkdir -p .t` e `TMPDIR="$PWD/.t"` no diretório raiz resolvem isso neste checkout.

## Builds e limites de validação

O workflow reutilizável `desktop.yml` define validação frontend e builds nativos
separados em Linux, macOS e Windows, tanto em `amd64` quanto em `arm64`. O CI o
chama para validar cada mudança; nightly e releases publicam seus seis pacotes e
incluem todos eles em `checksums.txt`. Em macOS/Windows, o build Go usa
`-tags gui,desktop,production`; no Linux, acrescenta `webkit2_41`. Os jobs de CLI
e seu loop com `CGO_ENABLED=0` permanecem separados.

O pacote desktop vincula `UniformTypeIdentifiers` via cgo no macOS. Isso permite
usar `go build` diretamente, sem depender das flags extras do comando de build
do Wails para os seletores nativos de arquivos.

O smoke nativo em Arch Linux/Hyprland foi repetido em 26/09/2026 com configuração
isolada, cache sintético, navegação por teclado e os três breakpoints. Ele revelou
uma falha no caminho acelerado do WebKitGTK/NVIDIA; a GUI agora desativa a
aceleração do WebView no Linux. O [registro do smoke](desktop-smoke-tests.md)
descreve a reprodução, a correção, os resultados e o roteiro para o MacBook.
As capturas automatizadas usam Chromium; elas não substituem testes de WebKitGTK.
A execução em macOS/Windows e as rodadas em GNOME/KDE ainda precisam de seus
respectivos ambientes. Terminal, diff e ações remotas não foram executados contra
contas ou clones reais durante estes testes.

O arquivo `.desktop` e o ícone SVG ficam em `desktop/build/linux/`; a aplicação
usa o mesmo App ID. Nenhum deles é instalado automaticamente. Instaladores,
persistência do tamanho da janela e notificações permanecem para os próximos
incrementos.

No macOS, os pacotes serão distribuídos sem assinatura Developer ID e sem
notarização. O projeto não exige conta paga Apple Developer nem credenciais
Apple no CI. O empacotamento em `.app` universal e `.dmg` e os testes nativos
continuam pendentes; assinatura e notarização não são critérios de conclusão. A GUI não
persiste nem impõe posição da janela no Wayland.

## Capturas verificadas

Dados fictícios do modo de demonstração:

- [1440 px, escuro](screenshots/gui-1440-dark.png)
- [900 px, escuro](screenshots/gui-900-dark.png)
- [600 px, escuro](screenshots/gui-600-dark.png)
- [1440 px, claro](screenshots/gui-1440-light.png)
- [Menu recolhido](screenshots/gui-sidebar-collapsed.png)
- [Painéis redimensionados](screenshots/gui-panels-resized.png)
