# pr-tracker

Interface de terminal para acompanhar **pull requests** (GitHub), **merge
requests** (GitLab), **issues** e os **comentários** de ambos em várias instâncias
ao mesmo tempo, incluindo instâncias
self-hosted. Toda a comunicação passa pelos CLIs oficiais `gh` e `glab`, então a
autenticação é a mesma que você já usa neles.

- Tem abas de PRs: **Revisar** (revisão pedida a você), **Meus**, **Atribuídos** e
  **Todos**. As abas de issues abertas são **Atribuídas**, **Criadas** e **Menções**.
- Mostra a **conversa** (`v`) com a descrição e os comentários em markdown
  renderizado. Nos PRs, também aparecem reviews (aprovou / pediu alterações) e
  comentários inline com `arquivo:linha`. Para **comentar**, use `n`.
- Mostra o status de **pipeline/checks** (sucesso, falha, em andamento,
  cancelado) com a lista de jobs, além da revisão, aprovações, conflitos e
  tamanho do diff.
- Atualiza sozinho num intervalo configurável (padrão **5 min**), ou na hora com `r`.
- Aceita teclado e mouse: dá para clicar em abas, linhas, itens de menu, botões e
  nos atalhos do rodapé. Todo atalho aparece na própria interface (`?` abre a ajuda).
- Faz checkout em **worktree** isolado ou no **clone configurado**, e depois
  permite aprovar ou fazer merge, removendo o worktree ou não.
- Integrações opcionais: **hunk** para o diff; **herdr** ou **tmux** para abrir
  terminal/diff em nova aba.

## Requisitos

