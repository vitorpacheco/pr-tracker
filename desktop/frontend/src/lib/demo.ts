import type { Item } from './list';
const rows = [
  [
    'platform/web',
    482,
    'Adiciona autenticação via token para API interna',
    'mrlima',
    'success',
    1,
  ],
  [
    'services/auth',
    317,
    'Refatora fluxo de renovação de sessão',
    'jcarvalho',
    'pending',
    1,
  ],
  [
    'platform/cli',
    219,
    'Melhora cache do pipeline e reduz tempo de build',
    'fernandes',
    'success',
    1,
  ],
  [
    'ui/components',
    741,
    'Novo componente de tabela com virtualização',
    'anacosta',
    'pending',
    2,
  ],
  [
    'services/payments',
    603,
    'Corrige tratamento de falha no webhook',
    'dlima',
    'failure',
    4,
  ],
  [
    'platform/cli',
    518,
    'Adiciona comando para gerenciar worktrees',
    'rbarbosa',
    'success',
    1,
  ],
  ['docs', 402, 'Atualiza documentação de contribuição', 'cferreira', '', 2],
  [
    'infra/k8s',
    288,
    'Ajusta probes de readiness e liveness',
    'mrlima',
    'success',
    4,
  ],
];
export const items = rows.map((r, i) => ({
  Kind: 0,
  Instance: i % 3 ? 'Corp Git' : 'Work Git',
  Provider: 'github',
  Host: 'github.com',
  Repo: String(r[0]),
  RepoURL: '',
  Number: Number(r[1]),
  Title: String(r[2]),
  URL: 'https://example.com',
  Author: String(r[3]),
  Draft: i === 3,
  SourceBranch: 'feature/token-auth',
  TargetBranch: 'main',
  FromFork: false,
  CreatedAt: new Date(Date.now() - 86400000).toISOString(),
  UpdatedAt: new Date(Date.now() - (i + 1) * 720000).toISOString(),
  Additions: 328,
  Deletions: 42,
  Files: 12,
  Review: i === 0 ? 'approved' : 'review_required',
  ApprovedBy: i === 0 ? ['jcarvalho', 'anacosta'] : [],
  ApprovedByMe: false,
  Conflicts: false,
  CI: String(r[4]),
  Checks: [
    { Name: 'build', State: 'success', URL: '' },
    { Name: 'test', State: 'success', URL: '' },
    { Name: 'lint', State: String(r[4]), URL: '' },
  ],
  Labels: ['enhancement', 'security'],
  Assignees: [],
  Comments: i + 2,
  Relations: Number(r[5]),
  Key: `demo-${i}`,
  Ref: `#${r[1]}`,
  Clone: '',
  Worktree: i === 0 ? '/example/worktree' : '',
})) as Item[];
items.push({
  ...items[0],
  Kind: 1,
  Key: 'issue-1',
  Title: 'Documentar configuração dos providers',
  Relations: 4,
  Ref: '#88',
  Number: 88,
});
export async function invoke(name: string, _args: unknown[]): Promise<unknown> {
  if (name === 'Load' || name === 'Refresh')
    return {
      RefreshSeconds: 300,
      Instances: [
        { Name: 'Corp Git', Disabled: false },
        { Name: 'Work Git', Disabled: false },
      ],
      Items: items,
      Errors: {},
      CacheError: '',
      SyncedAt: new Date().toISOString(),
      Revision: 0,
    };
  if (name === 'Detail')
    return {
      Body: 'Implementa autenticação via **Bearer token** para os endpoints da API interna.\n\n- Validação de escopos\n- Middleware de segurança\n- Cobertura de testes\n\n```go\nrequest.Header.Set("Authorization", "Bearer " + token)\n```',
      Comments: [
        {
          Author: 'jcarvalho',
          Body: 'A implementação ficou clara. Os testes de escopo estão passando.',
          CreatedAt: new Date().toISOString(),
          Review: 'approved',
          Path: '',
          Line: 0,
        },
      ],
    };
  if (name === 'Settings')
    return {
      RefreshInterval: '5m',
      Terminal: 'auto',
      DiffTool: 'hunk',
      WorktreeDir: '',
      CloneRoots: [],
      Instances: [
        {
          Name: 'Corp Git',
          Provider: 'github',
          Host: 'github.com',
          Disabled: false,
          MergeMethod: 'squash',
          AutoMerge: false,
          DeleteBranch: false,
        },
        {
          Name: 'Work Git',
          Provider: 'gitlab',
          Host: 'gitlab.com',
          Disabled: false,
          MergeMethod: 'merge',
          AutoMerge: false,
          DeleteBranch: false,
        },
      ],
      Repos: [],
    };
  if (name === 'MergeOptions')
    return { Method: 'squash', Auto: false, DeleteBranch: false };
  if (name === 'Diagnose')
    return {
      Tools: [
        { Name: 'git', Path: '/usr/bin/git', Hint: '' },
        { Name: 'gh', Path: '/usr/bin/gh', Hint: '' },
      ],
      Instances: [
        { Name: 'Corp Git', Disabled: false, Error: '' },
        { Name: 'Work Git', Disabled: false, Error: '' },
      ],
    };
  throw new Error('Demonstração: esta ação não altera dados.');
}
