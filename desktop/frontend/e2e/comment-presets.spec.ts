import { test, expect, type Page } from '@playwright/test';

async function mockServer(page: Page) {
  await page.addInitScript(() => {
    const cfg = {
      Language: 'pt',
      RefreshInterval: '5m',
      Terminal: 'auto',
      DiffTool: 'hunk',
      Instances: [
        {
          Name: 'github',
          Provider: 'github',
          Host: 'github.com',
          CommentPresets: [
            { Label: 'Aprovado', Body: 'aprovado\n\n**Testes verificados.**' },
            { Label: 'Reprovado', Body: 'reprovado' },
          ],
        },
        {
          Name: 'gitlab',
          Provider: 'gitlab',
          Host: 'git.company.com',
          CommentPresets: [
            { Label: 'Revisar', Body: 'Ajustar os testes antes do merge.' },
          ],
        },
      ],
      Repos: [],
    };
    const items = [
      { Instance: 'github', Title: 'PR GitHub', Kind: 0 },
      { Instance: 'gitlab', Title: 'MR GitLab', Kind: 0 },
      { Instance: 'github', Title: 'Issue GitHub', Kind: 1 },
    ].map((row, i) => ({
      Key: `item-${i}`,
      Ref: `#${i + 1}`,
      Clone: '',
      Worktree: '',
      Data: {
        ...row,
        Provider: row.Instance,
        Host: cfg.Instances.find((inst) => inst.Name === row.Instance)!.Host,
        Repo: 'team/service',
        Author: 'ana',
        Number: i + 1,
        Relations: 1,
        Comments: 0,
        UpdatedAt: new Date().toISOString(),
        Labels: [],
        Checks: [],
        ApprovedBy: [],
      },
    }));
    const state = {
      cfg,
      requests: [] as unknown[],
      fail: false,
      detailCalls: 0,
    };
    const view = () => ({
      Language: 'pt',
      RefreshSeconds: 300,
      Instances: state.cfg.Instances,
      Items: items,
      Errors: {},
      Revision: 0,
      SyncedAt: new Date().toISOString(),
    });
    Object.assign(window, {
      presetState: state,
      go: {
        main: {
          Bridge: {
            Load: async () => view(),
            Refresh: async () => view(),
            Theme: async () => ({}),
            Detail: async () => {
              state.detailCalls++;
              return { Body: 'Descrição', Comments: [] };
            },
            Settings: async () => structuredClone(state.cfg),
            SaveSettings: async (next: typeof cfg) => {
              state.cfg = structuredClone(next);
              return view();
            },
            Execute: async (request: unknown) => {
              state.requests.push(request);
              return {
                View: view(),
                Message: state.fail ? '' : 'Comentário enviado',
                Error: state.fail ? 'Falha ao enviar comentário' : '',
                ReloadThread: !state.fail,
              };
            },
          },
        },
      },
    });
  });
}

test.beforeEach(async ({ page }) => {
  await mockServer(page);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/');
  await expect(page.locator('.item-row')).toHaveCount(2);
});

test('preset buttons follow the PR/MR instance and do not appear on issues', async ({
  page,
}) => {
  const presets = page.locator('.comment-presets');
  await expect(presets.getByRole('button')).toHaveText([
    'Aprovado',
    'Reprovado',
  ]);
  await page
    .getByRole('button', { name: 'team/service #2: MR GitLab', exact: true })
    .click();
  await expect(presets.getByRole('button')).toHaveText(['Revisar']);
  await page.getByRole('button', { name: /^Conversa/ }).click();
  await expect(presets.getByRole('button', { name: 'Revisar' })).toBeVisible();
  await page.getByRole('button', { name: 'Issues', exact: true }).click();
  await expect(page.locator('.item-row')).toHaveCount(1);
  await expect(presets).toHaveCount(0);
});

