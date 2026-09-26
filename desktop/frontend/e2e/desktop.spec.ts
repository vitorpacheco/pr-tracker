import { test, expect } from '@playwright/test';
for (const width of [600, 900, 1440])
  for (const theme of ['dark', 'light'] as const) {
    test(`adaptive list ${width}px ${theme}`, async ({ page }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.emulateMedia({ colorScheme: theme });
      const errors: string[] = [];
      page.on('pageerror', (e) => errors.push(e.message));
      await page.goto('/?demo=1');
      await expect(page.locator('.item-row')).toHaveCount(8);
      await expect(page.locator('.detail-title')).toHaveText(
        'Adiciona autenticação via token para API interna',
      );
      await expect(page.locator('.inspector')).toBeVisible({
        visible: width >= 700,
      });
      await page.screenshot({
        path: `../../dist/desktop-preview/gui-${width}-${theme}.png`,
        fullPage: true,
      });
      const selected = await page
        .locator('.item-row.selected')
        .getAttribute('id');
      await page.keyboard.press('j');
      await expect(page.locator('.item-row.selected')).not.toHaveAttribute(
        'id',
        selected!,
      );
      if (width < 700) {
        await page.keyboard.press('Enter');
        await expect(page.locator('.inspector')).toBeVisible();
        await page
          .getByRole('button', { name: '← Voltar', exact: true })
          .click();
        await expect(page.locator('.list-pane')).toBeVisible();
      }
      await page.keyboard.press('/');
      await expect(
        page.getByRole('textbox', { name: 'Buscar itens' }),
      ).toBeFocused();
      await page
        .getByRole('textbox', { name: 'Buscar itens' })
        .fill('worktrees');
      await expect(page.locator('.item-row')).toHaveCount(1);
      expect(
        await page.evaluate(
          () => document.documentElement.scrollWidth <= innerWidth,
        ),
      ).toBe(true);
      expect(errors).toEqual([]);
    });
  }
test('palette, issue navigation, detail and local draft', async ({ page }) => {
  await page.goto('/?demo=1');
  await expect(page.locator('.item-row')).toHaveCount(8);
  await page.keyboard.press('Control+k');
  await expect(page.getByRole('dialog', { name: 'Comandos' })).toBeVisible();
  await page.getByRole('textbox', { name: 'Buscar comando' }).fill('conversa');
  await page.getByRole('button', { name: 'Ver conversa' }).click();
  await expect(page.locator('#comment')).toBeVisible();
  await page.locator('#comment').fill('Rascunho local');
  await page.getByRole('button', { name: 'Revisar envio' }).click();
  await expect(
    page.getByRole('dialog', { name: 'Enviar comentário?' }),
  ).toBeVisible();
  await page
    .getByRole('button', { name: 'Cancelar', exact: true })
    .first()
    .click();
  await expect(page.locator('#comment')).toHaveValue('Rascunho local');
  await page.getByRole('button', { name: 'Issues', exact: true }).click();
  await expect(page.locator('.item-row')).toHaveCount(1);
  await page.getByRole('button', { name: 'PRs', exact: true }).click();
  await expect(page.locator('#comment')).toHaveValue('Rascunho local');
});
test('keeps selection on resize and reports unavailable bridge', async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/?demo=1');
  await expect(page.locator('.item-row')).toHaveCount(8);
  await page.locator('.item-row').nth(3).click();
  const id = await page.locator('.item-row.selected').getAttribute('id');
  await page.setViewportSize({ width: 600, height: 900 });
  await expect(page.locator('.detail-title')).toContainText('tabela');
  await page.setViewportSize({ width: 1440, height: 900 });
  await expect(page.locator('.item-row.selected')).toHaveAttribute('id', id!);
  await page.goto('/');
  await expect(page.getByRole('alert')).toContainText(
    'Abra o aplicativo desktop',
  );
});

