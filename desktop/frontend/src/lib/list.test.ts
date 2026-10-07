import { describe, it, expect } from 'vitest';
import {
  filterItems,
  groupByRepository,
  nextSelection,
  type Item,
} from './list';
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
  it('groups repositories in recency order and preserves row order within groups', () => {
    const items = [
      { ...rows[0], Key: 'new', Repo: 'team/service' },
      { ...rows[0], Key: 'middle', Repo: 'team/tools' },
      { ...rows[0], Key: 'older', Repo: 'team/service' },
    ];
    const groups = groupByRepository(items);
    expect(groups.map((group) => group.repo)).toEqual([
      'team/service',
      'team/tools',
    ]);
    expect(groups.map((group) => group.items.map((item) => item.Key))).toEqual([
      ['new', 'older'],
      ['middle'],
    ]);
    expect(items.map((item) => item.Key)).toEqual(['new', 'middle', 'older']);
    expect(groupByRepository([])).toEqual([]);
  });
  it('keeps identically named repositories from different instances and hosts separate', () => {
    const items = [
      { ...rows[0], Host: 'github.com' },
      { ...rows[0], Instance: 'b', Host: 'github.com' },
      { ...rows[0], Host: 'git.example.com' },
    ];
    const groups = groupByRepository(items);
    expect(groups).toHaveLength(3);
    expect(new Set(groups.map((group) => group.key)).size).toBe(3);
  });
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
