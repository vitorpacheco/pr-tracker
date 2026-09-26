import { describe, it, expect } from 'vitest';
import { filterItems, nextSelection, type Item } from './list';
const rows = [
  {
    Key: 'pr',
    Kind: 0,
    Relations: 3,
    Instance: 'a',
    Repo: 'team/service',
    Title: 'Add token',
    Author: 'ana',
    Ref: '#1',
  },
  {
    Key: 'issue',
    Kind: 1,
    Relations: 4,
    Instance: 'a',
    Repo: 'team/service',
    Title: 'Follow up',
    Author: 'bob',
    Ref: '#2',
  },
  {
    Key: 'other',
    Kind: 0,
    Relations: 0,
    Instance: 'b',
    Repo: 'team/tools',
    Title: 'Fix build',
    Author: 'bob',
    Ref: '#3',
  },
] as Item[];
describe('list navigation and filtering', () => {
  it('combines kind, bitmask, instance and case-insensitive search', () => {
    expect(filterItems(rows, 0, 1, ' TOKEN ', 'a').map((p) => p.Key)).toEqual([
      'pr',
    ]);
    expect(filterItems(rows, 0, 2, 'ANA').map((p) => p.Key)).toEqual(['pr']);
    expect(filterItems(rows, 1, 4, '#2').map((p) => p.Key)).toEqual(['issue']);
    expect(filterItems(rows, 0, 0, '').map((p) => p.Key)).toEqual([
      'pr',
      'other',
    ]);
  });
  it('keeps cursor at bounds and handles empty or replaced selections', () => {
    expect(nextSelection(rows, 'pr', -1)).toBe('pr');
    expect(nextSelection(rows, 'other', 1)).toBe('other');
    expect(nextSelection(rows, 'missing', 1)).toBe('pr');
    expect(nextSelection([], 'pr', 1)).toBe('');
  });
});
