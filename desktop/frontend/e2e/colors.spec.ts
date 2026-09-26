import { test, expect } from '@playwright/test';

test('exports the displayed default and imports a custom palette over manual light mode', async ({
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
    const state = { palette: {}, exported: {} };
    Object.assign(window, {
      colorState: state,
      go: {
        main: {
          Bridge: {
            Load: async () => view,
            Refresh: async () => view,
            Theme: async () => state.palette,
            Settings: async () => ({
              Language: 'en',
              ToolPaths: {},
              Instances: [],
              Repos: [],
              ThemeFile: '',
            }),
            PickTheme: async () => '/tmp/colors.toml',
            ExportTheme: async (palette: object) => {
              state.exported = palette;
              return '/tmp/export.toml';
            },
            SaveSettings: async (cfg: { ThemeFile: string }) => {
              if (cfg.ThemeFile !== '/tmp/colors.toml')
                throw new Error('unexpected path');
              state.palette = {
                ...state.exported,
                Name: 'Custom',
                Background: '#123456',
              };
              return view;
            },
          },
        },
      },
    });
  });
  await page.goto('/');
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  await page
    .getByRole('combobox', { name: 'Theme', exact: true })
    .selectOption('light');
  await page
    .getByRole('button', { name: 'Export color scheme', exact: true })
    .click();
  await expect
    .poll(() =>
      page.evaluate(() => (window as any).colorState.exported.Background),
    )
    .toBe('#f6f8fc');
  expect(
    await page.evaluate(() => (window as any).colorState.exported.Panel),
  ).toBe('#ffffff');
  expect(
    await page.evaluate(
      () => (window as any).colorState.exported.SelectionText,
    ),
  ).toBe('#172b43');
  await page.screenshot({
    path: '../../dist/desktop-preview/color-settings.png',
    animations: 'disabled',
  });
  await page.getByRole('button', { name: 'Choose file', exact: true }).click();
  await expect(
    page.getByRole('textbox', { name: 'Color file', exact: true }),
  ).toHaveValue('/tmp/colors.toml');
  await page
    .getByRole('button', { name: 'Save settings', exact: true })
    .click();
  await expect(page.locator('html')).toHaveCSS(
    'background-color',
    'rgb(18, 52, 86)',
  );
});
