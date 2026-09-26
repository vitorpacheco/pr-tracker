import { test, expect } from '@playwright/test';

test('system follows Omarchy changes, manual theme wins, and missing palette restores defaults', async ({
  page,
}) => {
  await page.addInitScript(() => {
    const view = {
      Language: 'en',
      Items: [],
      Instances: [],
      Errors: {},
      RefreshSeconds: 300,
      Revision: 0,
    };
    const palette = {
      Name: 'Omarchy',
      Mode: 'dark',
      Background: '#121212',
      Panel: '#0d0d0d',
      Raised: '#1e1e1e',
      Border: '#333333',
      Muted: '#8a8a8d',
      Foreground: '#bebebe',
      Accent: '#e68e0d',
      Selection: '#2a2a2a',
      SelectionText: '#bebebe',
      Red: '#d35f5f',
      Green: '#ffc107',
      Yellow: '#b91c1c',
      Blue: '#e68e0d',
    };
    Object.assign(window, {
      smokePalette: palette,
      go: {
        main: {
          Bridge: {
            Load: async () => view,
            Refresh: async () => view,
            Theme: async () =>
              (window as unknown as { smokePalette: typeof palette })
                .smokePalette,
            Settings: async () => ({
              Language: 'en',
              ToolPaths: {},
              Instances: [],
              Repos: [],
            }),
          },
        },
      },
    });
  });
  await page.goto('/');
  await expect(page.locator('html')).toHaveAttribute(
    'data-system-theme',
    'omarchy',
  );
  await expect(page.locator('html')).toHaveCSS(
    'background-color',
    'rgb(18, 18, 18)',
  );
  await page.screenshot({
    animations: 'disabled',
    path: '../../dist/desktop-preview/omarchy-dark.png',
  });
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  await page
    .getByRole('combobox', { name: 'Theme', exact: true })
    .selectOption('light');
  await expect(page.locator('html')).not.toHaveAttribute(
    'data-system-theme',
    'omarchy',
  );
  await expect(page.locator('html')).toHaveCSS(
    'background-color',
    'rgb(246, 248, 252)',
  );
  await page.evaluate(() => {
    Object.assign(
      (window as unknown as { smokePalette: object }).smokePalette,
      {
        Mode: 'light',
        Background: '#faf4ed',
        Foreground: '#575279',
        Accent: '#56949f',
        Panel: '#ede7e1',
        Border: '#cecacd',
        Muted: '#6e6a86',
        Red: '#b4637a',
        Green: '#286983',
        Yellow: '#ea9d34',
        Blue: '#56949f',
        Raised: '#f2e9e1',
        Selection: '#dfdad9',
        SelectionText: '#575279',
      },
    );
  });
  await page.waitForTimeout(2200);
  await expect(page.locator('html')).toHaveCSS(
    'background-color',
    'rgb(246, 248, 252)',
  );
  await page
    .getByRole('combobox', { name: 'Theme', exact: true })
    .selectOption('system');
  await expect(page.locator('html')).toHaveCSS(
    'background-color',
    'rgb(250, 244, 237)',
  );
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'light');
  await page.keyboard.press('Escape');
  await page.screenshot({
    animations: 'disabled',
    path: '../../dist/desktop-preview/omarchy-light.png',
  });
  await page.evaluate(() => Object.assign(window, { smokePalette: {} }));
  await expect(page.locator('html')).not.toHaveAttribute(
    'data-system-theme',
    'omarchy',
  );
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'system');
  expect(
    await page
      .locator('html')
      .evaluate((el) => el.style.getPropertyValue('--accent')),
  ).toBe('');
});
