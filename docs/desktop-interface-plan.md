# Plano da interface desktop

Issue de acompanhamento: [#5 — Adicionar interface desktop multiplataforma com
Wails](https://github.com/vitorpacheco/pr-tracker/issues/5)

## Decisão aprovada

Usar **Wails v2** com **Svelte e TypeScript**, mantendo o núcleo da aplicação em
Go e preservando a interface de terminal existente.

A direção visual aprovada combina:

- a responsividade do protótipo 5, **Adaptive Tile**;
- a navegação por teclado do protótipo 4, **Terminal Hybrid**;
- a densidade da tabela e o inspector do protótipo 1, **Command Center**, quando
  houver largura suficiente.

## Protótipos

O [protótipo final](prototypes/final-adaptive-command-center.png) consolida a
direção escolhida em estados largo e estreito da mesma aplicação.

Explorações mantidas como referência:

1. [Command Center](prototypes/01-command-center.png): máxima densidade e
   inspector persistente; funciona melhor em janelas largas.
2. [Review Inbox](prototypes/02-review-inbox.png): excelente para triagem e leitura
   de conversas; três painéis exigem bastante largura.
4. [Terminal Hybrid](prototypes/04-terminal-hybrid.png): menor distância da TUI e
   melhor exposição dos atalhos; pode parecer denso para usuários novos.
5. [Adaptive Tile](prototypes/05-adaptive-tile.png): melhor base responsiva para
   Hyprland e ainda natural nos desktops tradicionais.

## Objetivos

- Funcionar como aplicação desktop no macOS, Hyprland/Wayland, GNOME, KDE e
  Windows.
- Tratar janelas em mosaico estreitas como um caso normal, não como uma versão
  degradada da interface.
- Compartilhar configuração, cache, providers, operações Git e regras de negócio
  entre a TUI e a GUI.
- Preservar os atalhos principais da TUI e adicionar command palette.
- Não depender de transparência, posicionamento absoluto ou decoração de janela
  específica de um compositor.
- Manter `gh`, `glab`, `tea` e `git` como ferramentas externas na primeira versão.

## Arquitetura-alvo

O seam principal será um módulo profundo `internal/app`. A TUI e o Wails serão
adapters desse módulo, sem conhecer detalhes de concorrência, cache, providers ou
worktrees.

```text
                     internal/app
              pequena interface compartilhada
       ┌──────────────────┴──────────────────┐
       │                                     │
internal/ui (Bubble Tea)              desktop/bridge (Wails)
       │                                     │
       └────────── interfaces de usuário ────┘

internal/app implementation
  ├── config
  ├── cache
  ├── provider
  ├── gitops
  ├── launch
  └── toolchain
```

Interface inicial sugerida:

```go
type Application struct { /* dependencies and state */ }

func (a *Application) Load(ctx context.Context) (Snapshot, error)
func (a *Application) Refresh(ctx context.Context) RefreshResult
func (a *Application) Detail(ctx context.Context, key ItemKey) (Detail, error)
func (a *Application) Execute(ctx context.Context, command Command) (Outcome, error)
```

`Execute` recebe comandos tipados para comentário, aprovação, merge, fechamento,
checkout, worktree e configuração. Isso evita publicar um método Wails para cada
detalhe da implementação. A interface é também a superfície principal de testes.

A seleção, a rota atual, a largura dos painéis e a command palette continuam
sendo estado de apresentação de cada adapter. Atualização concorrente, fallback
para cache, resolução de clones e regras de ação ficam no módulo compartilhado.

## Estrutura de diretórios proposta

```text
internal/
  app/                    # orquestração compartilhada e tipos de entrada/saída
  cache/
  config/
  gitops/
  launch/
  provider/
  toolchain/              # localização e diagnóstico de executáveis externos
  ui/                     # TUI existente, agora adapter de internal/app
desktop/
  main.go                 # inicialização Wails
  bridge.go               # adapter estreito Go <-> TypeScript
  wails.json
  frontend/
    src/
      lib/components/
      lib/stores/
      lib/commands/
      routes/
```

O projeto desktop fica dentro do mesmo módulo Go. Assim ele pode importar
`internal/*` sem publicar pacotes ou manter um segundo backend.

## Comportamento responsivo

Os breakpoints devem responder à largura da janela, não ao monitor:

| Largura útil | Apresentação |
|---|---|
| até 699 px | uma rota por vez: lista ou detalhe, com `Voltar` |
| 700–1099 px | lista + detalhe; navegação global recolhida |
| 1100 px ou mais | navegação + lista/tabela + inspector |

Regras:

- nenhuma coluna crítica terá largura fixa obrigatória;
- colunas secundárias da tabela desaparecem por prioridade;
- ações destrutivas ficam em `Mais ações` nas janelas estreitas;
- o item selecionado e a posição de rolagem sobrevivem à mudança de largura;
- `Esc` volta/fecha, `j/k` e setas navegam, `/` busca, `Cmd/Ctrl+K` abre comandos;
- `Cmd` é mostrado no macOS e `Ctrl` nos outros sistemas;
- tema segue o sistema, com opção manual claro/escuro.

## Fases de implementação

### 0. Registrar a direção de produto

- Definir Svelte + TypeScript como frontend.
- Registrar uma ADR curta com a direção visual aprovada, a escolha do Wails v2 e
  os critérios para migrar à v3 depois que ela estiver estável.
- Fixar os breakpoints e os atalhos que constituem o contrato inicial da GUI.

**Saída:** direção visual aprovada e backlog fatiado.

### 1. Extrair o módulo compartilhado

- Criar `internal/app` e mover para ele a carga do cache, refresh concorrente,
  composição dos resultados, detalhe de item e execução das ações.
- Manter filtros, cursor, modais e renderização dentro de `internal/ui`.
- Fazer a TUI consumir o novo módulo sem mudança observável de comportamento.
- Criar testes do módulo usando adapters falsos de provider e filesystem apenas
  nos seams internos necessários.

**Critério de conclusão:** `make check` passa e a TUI mantém todas as operações
atuais.

### 2. Criar a casca Wails

- Adicionar o projeto em `desktop/` e incorporar os assets do frontend.
- Expor um bridge pequeno que converta os tipos de `internal/app` para modelos
  gerados do TypeScript.
- Implementar ciclo de vida, cancelamento de contexts e prevenção de refreshes
  concorrentes duplicados.
- Adicionar uma tela de diagnóstico/onboarding equivalente a `doctor`.

**Critério de conclusão:** a GUI abre nos três sistemas, carrega primeiro o cache
e atualiza em segundo plano.

### 3. Implementar navegação e lista adaptativa

- Abas de PRs e issues, contagens, busca e filtros.
- Lista compacta no modo estreito e tabela densa no modo largo.
- Seleção persistente, navegação completa por teclado e command palette.
- Estados de carregamento, vazio, offline e erro parcial por instância.
- Virtualização da lista caso a medição mostre necessidade.

**Critério de conclusão:** todas as listas atuais da TUI podem ser operadas em
tiles de 600 px, 900 px e 1440 px de largura.

### 4. Implementar detalhe e conversa

- Resumo do PR/issue, branches, diff stats, labels e responsáveis.
- Checks/pipeline, aprovações e conflitos.
- Descrição e conversa em Markdown, incluindo reviews e comentários inline.
- Composer de comentário com rascunho local e confirmação antes do envio.
- Inspector lateral no modo largo e rota dedicada no modo estreito.

**Critério de conclusão:** visualizar e comentar em PRs e issues sem recorrer à
TUI.

### 5. Implementar ações e worktrees

- Aprovar, merge, fechar, checkout, criar/atualizar/remover worktree.
- Confirmações equivalentes às atuais, inclusive proteção de worktree sujo.
- Abrir navegador, pasta, terminal e diff.
- Manter a primeira versão do diff externo; avaliar diff integrado depois do MVP.
- Mostrar progresso por item sem bloquear o restante da interface.

**Critério de conclusão:** paridade funcional com todas as ações atuais da TUI.

### 6. Configuração e integração com o sistema

- CRUD de instâncias e repositórios, teste de autenticação e `track_all`.
- Resolver executáveis em um novo módulo `toolchain`, sem depender apenas do
  `PATH` herdado pela aplicação.
- No macOS, procurar também caminhos comuns como `/opt/homebrew/bin` e
  `/usr/local/bin`; permitir caminhos explícitos na configuração.
- No Linux, respeitar o `PATH` da sessão e permitir configurar `kitty`, `foot`,
  `wezterm` ou outro terminal. No Windows, detectar Windows Terminal e PowerShell.
- Usar seletor nativo de pasta para mapear clones.

**Critério de conclusão:** abrir a aplicação pelo Finder, launcher do Hyprland,
menu do GNOME/KDE ou menu Iniciar encontra as ferramentas configuradas e explica
claramente as ausentes.

### 7. Qualidade de desktop

- Menu de aplicação, atalhos, copiar links, notificações opcionais e atualização
  manual/automática.
- Acessibilidade: foco visível, navegação sem mouse, nomes acessíveis e contraste.
- Persistir tamanho da janela, painéis e preferências; não persistir posição no
  Wayland.
- Arquivo `.desktop`, App ID estável e ícones instalados corretamente no Linux.
- Testes em Hyprland, GNOME/Wayland, KDE/Wayland, macOS e Windows.

### 8. Empacotamento e releases

- Manter o release atual da CLI/TUI.
- Criar builds desktop nativos em runners separados; Wails/CGO não deve usar o
  loop atual de cross-compilation com `CGO_ENABLED=0`.
- macOS: `.app` universal e `.dmg`, assinatura e notarização.
- Windows: instalador x64/arm64, assinatura quando houver certificado.
- Linux: começar por tarball e pacotes `.deb`; adicionar RPM/AUR e AppImage após
  validação em Wayland.
- Publicar checksums e distinguir claramente artefatos CLI e desktop.

## Estratégia de testes

- **Go:** testes de `internal/app` pela sua interface; providers e gitops mantêm
  seus testes atuais.
- **Frontend:** testes de stores, filtros, atalhos e componentes com Vitest.
- **Contrato:** validar serialização dos modelos Go/TypeScript e todos os comandos.
- **Visual:** snapshots em 600, 900 e 1440 px, tema claro e escuro.
- **Integração:** frontend em navegador com bridge falso para fluxos rápidos.
- **Smoke desktop:** abrir binário, carregar snapshot e executar uma ação segura
  em Linux, macOS e Windows.
- **Wayland:** rodada manual obrigatória em Hyprland, GNOME e KDE antes de cada
  release candidata; registrar compositor e versões de GTK/WebKitGTK.

## Sequência sugerida de pull requests

1. `Extract shared application module`
2. `Make terminal UI use application module`
3. `Scaffold Wails desktop application`
4. `Add adaptive pull request list`
5. `Add detail and conversation views`
6. `Add pull request actions and worktrees`
7. `Add instance and repository settings`
8. `Add desktop packaging and release jobs`
9. `Polish keyboard navigation and accessibility`

Cada PR deve manter a TUI utilizável e adicionar testes no seam correspondente.

## Estimativa

Para uma pessoa trabalhando com foco:

- módulo compartilhado e scaffold: **1–2 semanas**;
- navegação, lista, detalhe e conversa: **2–3 semanas**;
- ações, configuração e integração com terminais: **1–2 semanas**;
- empacotamento, acessibilidade e validação multiplataforma: **1–2 semanas**.

Uma primeira versão utilizável pode surgir no fim da terceira semana. A paridade
funcional e os instaladores assinados tendem a levar **5–8 semanas**, dependendo
principalmente da infraestrutura de assinatura e da profundidade do diff
integrado.

## Riscos que precisam de prova antecipada

1. **PATH no macOS:** aplicações iniciadas pelo Finder não recebem necessariamente
   o mesmo ambiente do shell.
2. **Terminal externo:** não existe um comando universal consistente no Linux;
   precisa de detecção e configuração explícita.
3. **WebKitGTK:** validar cedo Markdown, clipboard, seleção de texto, drag and drop
   e desempenho de listas grandes.
4. **Empacotamento Linux:** dependências e compatibilidade variam por distribuição;
   o pacote deve declarar GTK/WebKitGTK corretamente.
5. **Estado duplicado:** não deixar TUI e frontend reimplementarem regras de
   refresh, merge ou worktree; essas regras pertencem ao módulo compartilhado.

## Decisões adiadas de propósito

- diff integrado;
- tray icon e execução permanente em background;
- notificações de sistema por evento;
- atualização automática do aplicativo;
- migração para Wails v3.

Esses itens ficam fora do MVP para reduzir risco de plataforma e preservar a
profundidade do módulo compartilhado.
