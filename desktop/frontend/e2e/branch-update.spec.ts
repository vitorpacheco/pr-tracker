import { test, expect } from '@playwright/test';

for (const width of [600, 1440]) {
  test(`update local branch at ${width}px`, async ({ page }) => {
    await page.addInitScript(() => {
      const state = {
        requests: [] as any[],
        pending: false,
        dirty: false,
        pushFailure: false,
      };
      const view = () => ({
        Language: 'pt',
        Revision: 0,
        RefreshSeconds: 300,
        Instances: [],
        Errors: {},
        Items: [
          {
            Key: 'pr-7',
            Ref: '!7',
            Clone: '/clone',
            Worktree: '/worktree',
            Data: {
              Kind: 0,
              Instance: 'work',
              Provider: 'gitlab',
              Host: 'git.company.com',
              Repo: 'team/service',
              Number: 7,
              Title: 'Adicionar testes',
              Author: 'ana',
              SourceBranch: 'feature/tests',
              TargetBranch: 'main',
              Relations: 1,
              UpdatedAt: new Date().toISOString(),
              Labels: [],
              Checks: [],
              ApprovedBy: [],
            },
          },
        ],
      });
      Object.assign(window, {
        branchState: state,
        go: {
          main: {
            Bridge: {
              Load: async () => view(),
              Refresh: async () => view(),
              Theme: async () => ({}),
              Detail: async () => ({ Body: 'Descrição', Comments: [] }),
              Execute: async (request: any) => {
                state.requests.push(request);
                if (state.pending)
                  await new Promise<void>((resolve) => {
                    (state as any).finish = resolve;
                  });
                return {
                  View: view(),
                  ReloadThread: false,
                  Dirty: state.dirty,
                  Error: state.dirty
                    ? 'worktree tem alterações locais'
                    : state.pushFailure
                      ? 'branch local atualizada, mas o push falhou: sem permissão'
                      : '',
                  Message: state.dirty ? '' : 'worktree pronto: /worktree',
                };
              },
            },
          },
        },
      });
    });
    await page.setViewportSize({ width, height: 900 });
    await page.goto('/');
    await expect(page.locator('.item-row')).toHaveCount(1);
    if (width === 600) await page.locator('.item-row').click();
    const update = page.getByRole('button', {
      name: 'Atualizar branch local',
      exact: true,
    });
    await expect(update).toBeEnabled();
    await page.getByRole('button', { name: /^Conversa/ }).click();
    await page.locator('#comment').fill('Rascunho preservado');
    await update.click();
    const dialog = page.getByRole('dialog', {
      name: 'Atualizar branch local?',
    });
    await expect(dialog).toContainText('team/service !7');
    await expect(dialog).toContainText('branch de origem');
    await expect(dialog).toContainText('worktree local');
    const push = dialog.getByRole('checkbox', {
      name: 'Fazer push após atualizar a branch',
    });
    await expect(push).not.toBeChecked();
    await push.check();
    await dialog.getByRole('button', { name: 'Cancelar', exact: true }).click();
    expect(
      await page.evaluate(() => (window as any).branchState.requests),
    ).toEqual([]);
    await page.evaluate(() => {
      (window as any).branchState.pending = true;
    });
    await update.click();
    await expect(push).not.toBeChecked();
    await dialog
      .getByRole('button', { name: 'Confirmar', exact: true })
      .click();
    await expect(update).toBeDisabled();
    await page.evaluate(() => {
      (window as any).branchState.pending = false;
      (window as any).branchState.finish();
    });
    await expect(update).toBeEnabled();
    await expect(page.locator('#comment')).toHaveValue('Rascunho preservado');
    expect(
      await page.evaluate(() =>
        (window as any).branchState.requests.map((r: any) => [
          r.Key,
          r.Action,
          r.Force,
          r.RemoveAfter,
          r.Push,
        ]),
      ),
    ).toEqual([['pr-7', 'update_worktree', false, false, false]]);
    await update.click();
    await push.check();
    await page.screenshot({
      path: `../../dist/desktop-preview/branch-push-${width}.png`,
    });
    await dialog
      .getByRole('button', { name: 'Confirmar', exact: true })
      .click();
    await expect(update).toBeEnabled();
    expect(
      await page.evaluate(() => (window as any).branchState.requests[1].Push),
    ).toBe(true);
    await page.evaluate(() => {
      (window as any).branchState.pushFailure = true;
    });
    await update.click();
    await expect(push).not.toBeChecked();
    await push.check();
    await dialog
      .getByRole('button', { name: 'Confirmar', exact: true })
      .click();
    await expect(page.getByRole('alert')).toContainText(
      'branch local atualizada, mas o push falhou',
    );
    await expect(page.locator('#comment')).toHaveValue('Rascunho preservado');
    await page.screenshot({
      path: `../../dist/desktop-preview/branch-update-${width}.png`,
    });
    await page.evaluate(() => {
      (window as any).branchState.dirty = true;
    });
    await update.click();
    await dialog
      .getByRole('button', { name: 'Confirmar', exact: true })
      .click();
    await expect(page.getByRole('alert')).toContainText(
      'worktree tem alterações locais',
    );
    await expect(page.getByRole('dialog')).toHaveCount(0);
    await expect(page.locator('#comment')).toHaveValue('Rascunho preservado');
    await expect(update).toBeEnabled();
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
  });
}

test('branch update requires a worktree and is absent on issues', async ({
  page,
}) => {
  await page.goto('/?demo=1');
  await expect(page.locator('.item-row')).toHaveCount(8);
  await page
    .locator('.item-row')
    .filter({ hasText: 'Refatora fluxo de renovação' })
    .click();
  await expect(
    page.getByRole('button', { name: 'Atualizar branch local', exact: true }),
  ).toBeDisabled();
  await page.getByRole('button', { name: 'Issues', exact: true }).click();
  await expect(page.locator('.item-row')).toHaveCount(1);
  await expect(page.locator('.branch-update')).toHaveCount(0);
});
