import type { main, provider } from '../../wailsjs/go/models';
export type Item = provider.Item & Omit<main.Item, 'Data'>;
export type View = Omit<main.View, 'Items'> & { Items: Item[] };
export function filterItems(
  items: Item[],
  kind: number,
  relation: number,
  query: string,
  instance = '',
): Item[] {
  const q = query.trim().toLocaleLowerCase();
  return items.filter(
    (item) =>
      item.Kind === kind &&
      (!relation || (item.Relations & relation) !== 0) &&
      (!instance || item.Instance === instance) &&
      (!q ||
        `${item.Title} ${item.Repo} ${item.Author} ${item.Ref}`
          .toLocaleLowerCase()
          .includes(q)),
  );
}
export function nextSelection(
  items: Item[],
  selected: string,
  delta: number,
): string {
  if (!items.length) return '';
  const index = items.findIndex((item) => item.Key === selected);
  return items[
    Math.max(
      0,
      Math.min(
        items.length - 1,
        (index < 0 ? (delta > 0 ? -1 : 1) : index) + delta,
      ),
    )
  ].Key;
}
export function relativeAge(value: string, language = 'en'): string {
  const minutes = Math.max(
    0,
    Math.floor((Date.now() - new Date(value).getTime()) / 60000),
  );
  if (!Number.isFinite(minutes)) return '—';
  const format = new Intl.RelativeTimeFormat(language, {
    numeric: 'auto',
    style: 'short',
  });
  if (minutes < 1) return format.format(0, 'second');
  if (minutes < 60) return format.format(-minutes, 'minute');
  if (minutes < 1440) return format.format(-Math.floor(minutes / 60), 'hour');
  return format.format(-Math.floor(minutes / 1440), 'day');
}
