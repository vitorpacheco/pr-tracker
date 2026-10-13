import { test, expect, type Page } from '@playwright/test';

async function setup(page: Page, provider = 'github', holds: string[] = []) {
  await page.addInitScript(
    ({ provider, holds }) => {
      const data = {
        Kind: 0,
        Instance: 'work',
        Provider: provider,
        Host: 'git.company.com',
        Repo: 'team/service',
        Number: 7,
        Title: 'Adicionar testes',
        Author: 'ana',
        SourceBranch: 'feature/tests',
        TargetBranch: 'main',
        BehindBase: true,
        CanUpdateBranch: true,
        Relations: 1,
        UpdatedAt: new Date().toISOString(),
        Labels: [],
        Checks: [],
        ApprovedBy: [],
        Conflicts: false,
      };
      const state = {
        holds: new Set(holds),
        finish: {} as Record<string, () => void>,
        failures: {} as Record<string, boolean>,
        requests: [] as any[],
      };
      async function gate(name: string) {
        if (state.holds.has(name))
          await new Promise<void>((resolve) => {
            state.finish[name] = resolve;
          });
        if (state.failures[name]) {
          delete state.failures[name];
          throw new Error('Falha simulada');
        }
      }
      const view = () => ({
        Language: 'pt',
        Revision: 0,
        RefreshSeconds: 300,
        Instances: [],
        Errors: {},
        Items: [
          {
            Key: 'pr-7',
            Ref: provider === 'gitlab' ? '!7' : '#7',
            Clone: '',
            Worktree: '',
            Data: data,
          },
        ],
      });
      Object.assign(window, {
        activityState: state,
        go: {
          main: {
            Bridge: {
              Load: async () => {
                await gate('load');
                return view();
              },
              Refresh: async () => {
                await gate('refresh');
                return view();
              },
              Theme: async () => ({}),
              Detail: async () => {
                await gate('detail');
                return { Body: 'Descrição', Comments: [] };
              },
              Settings: async () => {
                await gate('settings');
                return {
                  Instances: [],
                  Repos: [],
                  ToolPaths: {},
                  Language: 'pt',
                };
              },
              SaveSettings: async () => {
                await gate('save');
                return view();
              },
              Diagnose: async () => {
                await gate('diagnose');
                return { Tools: [], Instances: [] };
              },
              PickTheme: async () => {
                await gate('pick');
                return '';
              },
              ExportTheme: async () => {
                await gate('export');
                return '';
              },
              MergeOptions: async () => {
                await gate('merge-options');
                return { Method: 'merge' };
              },
              Execute: async (request: any) => {
                state.requests.push(request);
                await gate('execute');
                if (request.Action === 'update_branch') {
                  data.BehindBase = false;
                  data.CanUpdateBranch = false;
                }
                return {
                  View: view(),
                  Error: '',
                  Message: 'Solicitação enviada',
                  ReloadThread: false,
                };
              },
            },
          },
        },
      });
    },
    { provider, holds },
  );
}

async function release(page: Page, name: string, fail = false) {
  await page.evaluate(
    ({ name, fail }) => {
      const state = (window as any).activityState;
      state.holds.delete(name);
      state.failures[name] = fail;
      state.finish[name]();
    },
    { name, fail },
  );
}

test('initial load, refresh, conversation and merge preview report real activity', async ({
  page,
}) => {
  await setup(page, 'github', ['load', 'refresh', 'detail', 'merge-options']);
  await page.goto('/');
  await expect(
    page.locator('.notifications').getByRole('progressbar'),
  ).toHaveAttribute('aria-label', 'Carregando sua fila');
  await release(page, 'load');
  await expect(page.locator('.item-row')).toHaveCount(1);
  await expect(
    page.getByRole('button', { name: 'Atualizar', exact: true }),
  ).toBeDisabled();
  await expect(
    page.locator('.notifications').getByRole('progressbar'),
  ).toHaveAttribute('aria-label', 'Sincronizando itens…');
  await expect(
    page.locator('.detail-scroll').getByRole('progressbar'),
  ).toHaveAttribute('aria-label', 'Carregando conversa…');
  await release(page, 'refresh');
  await expect(
    page.getByRole('button', { name: 'Atualizar', exact: true }),
  ).toBeEnabled();
  await release(page, 'detail', true);
  await expect(
    page.locator('.detail-scroll').getByRole('progressbar'),
  ).toHaveCount(0);
  await expect(
    page.getByRole('button', { name: 'Tentar novamente' }),
  ).toBeVisible();
  await page.getByRole('button', { name: 'Tentar novamente' }).click();
  await expect(
    page.getByRole('button', { name: 'Tentar novamente' }),
  ).toHaveCount(0);
  await page.getByRole('button', { name: 'Merge', exact: true }).click();
  await expect(
    page.locator('.notifications').getByRole('progressbar'),
  ).toHaveAttribute('aria-label', 'Carregando opções de merge…');
  await release(page, 'merge-options', true);
  await expect(
    page.locator('.notifications').getByRole('progressbar'),
  ).toHaveCount(0);
  await expect(page.getByRole('alert')).toContainText('Falha simulada');
  await expect(page.getByRole('dialog')).toHaveCount(0);
});

