import { test, expect } from '@playwright/test';

for (const locale of ['en-US', 'pt-BR', 'de-DE']) {
  test(`language follows ${locale} and can be overridden`, async ({
    browser,
  }) => {
    const context = await browser.newContext({
      locale,
      viewport: { width: 1440, height: 950 },
    });
    const page = await context.newPage();
    const errors: string[] = [];
    page.on('pageerror', (error) => errors.push(error.message));
    await page.goto('/?demo=1');
    const portuguese = locale === 'pt-BR';
    await expect(page.locator('html')).toHaveAttribute(
      'lang',
      portuguese ? 'pt' : 'en',
    );
    await page
      .getByRole('button', {
        name: portuguese ? 'Configurações' : 'Settings',
        exact: true,
      })
      .click();
    await expect(
      page.getByRole('combobox', {
        name: portuguese ? 'Idioma' : 'Language',
        exact: true,
      }),
    ).toHaveValue('system');
    await page
      .getByRole('combobox', {
        name: portuguese ? 'Idioma' : 'Language',
        exact: true,
      })
      .selectOption(portuguese ? 'en' : 'pt');
    await page
      .getByRole('button', {
        name: portuguese ? 'Salvar configurações' : 'Save settings',
        exact: true,
      })
      .click();
    await expect(page.locator('html')).toHaveAttribute(
      'lang',
      portuguese ? 'en' : 'pt',
    );
    await expect(page.locator('.detail-title')).toHaveText(
      'Adiciona autenticação via token para API interna',
    );
    await page.reload();
    await expect(page.locator('html')).toHaveAttribute(
      'lang',
      portuguese ? 'en' : 'pt',
    );
    await page
      .getByRole('button', {
        name: portuguese ? 'Settings' : 'Configurações',
        exact: true,
      })
      .click();
    await expect(
      page.getByRole('combobox', {
        name: portuguese ? 'Language' : 'Idioma',
        exact: true,
      }),
    ).toHaveValue(portuguese ? 'en' : 'pt');
    await page.screenshot({
      path: `../../dist/desktop-preview/settings-${locale}.png`,
      fullPage: true,
    });
    // Cancel does not apply an unsaved language preference.
    await page
      .getByRole('combobox', {
        name: portuguese ? 'Language' : 'Idioma',
        exact: true,
      })
      .selectOption('system');
    await page
      .getByRole('button', {
        name: portuguese ? 'Cancel' : 'Cancelar',
        exact: true,
      })
      .click();
    await expect(page.locator('html')).toHaveAttribute(
      'lang',
      portuguese ? 'en' : 'pt',
    );
    await page
      .getByRole('button', {
        name: portuguese ? 'Settings' : 'Configurações',
        exact: true,
      })
      .click();
    await page
      .getByRole('combobox', {
        name: portuguese ? 'Language' : 'Idioma',
        exact: true,
      })
      .selectOption('system');
    await page
      .getByRole('button', {
        name: portuguese ? 'Save settings' : 'Salvar configurações',
        exact: true,
      })
      .click();
    await expect(page.locator('html')).toHaveAttribute(
      'lang',
      portuguese ? 'pt' : 'en',
    );
    expect(errors).toEqual([]);
    await context.close();
  });
}
