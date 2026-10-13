import { test, expect } from '@playwright/test';

const item = (number: number, conflicts: boolean) => ({
  Key: `pr-${number}`,
  Ref: `!${number}`,
  Clone: '',
  Worktree: '',
  Data: {
    Kind: 0,
    Instance: 'work',
    Provider: 'gitlab',
    Host: 'git.company.com',
    Repo: 'team/service',
    Number: number,
    Title: conflicts ? 'Refatorar autenticação' : 'Adicionar testes',
    Author: 'ana',
    SourceBranch: 'feature/auth',
    TargetBranch: 'main',
    Conflicts: conflicts,
    Relations: 1,
    UpdatedAt: new Date().toISOString(),
    Labels: [],
    Checks: [],
    ApprovedBy: [],
  },
});

async function bridge(page: any, items: any[], language: string) {
  await page.addInitScript(
    ([list, lang]: [any[], string]) => {
      const view = () => ({
        Language: lang,
        Revision: 0,
        RefreshSeconds: 300,
        Instances: [],
        Errors: {},
        Items: list,
      });
      Object.assign(window, {
        go: {
          main: {
            Bridge: {
              Load: async () => view(),
              Refresh: async () => view(),
              Theme: async () => ({}),
              Detail: async () => ({ Body: 'Descrição', Comments: [] }),
              Execute: async () => ({ View: view(), ReloadThread: false }),
            },
          },
        },
      });
    },
    [items, language],
  );
}

for (const width of [600, 1440]) {
  test(`conflict indicator at ${width}px`, async ({ page }) => {
    await bridge(page, [item(7, false), item(8, true)], 'pt');
    await page.setViewportSize({ width, height: 900 });
    await page.goto('/');

    const rows = page.locator('.item-row');
    await expect(rows).toHaveCount(2);
    await expect(rows.nth(0).locator('.pill.red')).toHaveCount(0);
    const badge = rows.nth(1).locator('.pill.red');
    await expect(badge).toHaveText('Conflitos');
    await expect(badge).toHaveAttribute(
      'title',
      'Conflitos precisam ser resolvidos antes do merge.',
    );

    await rows.nth(1).click();
    await expect(page.locator('.branch-sync .pill.red')).toHaveText(
      'Conflitos',
    );
    await expect(
      page.getByRole('button', { name: 'Atualizar com branch base' }),
    ).toBeDisabled();
    await page.screenshot({
      path: `../../dist/desktop-preview/conflicts-${width}.png`,
    });
  });
}

test('conflict indicator is translated', async ({ page }) => {
  await bridge(page, [item(8, true)], 'en');
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/');
  await expect(page.locator('.item-row .pill.red')).toHaveText('Conflicts');
  await expect(page.locator('.branch-sync .pill.red')).toHaveText('Conflicts');
});