test('preset confirmation previews exact body, cancel sends nothing, and send preserves drafts', async ({
  page,
}) => {
  await page.getByRole('button', { name: /^Conversa/ }).click();
  await page.locator('#comment').fill('Rascunho ainda em edição');
  const approved = page
    .locator('.comment-presets')
    .getByRole('button', { name: 'Aprovado', exact: true });
  await approved.click();
  const dialog = page.getByRole('dialog', { name: 'Enviar comentário?' });
  await expect(dialog).toContainText('team/service #1');
  await expect(dialog.locator('.comment-preview')).toHaveText(
    'aprovado\n\n**Testes verificados.**',
  );
  expect(
    await page.evaluate(() => (window as any).presetState.requests),
  ).toEqual([]);
  await dialog.getByRole('button', { name: 'Cancelar', exact: true }).click();
  await expect(page.locator('#comment')).toHaveValue(
    'Rascunho ainda em edição',
  );
  await approved.click();
  await page.screenshot({
    path: '../../dist/desktop-preview/preset-comment-confirmation.png',
  });
  await dialog.getByRole('button', { name: 'Confirmar', exact: true }).click();
  await expect(page.getByRole('status')).toContainText('Comentário enviado');
  expect(
    await page.evaluate(() => (window as any).presetState.requests),
  ).toEqual([
    {
      Key: 'item-0',
      Action: 'comment',
      Body: 'aprovado\n\n**Testes verificados.**',
      Force: false,
      RemoveAfter: false,
      Push: false,
    },
  ]);
  await expect(page.locator('#comment')).toHaveValue(
    'Rascunho ainda em edição',
  );
  await page.evaluate(() => {
    (window as any).presetState.fail = true;
  });
  await approved.click();
  await dialog.getByRole('button', { name: 'Confirmar', exact: true }).click();
  await expect(page.getByRole('alert')).toContainText(
    'Falha ao enviar comentário',
  );
  await expect(page.locator('#comment')).toHaveValue(
    'Rascunho ainda em edição',
  );
});

test('settings add, edit, remove and persist presets for each server', async ({
  page,
}) => {
  await page
    .getByRole('button', { name: 'Configurações', exact: true })
    .click();
  const settings = page.getByRole('dialog', { name: 'Configurações' });
  const github = settings.locator(':scope > fieldset').nth(0);
  await github
    .getByRole('textbox', { name: 'Texto do comentário', exact: true })
    .first()
    .fill('aprovado com ressalvas');
  await github
    .getByRole('button', { name: 'Remover comentário 2', exact: true })
    .click();
  await github
    .getByRole('button', { name: 'Adicionar comentário predefinido' })
    .click();
  await github
    .getByRole('textbox', { name: 'Nome do botão' })
    .nth(1)
    .fill('Precisa de testes');
  await github
    .getByRole('textbox', { name: 'Texto do comentário' })
    .nth(1)
    .fill('Adicione testes.\n\nObrigado.');
  await settings
    .getByRole('button', { name: 'Salvar configurações', exact: true })
    .click();
  await expect(settings).not.toBeVisible();
  await expect(page.locator('.comment-presets button')).toHaveText([
    'Aprovado',
    'Precisa de testes',
  ]);
  const cfg = await page.evaluate(() => (window as any).presetState.cfg);
  expect(cfg.Instances[0].CommentPresets).toEqual([
    { Label: 'Aprovado', Body: 'aprovado com ressalvas' },
    { Label: 'Precisa de testes', Body: 'Adicione testes.\n\nObrigado.' },
  ]);
  expect(cfg.Instances[1].CommentPresets).toEqual([
    { Label: 'Revisar', Body: 'Ajustar os testes antes do merge.' },
  ]);
  await page
    .getByRole('button', { name: 'Configurações', exact: true })
    .click();
  await expect(
    github.getByRole('textbox', { name: 'Texto do comentário' }).nth(1),
  ).toHaveValue('Adicione testes.\n\nObrigado.');
  await github.locator('.preset-editor').scrollIntoViewIfNeeded();
  await page.screenshot({
    path: '../../dist/desktop-preview/preset-comment-settings.png',
  });
  await github
    .getByRole('button', { name: 'Remover comentário 2', exact: true })
    .click();
  await github
    .getByRole('button', { name: 'Remover comentário 1', exact: true })
    .click();
  await settings
    .getByRole('button', { name: 'Salvar configurações', exact: true })
    .click();
  await expect(page.locator('.comment-presets')).toHaveCount(0);
});
