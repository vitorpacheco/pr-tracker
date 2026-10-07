import { test, expect } from '@playwright/test';

for (const provider of ['github', 'gitlab', 'gitea']) {
  for (const width of [600, 1440]) {
    test(`draft and ready confirmation on ${provider} at ${width}px`, async ({
      page,
    }) => {
      await page.addInitScript((provider) => {
        const data = {
          Kind: 0,
          Instance: 'work',
          Provider: provider,
          Host: 'git.company.com',
          Repo: 'team/service',
          Number: 7,
          Title: 'Adicionar testes',
          Draft: true,
          Author: 'ana',
          Relations: 1,
          UpdatedAt: new Date().toISOString(),
          Labels: [],
          Checks: [],
          ApprovedBy: [],
        };
        const state = {
          data,
          requests: [] as any[],
          fail: false,
          pending: false,
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
              Ref: provider === 'gitlab' ? '!7' : '#7',
              Clone: '',
              Worktree: '',
              Data: data,
            },
          ],
        });
        Object.assign(window, {
          draftState: state,
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
                  if (!state.fail) data.Draft = request.Action === 'mark_draft';
                  return {
                    View: view(),
                    Error: state.fail
                      ? 'Sem permissão para alterar o draft'
                      : '',
                    Message: state.fail ? '' : 'Estado atualizado',
                    ReloadThread: false,
                  };
                },
              },
            },
          },
        });
      }, provider);
      await page.setViewportSize({ width, height: 900 });
      await page.goto('/');
      await expect(page.locator('.item-row')).toHaveCount(1);
      if (width === 600) await page.locator('.item-row').click();
      await page.getByRole('button', { name: /^Conversa/ }).click();
      await page.locator('#comment').fill('Rascunho preservado');
      const ready = page.getByRole('button', {
        name: 'Marcar como Ready',
        exact: true,
      });
      await ready.click();
      const confirmation = page.getByRole('dialog', {
        name: 'Marcar como Ready?',
      });
      await expect(confirmation).toContainText('team/service');
      expect(
        await page.evaluate(() => (window as any).draftState.requests),
      ).toEqual([]);
      await confirmation
        .getByRole('button', { name: 'Cancelar', exact: true })
        .click();
      await expect(ready).toBeVisible();
      await ready.click();
      await confirmation
        .getByRole('button', { name: 'Confirmar', exact: true })
        .click();
      const draft = page.getByRole('button', {
        name: 'Marcar como Draft',
        exact: true,
      });
      await expect(draft).toBeVisible();
      await expect(page.locator('.detail-reference .pill')).toHaveText(
        'Aberto',
      );
      await expect(page.locator('.item-row .pill')).toHaveCount(0);
      await expect(page.locator('#comment')).toHaveValue('Rascunho preservado');
      await page.screenshot({
        path: `../../dist/desktop-preview/ready-${provider}-${width}.png`,
      });

      await page.evaluate(() => {
        (window as any).draftState.pending = true;
      });
      await draft.click();
      await page
        .getByRole('dialog', { name: 'Marcar como Draft?' })
        .getByRole('button', { name: 'Confirmar', exact: true })
        .click();
      await expect(draft).toBeDisabled();
      await page.evaluate(() => {
        (window as any).draftState.finish();
        (window as any).draftState.pending = false;
      });
      await expect(ready).toBeVisible();
      await expect(page.locator('.detail-reference .pill')).toHaveText(
        'Rascunho',
      );
      await expect(page.locator('.item-row .pill')).toHaveText('Rascunho');
      expect(
        await page.evaluate(() =>
          (window as any).draftState.requests.map((request: any) => [
            request.Key,
            request.Action,
          ]),
        ),
      ).toEqual([
        ['pr-7', 'mark_ready'],
        ['pr-7', 'mark_draft'],
      ]);
      await page.evaluate(() => {
        (window as any).draftState.fail = true;
      });
      await ready.click();
      await confirmation
        .getByRole('button', { name: 'Confirmar', exact: true })
        .click();
      await expect(page.getByRole('alert')).toContainText('Sem permissão');
      await expect(ready).toBeEnabled();
      await expect(page.locator('#comment')).toHaveValue('Rascunho preservado');
      expect(
        await page.evaluate(
          () => document.documentElement.scrollWidth <= innerWidth,
        ),
      ).toBe(true);
    });
  }
}

test('issues have no draft button', async ({ page }) => {
  await page.goto('/?demo=1');
  await expect(page.locator('.item-row')).toHaveCount(8);
  await page.getByRole('button', { name: 'Issues', exact: true }).click();
  await expect(page.locator('.item-row')).toHaveCount(1);
  await expect(page.locator('.draft-toggle')).toHaveCount(0);
});
