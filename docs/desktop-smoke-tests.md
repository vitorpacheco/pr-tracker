# Smoke tests desktop

## Arch Linux / Hyprland — 26/09/2026

Build local baseado em `fbc443d`, com a alteração de política de GPU em
`desktop/main.go`. A correção ainda precisa ser incluída em uma release para
validar o artefato distribuído.

| Componente | Versão |
|---|---|
| Kernel | 7.2.5-3-omarchy |
| Hyprland | 0.56.2 |
| GTK 3 | 3.24.52 |
| WebKitGTK 4.1 | 2.52.6 |
| GPU | NVIDIA GeForce RTX 3070 Ti |
| Driver NVIDIA | 610.57.04 |
| Escala do monitor | 1,25 |

### Falha encontrada e correção

O executável original encerrou com status 1 em duas inicializações: uma com
`GDK_BACKEND=wayland` e outra com a seleção padrão de backend. O trace com
`WAYLAND_DEBUG=1` registrou:

```text
wl_display.error(wp_linux_drm_syncobj_surface_v1, 4, "Missing acquire timeline")
Gdk-Message: Error 71 (Protocol error) dispatching to Wayland display.
```

Executar o mesmo binário com `WEBKIT_DISABLE_DMABUF_RENDERER=1` permitiu abrir a
interface. Em seguida, trocar `WebviewGpuPolicyOnDemand` por
`WebviewGpuPolicyNever` permitiu abrir e operar a GUI nativa sem essa variável.
A política conservadora evita o caminho acelerado no Linux; não altera macOS
ou Windows. O efeito sobre desempenho em listas grandes ainda não foi medido.

Esse problema depende do compositor, driver e WebKitGTK reais. A verificação de
regressão é iniciar o binário sem variáveis de contorno, conferir a janela
renderizada, interagir e redimensionar. Um teste unitário da constante de GPU
não exercitaria a falha.

### Verificações realizadas

- Build nativo via `make desktop-build`.
- Duas inicializações após a correção: configuração vazia e cache preenchido.
- Janela registrada pelo Hyprland com App ID
  `io.github.vitorpacheco.pr_tracker` e `xwayland: false`.
- Lista/inspector em 1440 e 900 px; lista e rota de detalhe em 600 px.
- Seleção do segundo item com `j`, preservada ao reduzir a largura.
- `Enter` abriu o detalhe estreito; `Esc` voltou à lista.
- `/` filtrou os três itens por título, deixando um resultado.
- `Ctrl+K`, busca na palette e `Tab`/`Enter` abriram configurações.
- Tema claro/escuro, mudança de português para inglês, salvamento no TOML
  isolado e persistência após reiniciar.
- Diagnóstico de executáveis pela GUI; nenhuma instância real foi autenticada.
- Cache SQLite sintético com três PRs; `tool_paths.gh = "/usr/bin/false"`
  simulou falha de consulta. Os itens permaneceram visíveis com aviso de erro.
- `a` abriu a confirmação de aprovação; `Esc` cancelou sem executar a ação.
- Fechamento normal das duas janelas de teste.
- `make check`, `make desktop-check` (10 testes frontend),
  `go test -tags gui,desktop,production,webkit2_41 ./desktop` e
  `CHROMIUM_PATH=/usr/bin/chromium make desktop-e2e` (17 testes).

Configuração, cache, logs e capturas locais ficam em `dist/desktop-smoke/`,
fora do versionamento. Os testes não publicaram comentários nem alteraram PRs
ou clones reais. Conversas remotas, terminal, diff, seletores de pasta e ciclo de
worktrees reais ainda precisam de validação nativa. Este smoke não conclui a
matriz multiplataforma nem a rodada de release candidata em GNOME/KDE.

## macOS / Apple Silicon — 26/09/2026

Build local do commit `4c30b50` em macOS 26.6.2 (25G83), `darwin/arm64`,
Go 1.27.1 e Node 26.10.0, com Command Line Tools em
`/Library/Developer/CommandLineTools`. CLI e GUI produziram executáveis
Mach-O arm64. Não foi utilizado um pacote baixado de release.

### Verificações realizadas

- `make check` e `go test -race ./...`: todos os pacotes passaram.
- `make build` e `make desktop-install desktop-check desktop-build
  DESKTOP_TAGS=gui,desktop,production`: builds concluídos, Svelte sem erros ou
  avisos e 10 testes frontend passando.
- `go test -tags gui,desktop,production ./desktop`: passou. O compilador emitiu
  um aviso de depreciação de `setShowsBaselineSeparator:` no código do Wails;
  não impediu build nem execução.
- Executável GUI iniciou sem saída de erro. Para interação pela ferramenta de
  acessibilidade, foi criado um `.app` local de teste com o mesmo binário,
  `Info.plist` mínimo e `LSEnvironment` apontando para configuração/cache isolados.
  Esse wrapper não é um pacote de distribuição validado.
- GUI renderizada no WebKit nativo, incluindo lista, inspector, branches,
  totais do diff, status de CI e mensagem de indisponibilidade do provider.
- `Cmd+K` abriu comandos; configurações abriram pela palette; diagnóstico
  encontrou `git` em `/opt/homebrew/bin` e `gh`, `glab` e `hunk` no mise.
  `tea` ausente. O diagnóstico CLI também encontrou herdr e tmux.
- Seletor nativo de arquivo TOML abriu e foi cancelado normalmente.
- Idioma alterado de português para inglês, salvo no TOML isolado e preservado
  após fechar com `Cmd+Q` e reabrir.