test('native contract preserves offline data and sanitizes Markdown', async ({
  page,
}) => {
  await page.addInitScript(() => {
    const item = {
      Kind: 0,
      Instance: 'work',
      Provider: 'github',
      Host: 'github.com',
      Repo: 'org/repo',
      Number: 7,
      Title: 'PR do cache',
      Author: 'ana',
      Draft: false,
      SourceBranch: 'feature',
      TargetBranch: 'main',
      Labels: [],
      Assignees: [],
      ApprovedBy: [],
      Checks: [],
      Comments: 0,
      Relations: 1,
      CI: '',
      Review: '',
      Conflicts: false,
      Additions: 0,
      Deletions: 0,
      Files: 0,
      UpdatedAt: new Date().toISOString(),
    };
    const view = {
      Items: [
        {
          Data: item,
          Key: 'work|org/repo|0#7',
          Ref: '#7',
          Clone: '',
          Worktree: '',
        },
      ],
      Errors: {},
      CacheError: '',
      SyncedAt: new Date().toISOString(),
      Revision: 0,
      RefreshSeconds: 300,
      Instances: [{ Name: 'work', Disabled: false }],
    };
    (window as unknown as { go: unknown }).go = {
      main: {
        Bridge: {
          Load: async () => view,
          Refresh: async () => ({ ...view, Errors: { work: 'Sem conexão' } }),
          Detail: async () => ({
            Body: '<img src="x" onerror="document.body.dataset.pwned=1"><script>document.body.dataset.pwned=1</script>Descrição segura',
            Comments: [],
          }),
        },
      },
    };
  });
  await page.goto('/');
  await expect(page.locator('.item-row')).toHaveCount(1);
  await expect(page.locator('.list-pane')).toContainText('Sem conexão');
  await expect(page.locator('.markdown')).toContainText('Descrição segura');
  expect(
    await page.locator('.markdown [onerror],.markdown script').count(),
  ).toBe(0);
  expect(await page.locator('body').getAttribute('data-pwned')).toBeNull();
});

test('dirty cleanup keeps its target after the closed PR leaves the list', async ({
  page,
}) => {
  await page.addInitScript(() => {
    const view = {
      Items: [
        {
          Data: {
            Kind: 0,
            Instance: 'work',
            Repo: 'org/repo',
            Number: 7,
            Title: 'PR para fechar',
            Author: 'ana',
            Labels: [],
            Checks: [],
            ApprovedBy: [],
            Comments: 0,
            Relations: 1,
            UpdatedAt: new Date().toISOString(),
          },
          Key: 'work|org/repo|0#7',
          Ref: '#7',
          Clone: '/clone',
          Worktree: '/worktree',
        },
      ],
      Errors: {},
      CacheError: '',
      SyncedAt: new Date().toISOString(),
      Revision: 0,
      RefreshSeconds: 300,
      Instances: [],
    };
    const requests: unknown[] = [];
    (window as unknown as { requests: unknown[] }).requests = requests;
    (window as unknown as { go: unknown }).go = {
      main: {
        Bridge: {
          Load: async () => view,
          Refresh: async () => view,
          Detail: async () => ({ Body: 'PR', Comments: [] }),
          Execute: async (request: { Action: string; Force: boolean }) => {
            requests.push(request);
            return {
              View: { ...view, Items: [] },
              Message: request.Force ? 'worktree removido' : 'PR fechado',
              Error: request.Force ? '' : 'worktree sujo',
              Dirty: !request.Force,
              NeedsClone: false,
              ReloadThread: false,
            };
          },
        },
      },
    };
  });
  await page.goto('/');
  await expect(page.locator('.item-row')).toHaveCount(1);
  await page.getByText('Mais ações', { exact: true }).click();
  await page
    .getByRole('button', { name: 'Fechar e remover worktree', exact: true })
    .click();
  await page.getByRole('button', { name: 'Confirmar', exact: true }).click();
  const confirm = page.getByRole('dialog', {
    name: 'Descartar alterações locais e remover worktree?',
  });
  await expect(confirm).toContainText('org/repo #7');
  await expect(confirm).toContainText('/worktree');
  await confirm.getByRole('button', { name: 'Confirmar', exact: true }).click();
  await expect(confirm).not.toBeVisible();
  expect(
    await page.evaluate(
      () => (window as unknown as { requests: unknown[] }).requests,
    ),
  ).toMatchObject([
    { Action: 'close', Force: false, RemoveAfter: true },
    { Action: 'remove_worktree', Force: true, Key: 'work|org/repo|0#7' },
  ]);
});

test('sidebar toggle preserves selection, draft and saved layout', async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/?demo=1');
  await expect(page.locator('.item-row')).toHaveCount(8);
  await page.locator('.item-row').nth(2).click();
  const selected = await page.locator('.item-row.selected').getAttribute('id');
  await page.getByRole('button', { name: /^Conversa/ }).click();
  await page.locator('#comment').fill('Rascunho durante ajuste do layout');
  const width = (await page.locator('.list-pane').boundingBox())!.width;
  await page
    .getByRole('button', { name: 'Recolher menu lateral', exact: true })
    .click();
  await expect(
    page.getByRole('button', { name: 'Abrir menu lateral', exact: true }),
  ).toHaveAttribute('aria-expanded', 'false');
  await expect(page.locator('.sidebar')).toBeHidden();
  await expect(page.getByRole('separator')).toHaveCount(1);
  expect(
    (await page.locator('.list-pane').boundingBox())!.width,
  ).toBeGreaterThan(width);
  await expect(page.locator('.item-row.selected')).toHaveAttribute(
    'id',
    selected!,
  );
  await expect(page.locator('#comment')).toHaveValue(
    'Rascunho durante ajuste do layout',
  );
  await page.screenshot({
    path: '../../dist/desktop-preview/gui-sidebar-collapsed.png',
  });
  await page.reload();
  await expect(page.locator('.sidebar')).toBeHidden();
  await page
    .getByRole('button', { name: 'Abrir menu lateral', exact: true })
    .click();
  await expect(page.locator('.sidebar')).toBeVisible();
  await page.reload();
  await expect(page.locator('.sidebar')).toBeVisible();
});

