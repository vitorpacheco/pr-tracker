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

## Roteiro para o MacBook

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
