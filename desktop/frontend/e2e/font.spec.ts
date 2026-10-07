import { test, expect, type Locator } from '@playwright/test';

async function fontSize(element: Locator) {
  return element.evaluate((el) => parseFloat(getComputedStyle(el).fontSize));
}

for (const modifier of ['Control', 'Meta']) {
  test(`${modifier} font shortcuts resize text, persist and work in dialogs`, async ({
    page,
  }) => {
    await page.setViewportSize({ width: 1440, height: 900 });
    await page.goto('/?demo=1');
    await expect(page.locator('.item-row')).toHaveCount(8);
    const title = page.locator('.detail-title');
    const search = page.getByRole('textbox', { name: 'Buscar itens' });
    const titleSize = await fontSize(title);
    const searchSize = await fontSize(search);
    const selected = await page
      .locator('.item-row.selected')
      .getAttribute('id');
    const listWidth = (await page.locator('.list-pane').boundingBox())!.width;

    await search.focus();
    await page.keyboard.press(`${modifier}+-`);
    await expect.poll(() => fontSize(title)).toBeCloseTo(titleSize * 0.9, 2);
    expect(await fontSize(search)).toBeCloseTo(searchSize * 0.9, 2);
    await expect(search).toBeFocused();
    await expect(search).toHaveValue('');
    await expect(page.locator('.item-row.selected')).toHaveAttribute(
      'id',
      selected!,
    );
    expect((await page.locator('.list-pane').boundingBox())!.width).toBe(
      listWidth,
    );
    await page.screenshot({
      path: `../../dist/desktop-preview/gui-font-${modifier.toLowerCase()}.png`,
    });

    await page.reload();
    await expect.poll(() => fontSize(title)).toBeCloseTo(titleSize * 0.9, 2);
    await page.keyboard.press(`${modifier}+=`);
    await expect.poll(() => fontSize(title)).toBeCloseTo(titleSize, 2);
    await page.keyboard.press(`${modifier}+Shift+=`);
    await expect.poll(() => fontSize(title)).toBeCloseTo(titleSize * 1.1, 2);
    await page.keyboard.press(`${modifier}+0`);
    await expect.poll(() => fontSize(title)).toBeCloseTo(titleSize, 2);

    await page
      .getByRole('button', { name: 'Configurações', exact: true })
      .click();
    const dialog = page.getByRole('dialog', { name: 'Configurações' });
    await expect(dialog).toBeVisible();
    const heading = dialog.locator('h2');
    const headingSize = await fontSize(heading);
    await page.keyboard.press(`${modifier}+-`);
    await expect
      .poll(() => fontSize(heading))
      .toBeCloseTo(headingSize * 0.9, 2);
    await expect(dialog).toBeVisible();
    await page.keyboard.press(`${modifier}+0`);
    await expect.poll(() => fontSize(heading)).toBeCloseTo(headingSize, 2);
  });
}

test('font shortcuts respect limits and ignore unrelated key combinations', async ({
  page,
}) => {
  await page.goto('/?demo=1');
  await expect(page.locator('.item-row')).toHaveCount(8);
  const title = page.locator('.detail-title');
  const size = await fontSize(title);
  for (const key of ['-', '=', 'Control+Alt+-']) await page.keyboard.press(key);
  expect(await fontSize(title)).toBe(size);
  for (let i = 0; i < 20; i++) await page.keyboard.press('Control+-');
  await expect.poll(() => fontSize(title)).toBeCloseTo(size * 0.5, 2);
  for (let i = 0; i < 25; i++) await page.keyboard.press('Control+=');
  await expect.poll(() => fontSize(title)).toBeCloseTo(size * 2, 2);
  await page.keyboard.press('Control+0');
  await expect.poll(() => fontSize(title)).toBeCloseTo(size, 2);
});

test('invalid or unavailable storage preserves working font shortcuts', async ({
  page,
}) => {
  await page.addInitScript(() => {
    localStorage.setItem('pr-tracker-font-scale', '"invalid"');
    Storage.prototype.setItem = () => {
      throw new Error('Storage disabled');
    };
  });
  await page.goto('/?demo=1');
  await expect(page.locator('.item-row')).toHaveCount(8);
  const title = page.locator('.detail-title');
  const size = await fontSize(title);
  await page.keyboard.press('Control+-');
  await expect.poll(() => fontSize(title)).toBeCloseTo(size * 0.9, 2);
});