- Cache SQLite sintético com três PRs restaurado na inicialização;
  `tool_paths.gh = "/usr/bin/false"` simulou falha de consulta, preservando
  os itens e exibindo aviso de dados anteriores.
- `j` selecionou o segundo item; `a` abriu confirmação para esse PR e `Esc`
  cancelou. `/` e busca por `Cache offline` reduziram a lista a um item.
- TUI executada em PTY com o mesmo cache: renderização, navegação com `j` e
  saída com `q` concluídas com status 0.
- GUI fechada normalmente após a rodada.

### Falha encontrada no teste de navegador

`CHROMIUM_PATH='/Applications/Google Chrome.app/Contents/MacOS/Google Chrome'
make desktop-e2e` passou em **18 de 19 testes**. Os testes de layouts em
600/900/1440 px, temas, idiomas, divisórias e persistência passaram no Chrome.
Isso não equivale a testar esses tamanhos no WebKit nativo.

Falhou `keeps selection on resize and reports unavailable bridge`, em
`desktop/frontend/e2e/desktop.spec.ts:89`: `getByRole('alert')` encontrou dois
elementos com a mensagem `Abra o aplicativo desktop`. A tela sem bridge pode
renderizar simultaneamente `themeError` e `error`, ambos preenchidos pela falha
de acesso ao backend. O teste exige um único alerta.

A correção posterior mantém o alerta de tema e só exibe o alerta geral quando
sua mensagem é diferente. O teste agora verifica explicitamente a existência
de um único alerta. Um novo cenário garante que falhas distintas de tema e
dados permaneçam visíveis e que a recuperação do tema preserve o erro de dados.
Após a correção, os **20 testes E2E passaram**, assim como `make check`,
`make desktop-check` e o build nativo com `DESKTOP_TAGS=gui,desktop,production`.

Configuração, cache, wrapper `.app`, log nativo e captura `native-cache.png`
ficam em `dist/macos-smoke/`, fora do versionamento. O contexto da falha E2E
fica em `dist/desktop-test-results/`, e suas capturas em `dist/desktop-preview/`.

### Limites desta rodada

Não foram consultadas instâncias reais, publicadas mensagens ou alterados PRs
e clones reais. Autenticação remota, conversas reais, terminal/diff externos,
worktrees pela GUI, seletor de clone, exportação de cores, mudança de tema e
redimensionamento nativo permanecem para outra rodada. Não foram validados
Intel, pacote de release, Gatekeeper/quarentena, `.app` universal ou `.dmg`.

## Roteiro complementar para o MacBook

Registrar macOS, arquitetura, versão/commit do pacote e resultados de cada item.
Usar o pacote `darwin_arm64` em Apple Silicon ou `darwin_amd64` em Intel enquanto
o `.app` universal e o `.dmg` não estiverem disponíveis. A distribuição do projeto
será sem assinatura Developer ID e sem notarização.

1. Abrir o executável com configuração e cache de teste. Quando houver `.app`,
   repetir pelo Finder para conferir o ambiente real de launcher.
2. Executar o diagnóstico e verificar `git` e os providers utilizados. Conferir
   resolução em `/opt/homebrew/bin` ou `/usr/local/bin` e caminhos explícitos.
3. Configurar uma instância autenticada; carregar dados, fechar e abrir novamente
   para conferir cache antes do refresh. Simular indisponibilidade e confirmar
   preservação dos dados.
4. Conferir lista, detalhe, checks e conversa em 600, 900 e 1440 px. Testar
   `j/k`, setas, `/`, `Enter`, `Esc`, `Cmd+K`, temas e idioma.
5. Abrir um link no navegador e selecionar um clone pelo seletor nativo. Em um
   repositório descartável, criar worktree, abrir terminal/diff e remover o
   worktree. Conferir a proteção com arquivo local modificado.
6. Conferir confirmações de comentário, aprovação, merge e fechamento, cancelando
   antes do envio. Executar ações remotas somente em PRs de teste destinados a isso.
7. Fechar normalmente, reabrir e verificar preferências e ausência de erros.

Registrar separadamente bloqueios do Gatekeeper ao abrir o pacote sem assinatura
e falhas da aplicação. Não desativar o Gatekeeper globalmente para os testes.

## Integração Omarchy — 26/09/2026

A GUI Wails e a TUI no Foot foram abertas no Hyprland com configuração/cache
isolados e a paleta real do Omarchy. Fundo, texto, destaque laranja e cores de
status seguiram o `colors.toml` ativo. Em uma segunda rodada, ambas leram uma
cópia da paleta em `XDG_STATE_HOME` temporário; substituir essa cópia pela paleta
clara Rose Pine atualizou as duas janelas sem reiniciar. O tema do desktop não
foi alterado. A TUI também continuou respeitando `NO_COLOR`.

Testes Go cobrem leitura atual/legada, prioridade dos caminhos, paleta inválida,
recarga e preservação de estado da TUI sem compartilhar estilos entre modelos.
O teste Playwright cobre a paleta na opção Sistema, precedência de Claro manual,
retorno a Sistema e remoção das cores quando a paleta deixa de estar disponível.
Capturas com bridge simulado:

- [GUI com paleta escura](screenshots/gui-omarchy-dark.png)
- [GUI com paleta clara](screenshots/gui-omarchy-light.png)

Capturas nativas locais: `dist/desktop-smoke/omarchy-gui-settled.png`,
`omarchy-tui-color.png`, `omarchy-gui-live-light.png` e
`omarchy-tui-live-light.png` no mesmo diretório.
