# Gitea: a integração com o `tea`

O provider `gitea` (`internal/provider/gitea.go`) fala com um servidor Gitea
através do [`tea`](https://gitea.com/gitea/tea), o CLI oficial mantido pela
própria Gitea. Como em GitHub e GitLab, **nenhum token passa pelo pr-tracker**:
a autenticação é a que o `tea` já tem.

Este documento registra o que o `tea` cobre, o que precisou ir direto na API e
as decisões que não são óbvias no código. Levantamento feito com `tea` 0.15.1
contra um Gitea 1.27.3.

## Requisitos

| | |
|---|---|
| Binário | `tea`, **0.14 ou mais novo** |
| Por quê | `tea api` existe desde a 0.12; aceitar um *slug* de repositório fora de um clone é da 0.14.1 |
| Instalação | `brew install tea`, pacote `tea-cli` no Debian, ou um binário das [releases](https://gitea.com/gitea/tea/releases) |
| Login | `tea login add --url https://git.empresa.com --token <token>` |

O provider checa a versão uma vez por instância. A checagem também serve de
defesa contra outro programa chamado `tea` no PATH — o nome é genérico o
bastante para colidir.

## Mapeamento de `provider.Client`

| Método | Implementação | Por quê |
|---|---|---|
| `List` | `tea api /repos/issues/search` + enriquecimento | único endpoint que filtra por relação com o usuário |
| `Thread` | `tea api` em `/issues/{n}`, `/issues/{n}/comments`, `/pulls/{n}/reviews` e `/reviews/{id}/comments` | o Gitea guarda reviews e comentários inline fora da lista de comentários do issue |
| `AddComment` | `tea api -X POST …/issues/{n}/comments` | evita depender do `tea comments add` (0.14.2+), que em versões anteriores travava no stdin |
| `Approve` | `tea pulls approve` | |
| `Merge` | `tea api -X POST …/pulls/{n}/merge` | `tea pulls merge` só expõe `--style`; apagar a branch e "merge quando os checks passarem" são opções da instância |
| `Close` | `tea pulls close` | |
| `Checkout` | `tea pulls checkout --branch`, rodando dentro do clone | |
| `HeadRef` | `refs/pull/{n}/head` | o Gitea mantém esse ref no repositório base, forks incluídos — o `gitops` funciona sem mudança |
| `AuthStatus` | `tea api /user` | ver "login sem token", abaixo |

## Como a lista é montada

O Gitea não tem GraphQL, então não existe o equivalente à chamada única que
`github.go` e `gitlab.go` fazem. O filtro "relacionado a mim" está em
`GET /repos/issues/search`, e cobre exatamente as abas do pr-tracker:

| Aba | Parâmetros |
|---|---|
| PRs · Revisar | `type=pulls&review_requested=true` |
| PRs · Meus | `type=pulls&created=true` |
| PRs · Atribuídos | `type=pulls&assigned=true` |
| Issues · Atribuídas / Criadas / Menções | `type=issues` + `assigned` / `created` / `mentioned` |

As seis buscas rodam em paralelo, com `limit=50` — o mesmo teto que `gh` e
`glab` usam aqui, e acima do qual o Gitea trunca de qualquer jeito
(`max_response_items`, 50 por padrão).

O problema é o payload: a busca devolve o **issue cru**, sem branch, diff stats,
`mergeable`, CI nem reviews. Por isso cada PR passa por até três chamadas:

- `/repos/{o}/{r}/pulls/{n}` → branches, `head.sha`, diff stats, `mergeable`,
  fork (`head.repo.full_name != base.repo.full_name`);
- `/repos/{o}/{r}/commits/{sha}/status` → o status combinado, que numa chamada
  só traz o estado agregado e a lista de checks;
- `/repos/{o}/{r}/pulls/{n}/reviews` → aprovações.

No máximo **seis PRs por vez**, porque servidor Gitea costuma ser pequeno e um
refresh não deve parecer um pico de tráfego. Falhas de enriquecimento são
toleradas de propósito: uma linha sem CI é melhor do que perder a lista inteira.

Buscar `/repos/{o}/{r}/pulls?state=open` uma vez por repositório sairia mais
barato em repositórios com muitos PRs relevantes, mas um PR além da primeira
página sumiria da lista — por isso o detalhe é por PR mesmo.

## Decisões que não são óbvias no código

**Login por nome, não por host.** `gh` tem `GH_HOST` e `glab` tem
`GITLAB_HOST`; o `tea` tem um catálogo de logins nomeados. O provider lê
`tea logins list --output json` uma vez e casa pela URL. Se o mesmo host tiver
mais de uma conta, uma instância com o mesmo nome do login fixa a escolha;
senão vale o login default. A URL guardada pelo `tea` também vira a base das
URLs, o que mantém servidores em HTTP puro ou em porta não padrão funcionando.

**Login sem token falha em silêncio.** Um login pode existir sem token usável
(criado só com ssh-key, por exemplo). Nesse caso `/repos/issues/search` continua
respondendo **200 com o que é público**, ou seja, a lista viria vazia e sem erro
nenhum. Por isso tanto `List` quanto `AuthStatus` passam antes por `/user`, que
é o endpoint que de fato exige autenticação.

**`tea api` sai com código 0 mesmo em 4xx.** O único sinal de falha é o envelope
`{"message": …}` no corpo, então toda resposta passa por `apiError`. O envelope
vira um erro tipado para que a dica de login possa ser anexada quando o problema
for autenticação.

**Não existe decisão de review agregada.** O Gitea não tem nada como o
`reviewDecision` do GitHub, então ela é derivada das reviews: a última de cada
usuário vale, descartadas são ignoradas, comentários não decidem nada, e um
pedido de alterações vence as aprovações.

**Draft não é conflito.** O `mergeable` do Gitea é falso para rascunhos e
também enquanto a checagem de conflito ainda está rodando. Rascunhos são
filtrados; uma checagem em andamento se resolve no refresh seguinte.

**`target_url` relativo.** Os checks das Gitea Actions vêm com URL relativa
(`/owner/repo/actions/runs/1/jobs/1`), que é completada com a base do servidor —
o mesmo tratamento que o `gitlab.go` dá ao `webPath`.

**Campo `Do` do merge.** O swagger do Gitea 1.27 expõe o campo como `do`, mas
versões mais antigas o ligam como `Do`. A requisição manda os dois; campos
desconhecidos são ignorados pelo servidor.

**Numeração compartilhada.** Issues e PRs dividem o mesmo contador dentro de um
repositório, então `Ref()` continua sendo `#N` e `Key()` já funcionava.

## Limitações conhecidas

- **Forgejo/Codeberg** devem funcionar (a API é compatível), mas não foram
  testados. O `tea` tem `--no-version-check` no `login add` se a verificação de
  versão reclamar.
- Uma única página de 50 itens por busca, como nos outros providers.
- Comandos interativos do `tea` (`tea pulls review`, o `--comments` do
  `tea pulls`) não são usados; o `run()` do pacote já zera o stdin.

## Como verificar à mão

```sh
tea --version
tea logins list -o json
tea api -l <login> /version
tea api -l <login> /user
tea api -l <login> '/repos/issues/search?type=pulls&state=open&review_requested=true&limit=50'
tea api -l <login> '/repos/{owner}/{repo}/pulls/{n}'
tea api -l <login> '/repos/{owner}/{repo}/commits/{sha}/status'
tea api -l <login> '/repos/{owner}/{repo}/pulls/{n}/reviews'
```
