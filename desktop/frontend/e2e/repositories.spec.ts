import { test, expect } from '@playwright/test';

for (const width of [600, 900, 1440]) {
  test(`PR repository groups, navigation and filtering at ${width}px`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 900 });
    await page.goto('/?demo=1');
    await expect(page.locator('.item-row')).toHaveCount(8);
    await expect(page.locator('.repository-heading')).toHaveCount(7);
    const cli = page.getByRole('group', {
      name: 'platform/cli · Corp Git',
      exact: true,
    });
    await expect(cli.locator('.repository-count')).toHaveText('2');
    await expect(cli.locator('.item-row strong')).toHaveText([
      'Melhora cache do pipeline e reduz tempo de build',
      'Adiciona comando para gerenciar worktrees',
    ]);
    const ids = await page
      .locator('.item-row')
      .evaluateAll((rows) => rows.map((row) => row.id));
    for (let i = 1; i < ids.length; i++) {
      await page.keyboard.press('j');
      await expect(page.locator('.item-row.selected')).toHaveAttribute(
        'id',
        ids[i],
      );
    }
    await page.keyboard.press('k');
    await expect(page.locator('.item-row.selected')).toHaveAttribute(
      'id',
      ids.at(-2)!,
    );
    await page.screenshot({
      path: `../../dist/desktop-preview/gui-repositories-${width}.png`,
    });
    expect(
      await page
        .locator('.rows')
        .evaluate((el) => el.scrollWidth <= el.clientWidth),
    ).toBe(true);

    const search = page.getByRole('textbox', { name: 'Buscar itens' });
    await search.fill('platform/cli');
    await expect(page.locator('.repository-heading')).toHaveCount(1);
    await expect(page.locator('.item-row')).toHaveCount(2);
    await search.fill('worktrees');
    await expect(cli.locator('.repository-count')).toHaveText('1');
    await expect(page.locator('.item-row')).toHaveCount(1);
    await search.fill('does-not-exist');
    await expect(page.locator('.repository-heading')).toHaveCount(0);
    await expect(
      page.getByRole('heading', { name: 'Nenhum resultado' }),
    ).toBeVisible();
    await search.fill('');
    await page.getByRole('button', { name: 'Issues', exact: true }).click();
    await expect(page.locator('.item-row')).toHaveCount(1);
    await expect(page.locator('.repository-heading')).toHaveCount(0);
  });
}