test('settings operations show their own label and clear indicators after errors', async ({
  page,
}) => {
  await setup(page, 'github', [
    'settings',
    'diagnose',
    'save',
    'pick',
    'export',
  ]);
  await page.goto('/');
  const settingsButton = page.getByRole('button', {
    name: 'Configurações',
    exact: true,
  });
  await settingsButton.click();
  await expect(settingsButton).toBeDisabled();
  await expect(
    page.locator('.notifications').getByRole('progressbar'),
  ).toHaveAttribute('aria-label', 'Carregando configurações…');
  await release(page, 'settings');
  const settings = page.getByRole('dialog', {
    name: 'Configurações',
    exact: true,
  });
  await settings.getByRole('button', { name: 'Executar diagnóstico' }).click();
  await expect(settings.getByRole('progressbar')).toHaveAttribute(
    'aria-label',
    'Verificando ferramentas e autenticação…',
  );
  await expect(
    settings.getByRole('button', { name: 'Salvar configurações' }),
  ).toBeDisabled();
  await expect(settings.getByRole('button', { name: 'Salvando…' })).toHaveCount(
    0,
  );
  await release(page, 'diagnose', true);
  await expect(settings.getByRole('progressbar')).toHaveCount(0);
  await settings
    .getByRole('button', { name: 'Selecionar arquivo', exact: true })
    .click();
  await expect(settings.getByRole('progressbar')).toHaveAttribute(
    'aria-label',
    'Selecionando arquivo de cores…',
  );
  await release(page, 'pick');
  await expect(settings.getByRole('progressbar')).toHaveCount(0);
  await settings
    .getByRole('button', { name: 'Exportar esquema de cores', exact: true })
    .click();
  await expect(settings.getByRole('progressbar')).toHaveAttribute(
    'aria-label',
    'Exportando esquema de cores…',
  );
  await release(page, 'export');
  await expect(settings.getByRole('progressbar')).toHaveCount(0);
  await settings.getByRole('button', { name: 'Salvar configurações' }).click();
  await expect(settings.getByRole('progressbar')).toHaveAttribute(
    'aria-label',
    'Salvando configurações e sincronizando…',
  );
  await expect(
    settings.getByRole('button', { name: 'Executar diagnóstico' }),
  ).toBeDisabled();
  await expect(
    settings.getByRole('button', { name: 'Verificando…' }),
  ).toHaveCount(0);
  await release(page, 'save', true);
  await expect(settings.getByRole('progressbar')).toHaveCount(0);
  await expect(
    settings.getByRole('button', { name: 'Salvar configurações' }),
  ).toBeEnabled();
  await expect(settings.getByRole('alert')).toContainText('Falha simulada');
});

for (const provider of ['github', 'gitlab', 'gitea']) {
  for (const width of [600, 1440]) {
    test(`remote base update on ${provider} at ${width}px`, async ({
      page,
    }) => {
      await setup(page, provider, ['execute']);
      await page.setViewportSize({ width, height: 900 });
      await page.emulateMedia({ reducedMotion: 'reduce' });
      await page.goto('/');
      if (width === 600) await page.locator('.item-row').click();
      const update = page.getByRole('button', {
        name: 'Atualizar com branch base',
        exact: true,
      });
      await expect(update).toBeEnabled();
      await expect(
        page.getByRole('button', {
          name: 'Atualizar branch local',
          exact: true,
        }),
      ).toBeDisabled();
      await update.click();
      const dialog = page.getByRole('dialog', {
        name: 'Atualizar com branch base?',
      });
      await expect(dialog).toContainText('main → feature/tests');
      await expect(dialog).toContainText(
        provider === 'gitlab' ? 'rebase' : 'merge',
      );
      await expect(dialog.getByRole('checkbox')).toHaveCount(0);
      await dialog
        .getByRole('button', { name: 'Cancelar', exact: true })
        .click();
      expect(
        await page.evaluate(() => (window as any).activityState.requests),
      ).toEqual([]);
      await update.click();
      await dialog
        .getByRole('button', { name: 'Confirmar', exact: true })
        .click();
      await expect(update).toBeDisabled();
      await expect(page.locator('.item-row')).toHaveAttribute(
        'aria-busy',
        'true',
      );
      const progress = page.locator('.notifications').getByRole('progressbar');
      await expect(progress).toHaveAttribute(
        'aria-label',
        /Atualizando branch com a base no servidor/,
      );
      await expect(progress).not.toHaveAttribute('aria-valuenow', /./);
      await expect(page.locator('.notifications .spinner')).toHaveCSS(
        'animation-name',
        'none',
      );
      await page.screenshot({
        path: `../../dist/desktop-preview/base-update-${provider}-${width}.png`,
      });
      await release(page, 'execute', true);
      await expect(progress).toHaveCount(0);
      await expect(update).toBeEnabled();
      await expect(page.getByRole('alert')).toContainText('Falha simulada');
      await update.click();
      await dialog
        .getByRole('button', { name: 'Confirmar', exact: true })
        .click();
      await expect(page.locator('.branch-sync .pill')).toHaveCount(0);
      if (provider === 'github') await expect(update).toBeDisabled();
      else await expect(update).toBeEnabled();
      expect(
        await page.evaluate(() =>
          (window as any).activityState.requests.map((r: any) => [
            r.Action,
            r.Push,
            r.Force,
            r.RemoveAfter,
          ]),
        ),
      ).toEqual([
        ['update_branch', false, false, false],
        ['update_branch', false, false, false],
      ]);
    });
  }
}