| Ferramenta | Uso | Obrigatória |
|---|---|---|
| `git` | worktrees e checkout | sim |
| [`gh`](https://cli.github.com) | instâncias GitHub (github.com e GHES) | para GitHub |
| [`glab`](https://gitlab.com/gitlab-org/cli) | instâncias GitLab (gitlab.com e self-hosted) | para GitLab |
| [`hunk`](https://github.com/modem-dev/hunk) | visualizar diff (sem ele, usa `git diff`) | não |
| `herdr` / `tmux` | abrir terminal/diff em nova aba | não |

Se faltar um CLI, a interface mostra um aviso com o link de instalação, e
`pr-tracker doctor` lista tudo o que falta. Sobre o **Bitbucket**, veja
[docs/bitbucket.md](docs/bitbucket.md): não há CLI oficial e a integração ficou
para depois.

## Instalação

Baixe um binário na página de
[releases](https://github.com/vitorpacheco/pr-tracker/releases). Há uma versão
estável para cada tag `v*` e uma `nightly`, que é atualizada a cada commit na
`main`. Também dá para compilar:

```sh
go install github.com/vitorpacheco/pr-tracker@latest
# ou, a partir do clone (instala em ~/.local/bin; mude com PREFIX=...):
make install
```

## Uso

```sh
pr-tracker                 # interface
pr-tracker doctor          # verifica CLIs, autenticação e caminhos
pr-tracker list            # lista os PRs no stdout
pr-tracker instance add --provider gitlab --host gitlab.empresa.com --name trabalho --merge-method squash
pr-tracker instance list
pr-tracker repo set --instance trabalho --repo grupo/sub/projeto --path ~/code/projeto
```

Na primeira execução, o arquivo de configuração é criado e já recebe
`github.com`/`gitlab.com` se `gh`/`glab` estiverem autenticados nesses hosts.

Instâncias self-hosted precisam estar autenticadas no CLI:

```sh
gh auth login --hostname github.empresa.com
glab auth login --hostname gitlab.empresa.com
```

### Atalhos

| Tecla | Ação |
|---|---|
| `↑↓` `j k`, `g G`, `pgup pgdn` | navegar |
| `1`–`4` / `5`–`7`, `tab` | abas de PRs / issues |
| `/` | filtrar (título, repo, autor, branch) |
| `enter` / clique | menu de ações do item |
| `v` | ver a conversa (descrição, comentários e reviews) |
| `n` | escrever um comentário (`ctrl+s` envia, `esc` cancela) |
| `w` | checkout em worktree (ou atualizar um existente) |
| `c` | checkout no clone local configurado (`gh pr checkout` / `glab mr checkout`) |
| `d` | diff no hunk (ou `git diff`) dentro do worktree |
| `t` | abrir terminal no worktree |
| `a` / `A` | aprovar / aprovar e remover o worktree |
| `m` / `M` | merge / merge e remover o worktree |
| `x` | remover o worktree sem aprovar |
| `o` | abrir no navegador |
| `p` | definir a pasta local do repositório |
| `r` | atualizar agora |
| `i` | instâncias (`n` nova, `e` editar, `space` ativar/desativar, `t` testar auth, `D` remover) |
| `s` | configurações |
| `?` | ajuda |
| `q` | sair |

Em issues só existem `v`, `n` e `o`. As ações de worktree, checkout, aprovação
e merge valem apenas para PRs. Na tela de conversa, a navegação é com `↑↓`,
`space`/`pgdn`, `g`/`G` e roda do mouse; `r` recarrega e `esc` volta.

Aprovar, fazer merge, fazer checkout e remover sempre pedem confirmação. Se o
worktree tiver alterações não commitadas, a remoção pede uma segunda
confirmação antes de forçar.

## Worktrees

Os worktrees ficam em `~/.pr-tracker/worktrees/<repo>-<número>-<hash>`. O hash
tem 8 caracteres, é estável e vem de host+repo+número, o que evita colisão entre
instâncias e repositórios com o mesmo nome.

- O head do PR é buscado por um ref do servidor que também funciona para forks
  (`refs/pull/N/head` no GitHub, `refs/merge-requests/N/head` no GitLab). O
  worktree fica na branch local `pr-tracker/<N>-<hash>`.
- Em PRs do mesmo repositório, a branch local acompanha `origin/<branch>`
  (`git pull` funciona).
- Remover o worktree também apaga a branch e o ref locais criados pelo pr-tracker.

O clone local de cada repositório vem do mapeamento em `[[repos]]` (tecla `p` ou
`pr-tracker repo set`). Sem mapeamento, o pr-tracker procura em `clone_roots`
(`<root>/<repo>`, `<root>/<owner>/<repo>`, `<root>/<host>/<owner>/<repo>`) e só
aceita a pasta se algum remote apontar para o repositório. Se nada for
encontrado, ele pergunta a pasta na hora.

## Terminal: herdr, tmux ou inline

`terminal = "auto"` escolhe **herdr** quando roda dentro dele (`HERDR_ENV=1`,
nova aba via `herdr tab create` + `herdr pane run`). Senão, escolhe **tmux**
quando `$TMUX` está definido (`tmux new-window`). Fora dos dois, a interface é
suspensa e o terminal ou diff abre no lugar dela, voltando quando você sai.

## Configuração

| SO | Arquivo |
|---|---|
| Linux | `$XDG_CONFIG_HOME/pr-tracker/config.toml` (padrão `~/.config/pr-tracker/config.toml`) |
| macOS | `~/.config/pr-tracker/config.toml` |
| Windows | `%USERPROFILE%\.config\pr-tracker\config.toml` |

`PR_TRACKER_CONFIG` sobrescreve o caminho. Exemplo:

```toml
refresh_interval = "5m"       # 90s, 5m, 1h… (mínimo 15s)
terminal = "auto"             # auto | herdr | tmux | inline
diff_tool = "hunk"            # hunk | git
# worktree_dir = "~/.pr-tracker/worktrees"
clone_roots = ["~/code", "~/work"]

[[instances]]
  name = "github.com"
  provider = "github"
  host = "github.com"
  merge_method = "squash"     # merge | squash | rebase

[[instances]]
  name = "trabalho"
  provider = "gitlab"
  host = "gitlab.empresa.com"
  merge_method = "merge"
  auto_merge = true           # merge quando o pipeline passar
  delete_branch = true        # apaga a branch de origem após o merge

[[repos]]
  instance = "trabalho"
  name = "grupo/sub/projeto"
  path = "~/work/projeto"
  remote = "origin"           # opcional; detectado pela URL
  merge_method = "squash"     # opcional; sobrescreve a instância
```

## Como os dados são obtidos

A cada atualização, cada instância faz poucas chamadas GraphQL:

- GitHub, em **uma** chamada `gh api graphql --hostname <host>`: três buscas de
  PRs (`review-requested:@me`, `author:@me`, `assignee:@me`, com o
  `statusCheckRollup` do último commit) e três de issues (`assignee:@me`,
  `author:@me`, `mentions:@me`).
- GitLab, em **duas** chamadas `glab api graphql --hostname <host>`:
  - `currentUser.{reviewRequested,authored,assigned}MergeRequests`, com o
    `headPipeline` e seus jobs;
  - `issues(assigneeUsernames|authorUsername)` e os to-dos pendentes de menção
    (`mentioned`, `directly_addressed`).
  As MRs ficam sem labels para respeitar o limite de complexidade de query do
  GitLab.

### Cache local

A interface mantém em SQLite o último snapshot completo obtido com sucesso de
cada instância. Ao abrir o `pr-tracker`, esse snapshot aparece imediatamente
enquanto uma atualização remota roda em segundo plano. Se GitHub, GitLab ou a
rede estiverem indisponíveis, os dados anteriores continuam visíveis junto do
aviso de falha.

O banco é um cache descartável, não a fonte de verdade. Cada atualização
bem-sucedida substitui atomicamente o snapshot daquela instância; falhas nunca
apagam o snapshot anterior. Por padrão, o arquivo fica no diretório de cache do
sistema (`$XDG_CACHE_HOME/pr-tracker/cache.db` ou `~/.cache/pr-tracker/cache.db`
no Linux). `PR_TRACKER_CACHE` sobrescreve o caminho. O arquivo pode ser apagado
com o programa fechado e será reconstruído na atualização seguinte. Ele tem
modo `0600` em sistemas Unix; no Windows, segue as ACLs herdadas do diretório
de cache do usuário. Contém metadados dos itens, mas não credenciais nem as
conversas carregadas sob demanda.

A conversa só é carregada quando você abre (`v`):

- GitHub: `issueOrPullRequest`, com comentários, reviews e comentários inline.
- GitLab: `notes(filter: ONLY_COMMENTS)`, que ignora as notas de sistema.

Para comentar, o pr-tracker usa `gh pr|issue comment` e
`glab api POST projects/:id/(merge_requests|issues)/:iid/notes`.

As ações usam `gh pr review/merge/checkout -R host/owner/repo` e
`glab mr approve/merge/checkout -R <url do projeto>`.

## Desenvolvimento

```sh
make          # lista os comandos
make check    # gofmt + go vet + testes
make run ARGS=doctor
make vuln     # govulncheck: vulnerabilidades conhecidas nas dependências
make dist     # binários para Linux, macOS e Windows em ./dist
make package  # dist + .tar.gz/.zip + checksums.txt (o que a release publica)
```

### CI e releases (GitHub Actions)

| Workflow | Quando roda | O que faz |
|---|---|---|
| `ci.yml` | PRs e pushes em outras branches | `go mod tidy` limpo, `make lint`, testes em Linux (com `-race`), macOS e Windows, `make package` (artefato por 7 dias) e `govulncheck` |
| `nightly.yml` | push na `main` | roda o CI e recria a pre-release `nightly`, que aponta para o novo commit (versão `nightly-AAAAMMDD-<sha>`) |
| `release.yml` | push de tag `v*` | roda o CI e publica a release com notas geradas; tags com sufixo (`v1.0.0-rc.1`) viram pre-release |
| `vulncheck.yml` | toda segunda e manual | `govulncheck` na `main`, para pegar alertas novos sem depender de commit |

O Dependabot atualiza as dependências Go e as actions uma vez por semana.
Para publicar uma versão:

```sh
git tag -a v0.1.0 -m "v0.1.0" && git push origin v0.1.0
```

A estrutura é `internal/config` (arquivo e caminhos), `internal/provider`
(gh/glab/bitbucket), `internal/gitops` (clones e worktrees), `internal/launch`
(herdr/tmux/inline/navegador) e `internal/ui` (Bubble Tea v2).
