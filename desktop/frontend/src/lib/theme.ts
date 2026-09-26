import type { theme } from '../../wailsjs/go/models';

export type ThemePreference = 'system' | 'light' | 'dark';
export function preference(value: string): ThemePreference {
  return value === 'light' || value === 'dark' ? value : 'system';
}

const fixedColors = [
  'dialog-shadow',
  'backdrop',
  'success-background',
  'approve-background',
  'primary-background',
  'avatar-background',
  'label-background',
  'success-border',
  'approve-border',
  'primary-hover',
  'selected-border',
  'label-border',
  'primary-border',
  'avatar-border',
  'label-alt-border',
  'label-alt-background',
  'danger-background',
  'error-background',
  'error-border',
  'label-alt-text',
  'avatar-text',
  'danger-border',
  'pending-text',
  'approve-text',
  'primary-text',
];
const variables = {
  '--bg': 'Background',
  '--panel': 'Panel',
  '--raised': 'Raised',
  '--line': 'Border',
  '--muted': 'Muted',
  '--text': 'Foreground',
  '--accent': 'Accent',
  '--selected': 'Selection',
  '--selected-text': 'SelectionText',
  '--green': 'Green',
  '--red': 'Red',
  '--yellow': 'Yellow',
  '--blue': 'Blue',
} as const;

// Remove every override before applying a new palette, so manual themes and
// missing Omarchy installations return to the regular CSS/system defaults.
export function applyTheme(
  root: HTMLElement,
  selected: ThemePreference,
  palette?: theme.Palette,
) {
  root.dataset.theme = selected;
  delete root.dataset.systemTheme;
  root.style.removeProperty('color-scheme');
  for (const variable of [
    ...Object.keys(variables),
    '--shadow',
    ...fixedColors.map((key) => '--' + key),
  ])
    root.style.removeProperty(variable);
  if (!palette?.Name || (palette.Name !== 'Custom' && selected !== 'system'))
    return;
  root.dataset.theme = palette.Mode;
  if (!palette.GUI || !Object.keys(palette.GUI).length)
    root.dataset.systemTheme = 'omarchy';
  root.style.setProperty('color-scheme', palette.Mode);
  for (const [variable, field] of Object.entries(variables)) {
    const color = palette[field];
    if (/^#[\da-f]{6}$/i.test(color)) root.style.setProperty(variable, color);
  }
  for (const key of ['shadow', ...fixedColors]) {
    const color = palette.GUI?.[key];
    if (color && /^#[\da-f]{3,8}$/i.test(color))
      root.style.setProperty('--' + key, color);
  }
}

// Capture the displayed palette, including a manual light/dark preference.
export function snapshotTheme(
  root: HTMLElement,
  active?: theme.Palette,
): theme.Palette {
  const computed = getComputedStyle(root);
  const color = (key: string) => {
    const value = computed.getPropertyValue(key).trim();
    return /^#[\da-f]{3}$/i.test(value)
      ? '#' + [...value.slice(1)].map((c) => c + c).join('')
      : value;
  };
  const p = {} as theme.Palette;
  for (const [variable, field] of Object.entries(variables))
    p[field] = color(variable);
  p.Name = 'Custom';
  p.Mode = computed.colorScheme === 'light' ? 'light' : 'dark';
  p.AccentAlt = p.Accent;
  p.Dim = p.Muted;
  p.Heading = p.Foreground;
  p.LabelBackground = p.Selection;
  p.TitleText = p.Background;
  if (root.dataset.systemTheme === 'omarchy' && active) {
    return { ...active, Name: 'Custom' };
  }
  p.GUI = Object.fromEntries(
    ['shadow', ...fixedColors].map((key) => [key, color('--' + key)]),
  );
  return p;
}
