# pr-tracker

Interface de terminal para acompanhar **pull requests** (GitHub) e **merge
requests** (GitLab) de várias instâncias ao mesmo tempo, incluindo instâncias
self-hosted. Toda a comunicação passa pelos CLIs oficiais `gh` e `glab`, então a
autenticação é a mesma que você já usa neles.

- Tem abas **Revisar** (revisão pedida a você), **Meus**, **Atribuídos** e **Todos**.
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
| `1`–`4`, `tab` | trocar aba |
| `/` | filtrar (título, repo, autor, branch) |
| `enter` / clique | menu de ações do PR |
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

Cada instância faz **uma** chamada GraphQL por atualização:

- GitHub: `gh api graphql --hostname <host>`, com três buscas
  (`review-requested:@me`, `author:@me`, `assignee:@me`) mais o
  `statusCheckRollup` do último commit.
- GitLab: `glab api graphql --hostname <host>`, com
  `currentUser.{reviewRequested,authored,assigned}MergeRequests` mais o
  `headPipeline` e seus jobs.

As ações usam `gh pr review/merge/checkout -R host/owner/repo` e
`glab mr approve/merge/checkout -R <url do projeto>`.

## Desenvolvimento

```sh
make          # lista os comandos
make check    # gofmt + go vet + testes
make run ARGS=doctor
make dist     # binários para Linux, macOS e Windows em ./dist
```

A estrutura é `internal/config` (arquivo e caminhos), `internal/provider`
(gh/glab/bitbucket), `internal/gitops` (clones e worktrees), `internal/launch`
(herdr/tmux/inline/navegador) e `internal/ui` (Bubble Tea v2).