test('panel dividers resize by pointer and keyboard, persist and respect available space', async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/?demo=1');
  await expect(page.locator('.item-row')).toHaveCount(8);
  const menu = page.getByRole('separator', { name: 'Largura do menu lateral' });
  const details = page.getByRole('separator', { name: 'Largura dos detalhes' });
  for (const [handle, offset, expected] of [
    [menu, 60, 260],
    [details, -130, 500],
  ] as const) {
    const box = (await handle.boundingBox())!;
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
    await page.mouse.down();
    await page.mouse.move(
      box.x + box.width / 2 + offset,
      box.y + box.height / 2,
      { steps: 8 },
    );
    await page.mouse.up();
    await expect(handle).toHaveAttribute('aria-valuenow', String(expected));
  }
  expect((await page.locator('.sidebar').boundingBox())!.width).toBe(260);
  expect((await page.locator('.inspector').boundingBox())!.width).toBe(500);
  await page.screenshot({
    path: '../../dist/desktop-preview/gui-panels-resized.png',
  });
  await page.reload();
  await expect(menu).toHaveAttribute('aria-valuenow', '260');
  await expect(details).toHaveAttribute('aria-valuenow', '500');
  await details.focus();
  await page.keyboard.press('Shift+ArrowLeft');
  await expect(details).toHaveAttribute('aria-valuenow', '540');
  await page.keyboard.press('Home');
  await expect(details).toHaveAttribute('aria-valuenow', '300');
  await page.keyboard.press('End');
  await expect(details).toHaveAttribute('aria-valuenow', '848');
  expect((await page.locator('.list-pane').boundingBox())!.width).toBe(320);
  // List content also adapts when the window itself remains wide.
  await expect(page.locator('.item-row .author').first()).toBeHidden();
  expect(
    await page
      .locator('.rows')
      .evaluate((el) => el.scrollWidth <= el.clientWidth),
  ).toBe(true);
  const box = (await menu.boundingBox())!;
  await page.mouse.move(box.x + 3, box.y + 100);
  await page.mouse.down();
  await page.mouse.move(1400, box.y + 100, { steps: 8 });
  await page.mouse.up();
  await expect(menu).toHaveAttribute('aria-valuenow', '360');
  expect((await page.locator('.list-pane').boundingBox())!.width).toBe(320);
  await page.setViewportSize({ width: 700, height: 900 });
  await expect(menu).toHaveCount(0);
  await expect(details).toHaveAttribute('aria-valuenow', '374');
  expect((await page.locator('.list-pane').boundingBox())!.width).toBe(320);
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  await page.setViewportSize({ width: 600, height: 900 });
  await expect(details).toHaveCount(0);
  await page.setViewportSize({ width: 1440, height: 900 });
  await expect(menu).toHaveAttribute('aria-valuenow', '360');
  await expect(details).toHaveAttribute('aria-valuenow', '748');
});

for (const width of [600, 900]) {
  test(`compact sidebar opens and closes without displacing panels at ${width}px`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 900 });
    await page.goto('/?demo=1');
    await expect(page.locator('.item-row')).toHaveCount(8);
    const initial = await page.locator('.list-pane').boundingBox();
    const toggle = page.getByRole('button', {
      name: 'Abrir menu lateral',
      exact: true,
    });
    await toggle.focus();
    await page.keyboard.press('Enter');
    await expect(page.locator('.sidebar')).toBeVisible();
    await page
      .locator('.sidebar')
      .getByRole('button', { name: 'Revisar' })
      .click();
    await expect(page.locator('.list-pane')).toHaveCSS('display', 'flex');
    expect(await page.locator('.list-pane').boundingBox()).toEqual(initial);
    await page.keyboard.press('Escape');
    await expect(page.locator('.sidebar')).toBeHidden();
    await expect(toggle).toBeFocused();
    await expect(page.locator('.list-pane')).toBeVisible();
    await toggle.click();
    await page
      .getByRole('button', { name: 'Fechar menu lateral', exact: true })
      .click({ position: { x: width - 10, y: 100 } });
    await expect(page.locator('.sidebar')).toBeHidden();
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
  });
}
