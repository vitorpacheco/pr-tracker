# Bitbucket: investigação de CLI (setembro/2026)

**Resumo:** a Atlassian não oferece um CLI oficial para pull requests do Bitbucket.
Por isso a integração ficou para depois. Hoje o provider `bitbucket` é só um
placeholder: dá para cadastrar a instância, mas ela aparece como "não suportado
ainda" e não é consultada.

## O que foi avaliado

| Ferramenta | Mantenedor | Bitbucket Cloud | Data Center / Server | PRs (listar, aprovar, merge) | Pipelines / builds | Observações |
|---|---|---|---|---|---|---|
| `acli` (Atlassian CLI) | **Atlassian (oficial)** | ✘ | ✘ | ✘ | ✘ | Os comandos cobrem só `jira`, `admin` e `rovodev`. Não tem nada de Bitbucket. |
| [`bkt`](https://github.com/avivsinai/bitbucket-cli) (avivsinai/bitbucket-cli) | comunidade | ✔ | ✔ | ✔ `pr list/view/approve/merge/checkout` | ✔ `pr checks`, `pipeline run`, `status pipeline` | Tem ergonomia parecida com o `gh`, saída `--json`/`--yaml` e várias contas/hosts (`bkt auth login <host>`), com credenciais no keychain do SO. Está ativo (v0.2x) e dá para instalar via brew, winget, scoop, nix ou `go install`. |
| [`atlassian-cli`](https://github.com/omar16100/atlassian-cli) | comunidade | ✔ | ? | ✔ `bitbucket pr create/merge/approve` | ✔ | É um binário único em Rust para Jira, Confluence e Bitbucket. O foco é Atlassian Cloud. |

## Recomendação para a integração futura

O candidato natural é o **`bkt`**, porque é o que mais se parece com `gh`/`glab`:

- Ele suporta Cloud e Data Center (self-hosted), com várias instâncias. Isso é
  requisito do pr-tracker.
- A saída estruturada (`--json`) permite implementar `provider.Client` do mesmo
  jeito que `github.go` e `gitlab.go`.
- O `bkt pr checks` expõe o status dos builds e pipelines de cada PR.
- Os PRs do Bitbucket têm ref de head no servidor. No Data Center é
  `refs/pull-requests/<id>/from`. No Cloud **não existe** ref equivalente, então
  o worktree teria que fazer fetch da branch de origem (ou do fork).

Por não ser oficial, adicionar o `bkt` como dependência é uma decisão em aberto.

Para implementar, basta completar `internal/provider/bitbucket.go` (hoje ele
retorna `ErrNotSupported`). A UI já trata o provider `bitbucket` e a ferramenta
`bkt` nas mensagens de "CLI não instalado".
