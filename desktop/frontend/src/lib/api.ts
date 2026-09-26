import { translator, browserLanguage } from './i18n';
import * as native from '../../wailsjs/go/main/Bridge';
import type { main, config, provider, app } from '../../wailsjs/go/models';
import type { View } from './list';
function flatten(view: main.View): View {
  return {
    ...view,
    Items: (view.Items ?? []).map((item) => ({
      ...item.Data,
      Key: item.Key,
      Ref: item.Ref,
      Clone: item.Clone,
      Worktree: item.Worktree,
    })),
  };
}
async function readView(
  name: 'Load' | 'Refresh' | 'SaveSettings',
  ...args: unknown[]
): Promise<View> {
  const view = await call<main.View>(name, ...args);
  return demoMode ? (view as unknown as View) : flatten(view);
}
export const demoMode =
  new URLSearchParams(location.search).get('demo') === '1';
let demo: typeof import('./demo');
async function call<T>(
  name: keyof typeof native,
  ...args: unknown[]
): Promise<T> {
  if (demoMode) {
    demo ??= await import('./demo');
    return demo.invoke(name, args) as Promise<T>;
  }
  if (!('go' in window))
    throw new Error(
      translator(browserLanguage())(
        'Abra o aplicativo desktop. Para explorar dados fictícios no navegador, use ?demo=1.',
      ),
    );
  return (native[name] as (...args: unknown[]) => Promise<T>)(...args);
}
export const api = {
  load: () => readView('Load'),
  refresh: () => readView('Refresh'),
  detail: (key: string) => call<provider.Thread>('Detail', key),
  execute: async (request: main.Request) => {
    const result = await call<main.Result>('Execute', request);
    return { ...result, View: flatten(result.View) };
  },
  settings: () => call<config.Config>('Settings'),
  saveSettings: (cfg: config.Config) => readView('SaveSettings', cfg),
  diagnose: () => call<app.Diagnostics>('Diagnose'),
  pickFolder: () => call<string>('PickFolder'),
  openFolder: (key: string) => call<void>('OpenFolder', key),
  mergeOptions: (key: string) =>
    call<provider.MergeOptions>('MergeOptions', key),
  openItem: (key: string) => call<void>('OpenItem', key),
};
