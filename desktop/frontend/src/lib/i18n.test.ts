import { describe, it, expect } from 'vitest';
import { matchLanguage, translator, statusLabel } from './i18n';
import { relativeAge } from './list';

describe('interface language', () => {
  it.each([
    ['pt-BR', 'pt'],
    ['pt_PT.UTF-8', 'pt'],
    ['en-US', 'en'],
    ['de-DE', 'en'],
    ['C', 'en'],
    ['', 'en'],
  ])('resolves %s to %s', (locale, expected) => {
    expect(matchLanguage(locale)).toBe(expected);
  });
  it('translates interface labels and preserves unknown content', () => {
    expect(translator('en')('Configurações')).toBe('Settings');
    expect(translator('pt')('Configurações')).toBe('Configurações');
    expect(translator('en')('custom title')).toBe('custom title');
    expect(statusLabel('pt', 'changes_requested')).toBe(
      'alterações solicitadas',
    );
    expect(statusLabel('en', 'changes_requested')).toBe('changes requested');
  });
  it('formats relative dates in the chosen language', () => {
    const date = new Date(Date.now() - 5 * 60000).toISOString();
    expect(relativeAge(date, 'en')).toContain('ago');
    expect(relativeAge(date, 'pt')).toContain('há');
    expect(relativeAge('invalid', 'en')).toBe('—');
  });
});
