<script lang="ts">
  import { translator, browserLanguage, statusLabel } from './lib/i18n';
  let language: string = browserLanguage();
  $: t = translator(language);
  $: document.documentElement.lang = language;
  import { onMount, tick } from 'svelte';
  import { marked } from 'marked';
  import DOMPurify from 'dompurify';
  import { api, demoMode } from './lib/api';
  import { applyTheme, preference, snapshotTheme } from './lib/theme';
  import type { View } from './lib/list';
  import {
    filterItems,
    groupByRepository,
    nextSelection,
    relativeAge,
  } from './lib/list';
  import Splitter from './lib/Splitter.svelte';
  import Activity from './lib/Activity.svelte';
  import type { main, provider, config, app } from '../wailsjs/go/models';
  let fontScale = 1;
  function changeFontScale(value: number) {
    fontScale =
      typeof value === 'number' && Number.isFinite(value)
        ? Math.round(Math.max(0.5, Math.min(2, value)) * 10) / 10
        : 1;
    document.documentElement.style.setProperty(
      '--font-scale',
      String(fontScale),
    );
    try {
      localStorage.setItem('pr-tracker-font-scale', String(fontScale));
    } catch {
      /* Font sizing remains usable when storage is disabled. */
    }
  }
  let view: View = {
    Language: language,
    RefreshSeconds: 300,
    Instances: [],
    Items: [],
    Errors: {},
    CacheError: '',
    SyncedAt: '',
    Revision: 0,
  } as View;
  let kind = 0,
    relation = 0,
    query = '',
    instance = '',
    selected = '',
    detailOpen = false;
  let loading = true,
    refreshing = false,
    error = '',
    themeError = '',
    notice = '',
    detailError = '',
    detailLoading = false;
  let thread: provider.Thread | null = null,
    detailTab = 'details';
  let busy: Record<string, string> = {};
  let pending: Record<string, string> = {};

  async function activity<T>(
    key: string,
    label: string,
    work: () => Promise<T>,
  ): Promise<T> {
    pending[key] = label;
    try {
      return await work();
    } finally {
      delete pending[key];
      pending = { ...pending };
    }
  }
  let search: HTMLInputElement;
  let list: HTMLElement;
  let palette: HTMLDialogElement;
  let confirmation: HTMLDialogElement;
  let settingsDialog: HTMLDialogElement;
  let mergeOptions: provider.MergeOptions | null = null;
  let paletteQuery = '',
    command: main.Request | null = null,
    confirmLabel = '';
  let confirmTarget = '',
    confirmWorktree = '',
    confirmSource = '',
    confirmBase = '',
    confirmProvider = '';
  let confirmUsesDraft = false;
  let draft = '',
    drafts: Record<string, string> = {};
  let settings: config.Config | null = null,
    diagnostics: app.Diagnostics | null = null,
    settingsBusy = false;
  $: settingsBusy = Object.keys(pending).some((key) =>
    key.startsWith('settings-'),
  );
  let autoTimer: ReturnType<typeof setTimeout>;
  let theme = 'system',
    mounted = false,
    detailGeneration = 0;
  let windowWidth = window.innerWidth;
  let sidebarCollapsed = false,
    sidebarDrawerOpen = false,
    sidebarWidth = 200,
    inspectorWidth = 370;
  let sidebarToggle: HTMLButtonElement;
  $: wideLayout = windowWidth >= 1100;
  $: sidebarVisible = wideLayout ? !sidebarCollapsed : sidebarDrawerOpen;
  $: sidebarMax = Math.max(180, Math.min(360, windowWidth - 632));
  $: sidebarSize = Math.max(180, Math.min(sidebarMax, sidebarWidth));
  $: sidebarSpace = wideLayout && sidebarVisible ? sidebarSize + 6 : 0;
  $: inspectorMax = Math.max(300, windowWidth - sidebarSpace - 326);
  $: inspectorSize = Math.max(300, Math.min(inspectorMax, inspectorWidth));

  function saveLayout() {
    try {
      localStorage.setItem(
        'pr-tracker-layout',
        JSON.stringify({ sidebarCollapsed, sidebarWidth, inspectorWidth }),
      );
    } catch {
      /* Layout remains usable when storage is disabled. */
    }
  }
  function toggleSidebar() {
    if (wideLayout) {
      sidebarCollapsed = !sidebarCollapsed;
      saveLayout();
    } else sidebarDrawerOpen = !sidebarDrawerOpen;
  }
  function closeDrawer() {
    sidebarDrawerOpen = false;
    sidebarToggle?.focus();
  }
  const isMac =
    typeof navigator !== 'undefined' && /Mac/.test(navigator.platform);
  const modifier = isMac ? '⌘' : 'Ctrl';
  $: prTabs = [
    [t('Todos'), 0],
    [t('Revisar'), 1],
    [t('Meus'), 2],
    [t('Atribuídos'), 4],
  ] as const;
  $: issueTabs = [
    [t('Todas'), 0],
    [t('Atribuídas'), 4],
    [t('Criadas'), 2],
    [t('Menções'), 8],
  ] as const;
  $: tabs = kind === 0 ? prTabs : issueTabs;
  $: filtered = filterItems(view.Items, kind, relation, query, instance);
  $: groups =
    kind === 0
      ? groupByRepository(filtered)
      : [{ key: 'issues', repo: '', instance: '', items: filtered }];
  $: visible = groups.flatMap((group) => group.items);
  $: item = view.Items.find((p) => p.Key === selected);
  $: commentPresets =
    item?.Kind === 0
      ? ((view.Instances ?? []).find((inst) => inst.Name === item.Instance)
          ?.CommentPresets ?? [])
      : [];
  $: if (mounted && visible.length && !visible.some((p) => p.Key === selected))
    // Select after this update so the inspector renders while Detail is pending.
    void tick().then(() => {
      if (mounted && visible.length && !visible.some((p) => p.Key === selected))
        void select(visible[0].Key, false);
    });
  $: if (mounted && !visible.length && selected) {
    drafts[selected] = draft;
    selected = '';
    thread = null;
    detailOpen = false;
    detailGeneration++;
    detailLoading = false;
  }
  $: commands = [
    { label: t('Atualizar agora'), key: 'r', run: () => refresh() },
    { label: t('Buscar itens'), key: '/', run: () => search?.focus() },
    {
      label: t('Ver conversa'),
      key: 'v',
      run: () => {
        detailTab = 'conversation';
        detailOpen = true;
      },
    },
    {
      label: t('Comentar'),
      key: 'n',
      run: () => {
        detailTab = 'conversation';
        detailOpen = true;
        void tick().then(() => document.getElementById('comment')?.focus());
      },
    },
    { label: t('Abrir no navegador'), key: 'o', run: () => openBrowser() },
    {
      label: t('Criar / atualizar worktree'),
      key: 'w',
      run: () => ask('update_worktree', t('Criar ou atualizar worktree')),
    },
    {
      label: t('Abrir terminal'),
      key: 't',
      run: () => ask('prepare_terminal', t('Abrir terminal no worktree')),
    },
    {
      label: t('Abrir diff'),
      key: 'd',
      run: () => ask('prepare_diff', t('Abrir diff no terminal')),
    },
    {
      label: t('Aprovar'),
      key: 'a',
      run: () => ask('approve', t('Aprovar pull request')),
    },
    { label: 'Merge', key: 'm', run: () => ask('merge', t('Fazer merge')) },
    { label: t('Configurações'), key: ',', run: () => openSettings() },
  ].filter((c) =>
    c.label.toLocaleLowerCase().includes(paletteQuery.toLocaleLowerCase()),
  );
  const textError = (e: unknown) =>
    e instanceof Error ? e.message : String(e);
  const markdown = (value: string) =>
    DOMPurify.sanitize(marked.parse(value, { async: false }) as string);
  function setView(next: View) {
    if (next.Revision < view.Revision) return;
    language = next.Language || browserLanguage();
    view = { ...next, Items: next.Items ?? [], Errors: next.Errors ?? {} };
    scheduleRefresh();
  }
  function scheduleRefresh() {
    clearTimeout(autoTimer);
    if (mounted)
      autoTimer = setTimeout(
        () => void refresh(),
        (view.RefreshSeconds || 300) * 1000,
      );
  }
  async function refresh() {
    if (refreshing || loading) return;
    refreshing = true;
    error = '';
    try {
      setView(await api.refresh());
    } catch (e) {
      error = textError(e);
    } finally {
      refreshing = false;
      loading = false;
      scheduleRefresh();
    }
  }
  async function select(key: string, open = true) {
    if (selected) drafts[selected] = draft;
    selected = key;
    draft = drafts[key] ?? '';
    if (open) detailOpen = true;
    thread = null;
    detailError = '';
    detailLoading = true;
    const generation = ++detailGeneration;
    try {
      const value = await api.detail(key);
      if (generation === detailGeneration) thread = value;
    } catch (e) {
      if (generation === detailGeneration) detailError = textError(e);
    } finally {
      if (generation === detailGeneration) detailLoading = false;
    }
  }
  function switchKind(value: number) {
    kind = value;
    relation = 0;
    detailOpen = false;
  }
  async function openBrowser() {
    if (!item || pending['browser']) return;
    const key = item.Key;
    try {
      await activity('browser', t('Abrindo navegador…'), () =>
        api.openItem(key),
      );
    } catch (e) {
      error = textError(e);
    }
  }
  async function ask(
    action: string,
    label: string,
    force = false,
    removeAfter = false,
    body?: string,
  ) {
    if (!item || busy[item.Key] || pending['merge-options']) return;
    if (item.Kind === 1 && action !== 'comment') {
      notice = t('Ação disponível apenas para pull/merge requests.');
      return;
    }
    confirmTarget = `${item.Repo} ${item.Ref}`;
    confirmWorktree = item.Worktree;
    confirmSource = item.SourceBranch;
    confirmBase = item.TargetBranch;
    confirmProvider = item.Provider;
    command = {
      Key: item.Key,
      Action: action,
      Body: body ?? draft,
      Force: force,
      RemoveAfter: removeAfter,
      Push: false,
    } as main.Request;
    confirmUsesDraft = body === undefined;
    mergeOptions = null;
    if (action === 'merge') {
      try {
        const key = item.Key;
        mergeOptions = await activity(
          'merge-options',
          t('Carregando opções de merge…'),
          () => api.mergeOptions(key),
        );
      } catch (e) {
        error = textError(e);
        return;
      }
    }
    confirmLabel = label;
    confirmation.showModal();
  }
  async function execute() {
    if (!command) return;
    const request = command;
    const clearDraft = confirmUsesDraft;
    const target = view.Items.find((entry) => entry.Key === request.Key);
    confirmation.close();
    const labels: Record<string, string> = {
      comment: t('Enviando comentário…'),
      approve: t('Aprovando PR/MR…'),
      merge: t('Fazendo merge…'),
      close: t('Fechando PR/MR…'),
      mark_ready: t('Marcando como Ready…'),
      mark_draft: t('Marcando como Draft…'),
      checkout: t('Trocando branch…'),
      update_worktree: request.Push
        ? t('Atualizando branch e fazendo push…')
        : t('Atualizando worktree…'),
      update_branch: t('Atualizando branch com a base no servidor…'),
      prepare_terminal: t('Preparando terminal…'),
      prepare_diff: t('Preparando diff…'),
      remove_worktree: t('Removendo worktree…'),
    };
    busy[request.Key] =
      `${target?.Repo ?? ''} ${target?.Ref ?? ''} · ${labels[request.Action] || t('Executando ação…')}`;
    error = '';
    notice = '';
    try {
      const result = await api.execute(request);
      setView(result.View);
      notice = result.Message;
      error = result.Error;
      if (result.ReloadThread && selected === request.Key) {
        if (clearDraft) {
          draft = '';
          delete drafts[request.Key];
        }
        await select(request.Key);
      }
      if (
        result.Dirty &&
        (request.Action === 'remove_worktree' || request.RemoveAfter)
      ) {
        confirmTarget = target ? `${target.Repo} ${target.Ref}` : request.Key;
        confirmWorktree = target?.Worktree ?? '';
        command = {
          Key: request.Key,
          Action: 'remove_worktree',
          Body: '',
          Force: true,
          RemoveAfter: false,
        } as main.Request;
        confirmLabel = t('Descartar alterações locais e remover worktree');
        confirmation.showModal();
      }
      if (result.NeedsClone) {
        await openSettings();
        notice = t(
          'Defina a pasta local do repositório nas configurações e repita a ação.',
        );
      }
    } catch (e) {
      error = textError(e);
    } finally {
      delete busy[request.Key];
      busy = { ...busy };
    }
  }
  async function openSettings() {
    if (pending['open-settings'] || settingsBusy) return;
    try {
      settings = await activity(
        'open-settings',
        t('Carregando configurações…'),
        () => api.settings(),
      );
      settings.Instances ??= [];
      for (const inst of settings.Instances) inst.CommentPresets ??= [];
      settings.Repos ??= [];
      settings.ToolPaths ??= {};
      settings.DesktopTerminal ??= '';
      settings.Language ||= 'system';
      diagnostics = null;
      settingsDialog.showModal();
    } catch (e) {
      error = textError(e);
    }
  }
  async function saveSettings() {
    if (!settings || settingsBusy) return;
    try {
      await activity(
        'settings-save',
        t('Salvando configurações e sincronizando…'),
        async () => {
          setView(await api.saveSettings(settings!));
          systemPalette = await api.theme();
        },
      );
      applyTheme(document.documentElement, preference(theme), systemPalette);
      settingsDialog.close();
      notice = translator(language)('Configurações salvas.');
    } catch (e) {
      error = textError(e);
    }
  }
  async function pickThemeFile() {
    if (settingsBusy) return;
    try {
      const path = await activity(
        'settings-theme',
        t('Selecionando arquivo de cores…'),
        () => api.pickTheme(),
      );
      if (path && settings) settings.ThemeFile = path;
    } catch (e) {
      error = textError(e);
    }
  }
  async function exportColors() {
    if (settingsBusy) return;
    try {
      const path = await activity(
        'settings-export',
        t('Exportando esquema de cores…'),
        () =>
          api.exportTheme(
            snapshotTheme(document.documentElement, systemPalette),
          ),
      );
      if (path) notice = t('Esquema de cores exportado') + ': ' + path;
    } catch (e) {
      error = textError(e);
    }
  }
  async function diagnose() {
    if (settingsBusy) return;
    try {
      diagnostics = await activity(
        'settings-diagnose',
        t('Verificando ferramentas e autenticação…'),
        () => api.diagnose(),
      );
    } catch (e) {
      error = textError(e);
    }
  }
  async function openFolder() {
    if (!item || pending['folder']) return;
    const key = item.Key;
    try {
      await activity('folder', t('Abrindo pasta local…'), () =>
        api.openFolder(key),
      );
    } catch (e) {
      error = textError(e);
    }
  }
  async function pickClone(repo: config.Repo) {
    if (settingsBusy) return;
    try {
      const path = await activity(
        'settings-folder',
        t('Selecionando pasta do clone…'),
        () => api.pickFolder(),
      );
      if (path) repo.Path = path;
    } catch (e) {
      error = textError(e);
    }
  }
  function changeTheme(value: string) {
    theme = preference(value);
    applyTheme(document.documentElement, preference(theme), systemPalette);
    try {
      localStorage.setItem('pr-tracker-theme', value);
    } catch {
      /* storage can be disabled */
    }
  }
  let systemPalette: Awaited<ReturnType<typeof api.theme>>;
  function keydown(e: KeyboardEvent) {
    if ((e.ctrlKey || e.metaKey) && !e.altKey) {
      let next: number;
      if (e.key === '-') next = fontScale - 0.1;
      else if (e.key === '+' || e.key === '=') next = fontScale + 0.1;
      else if (e.key === '0') next = 1;
      else next = NaN;
      if (Number.isFinite(next)) {
        e.preventDefault();
        changeFontScale(next);
        return;
      }
    }
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'k') {
      e.preventDefault();
      if (!confirmation.open && !settingsDialog.open) {
        paletteQuery = '';
        palette.open ? palette.close() : palette.showModal();
      }
      return;
    }
    if (palette?.open || confirmation?.open || settingsDialog?.open) return;
    if (e.key === 'Escape') {
      if (!wideLayout && sidebarDrawerOpen) {
        closeDrawer();
        return;
      }
      detailOpen = false;
      (document.activeElement as HTMLElement)?.blur();
      return;
    }
    if (
      e.target instanceof HTMLElement &&
      e.target.closest(
        'input,textarea,select,[contenteditable="true"],[role="separator"]',
      )
    )
      return;
    if (e.ctrlKey || e.metaKey || e.altKey) return;
    if (
      e.key === 'Enter' &&
      e.target instanceof HTMLElement &&
      e.target.closest('button,a,summary')
    )
      return;
    if (['j', 'k', 'ArrowDown', 'ArrowUp'].includes(e.key)) {
      e.preventDefault();
      const next = nextSelection(
        visible,
        selected,
        ['j', 'ArrowDown'].includes(e.key) ? 1 : -1,
      );
      if (next) {
        void select(next, false);
        void tick().then(() =>
          document
            .getElementById(`row-${next}`)
            ?.scrollIntoView({ block: 'nearest' }),
        );
      }
    } else if (e.key === 'Enter') {
      detailOpen = true;
    } else if (e.key === 'Escape') {
      detailOpen = false;
      search?.blur();
    } else if (e.key === '/') {
      e.preventDefault();
      search?.focus();
    } else if (e.key === 'r') {
      void refresh();
    } else if (e.key === 'v') {
      detailTab = 'conversation';
      detailOpen = true;
    } else if (e.key === 'n') {
      detailTab = 'conversation';
      detailOpen = true;
      void tick().then(() => document.getElementById('comment')?.focus());
    } else if (e.key === 'o') {
      void openBrowser();
    } else if (e.key === 'a') {
      ask('approve', t('Aprovar pull request'));
    } else if (e.key === 'm') {
      ask('merge', t('Fazer merge'));
    } else if (e.key === 't') {
      void ask('prepare_terminal', t('Abrir terminal no worktree'));
    } else if (e.key === 'd') {
      void ask('prepare_diff', t('Abrir diff no terminal'));
    } else if (e.key === 'w') {
      ask('update_worktree', t('Criar ou atualizar worktree'));
    }
  }
  onMount(() => {
    mounted = true;
    try {
      changeFontScale(
        JSON.parse(localStorage.getItem('pr-tracker-font-scale') ?? '1'),
      );
    } catch {
      changeFontScale(1);
    }
    try {
      const layout = JSON.parse(
        localStorage.getItem('pr-tracker-layout') ?? '{}',
      );
      sidebarCollapsed = layout.sidebarCollapsed === true;
      if (
        typeof layout.sidebarWidth === 'number' &&
        Number.isFinite(layout.sidebarWidth)
      )
        sidebarWidth = Math.max(180, Math.min(360, layout.sidebarWidth));
      if (
        typeof layout.inspectorWidth === 'number' &&
        Number.isFinite(layout.inspectorWidth)
      )
        inspectorWidth = Math.max(300, layout.inspectorWidth);
    } catch {
      /* Ignore missing or invalid layout preferences. */
    }
    try {
      changeTheme(localStorage.getItem('pr-tracker-theme') ?? 'system');
      drafts = JSON.parse(localStorage.getItem('pr-tracker-drafts') ?? '{}');
    } catch {
      drafts = {};
    }
    let stopped = false;
    const updateTheme = async () => {
      try {
        const palette = await api.theme();
        if (stopped) return;
        systemPalette = palette;
        themeError = '';
        applyTheme(document.documentElement, preference(theme), palette);
      } catch (e) {
        if (!stopped) themeError = textError(e);
        /* Keep the last working palette if its file becomes unreadable. */
      }
    };
    void updateTheme();
    const themeTimer = setInterval(updateTheme, 2000);
    void (async () => {
      try {
        setView(await api.load());
      } catch (e) {
        error = textError(e);
      } finally {
        loading = false;
      }
      if (!stopped) await refresh();
    })();
    const saveDrafts = () => {
      if (selected) drafts[selected] = draft;
      try {
        localStorage.setItem('pr-tracker-drafts', JSON.stringify(drafts));
      } catch {}
    };
    window.addEventListener('beforeunload', saveDrafts);
    const draftTimer = setInterval(saveDrafts, 2000);
    return () => {
      stopped = true;
      mounted = false;
      clearTimeout(autoTimer);
      clearInterval(draftTimer);
      clearInterval(themeTimer);
      saveDrafts();
      window.removeEventListener('beforeunload', saveDrafts);
    };
  });
</script>

<svelte:window
  onkeydown={keydown}
  bind:innerWidth={windowWidth}
  onresize={() => (sidebarDrawerOpen = false)}
/>
<div
  class="shell"
  class:route-detail={detailOpen}
  style={`--sidebar-width: ${sidebarSpace ? sidebarSize : 0}px; --sidebar-divider: ${sidebarSpace ? 6 : 0}px; --inspector-width: ${inspectorSize}px;`}
>
  <header class="topbar">
    <button
      bind:this={sidebarToggle}
      class="icon-button sidebar-toggle"
      title={sidebarVisible
        ? t('Recolher menu lateral')
        : t('Abrir menu lateral')}
      aria-label={sidebarVisible
        ? t('Recolher menu lateral')
        : t('Abrir menu lateral')}
      aria-expanded={sidebarVisible}
      aria-controls="workspace-sidebar"
      onclick={toggleSidebar}>☰</button
    >
    <div class="brand">
      <span class="brand-icon">⑂</span> pr-tracker
      <span class="desktop-label">DESKTOP</span>
    </div>
    <nav class="kind-tabs" aria-label={t('Tipo de item')}>
      <button class:active={kind === 0} onclick={() => switchKind(0)}
        >PRs</button
      ><button class:active={kind === 1} onclick={() => switchKind(1)}
        >Issues</button
      >
    </nav>
    <label class="search"
      ><span aria-hidden="true">⌕</span><input
        bind:this={search}
        bind:value={query}
        placeholder={t('Buscar título, repo, autor…')}
        aria-label={t('Buscar itens')}
      /><kbd>/</kbd></label
    >
    <button
      class="icon-button"
      title={`${t('Comandos')} (${modifier}+K)`}
      aria-label={t('Abrir comandos')}
      onclick={() => {
        paletteQuery = '';
        palette.showModal();
      }}>⌘</button
    >
    <button
      class="icon-button"
      disabled={refreshing || loading}
      title={t('Atualizar (r)')}
      aria-label={t('Atualizar')}
      onclick={refresh}
      >{#if refreshing || loading}<Activity
          compact
          label={t('Sincronizando itens…')}
        />{:else}↻{/if}</button
    >
    <button
      class="icon-button"
      title={t('Configurações')}
      aria-label={t('Configurações')}
      disabled={!!pending['open-settings'] || settingsBusy}
      class:spinning={!!pending['open-settings']}
      onclick={openSettings}
    >
      <svg
        class="settings-icon"
        aria-hidden="true"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="1.6"
        stroke-linecap="round"
        stroke-linejoin="round"
      >
        <path
          d="M18.66 9.84L21.38 10.51L21.38 13.49L18.66 14.16L18.24 15.18L19.69 17.58L17.58 19.69L15.18 18.24L14.16 18.66L13.49 21.38L10.51 21.38L9.84 18.66L8.82 18.24L6.42 19.69L4.31 17.58L5.76 15.18L5.34 14.16L2.62 13.49L2.62 10.51L5.34 9.84L5.76 8.82L4.31 6.42L6.42 4.31L8.82 5.76L9.84 5.34L10.51 2.62L13.49 2.62L14.16 5.34L15.18 5.76L17.58 4.31L19.69 6.42L18.24 8.82Z"
        />
        <circle cx="12" cy="12" r="3" />
      </svg>
    </button>
  </header>
  <div class="notifications">
    {#if loading || refreshing}<Activity
        progress
        label={loading ? t('Carregando sua fila') : t('Sincronizando itens…')}
      />{/if}
    {#each Object.entries(pending) as [key, label] (key)}<Activity
        {label}
        progress
      />{/each}
    {#each Object.entries(busy) as [key, label] (key)}<Activity
        {label}
        progress
      />{/each}
    {#if themeError}<div class="banner error" role="alert">
        {themeError}
      </div>{/if}
    {#if error && error !== themeError}<div class="banner error" role="alert">
        {error}<button
          aria-label={t('Fechar erro')}
          onclick={() => (error = '')}>×</button
        >
      </div>{/if}
    {#if notice}<div class="banner" role="status">
        {notice}<button
          aria-label={t('Fechar aviso')}
          onclick={() => (notice = '')}>×</button
        >
      </div>{/if}
  </div>
  {#if sidebarVisible && !wideLayout}
    <button
      class="sidebar-backdrop"
      aria-label={t('Fechar menu lateral')}
      onclick={closeDrawer}
    ></button>
  {/if}
  <aside
    id="workspace-sidebar"
    class="sidebar"
    hidden={!sidebarVisible}
    aria-label={t('Menu lateral')}
  >
    {#if !wideLayout}<button class="drawer-close" onclick={closeDrawer}
        >{t('← Recolher menu')}</button
      >{/if}
    <div class="eyebrow">WORKSPACE</div>
    <h2>{t('Seu trabalho,')}<br />{t('em um lugar.')}</h2>
    <button class:chosen={!instance} onclick={() => (instance = '')}
      ><span>{t('◈ Todas as instâncias')}</span><span class="count"
        >{view.Items.length}</span
      ></button
    >
    <div class="eyebrow section-label">{t('INSTÂNCIAS')}</div>
    {#each (view.Instances ?? [])
      .filter((entry) => !entry.Disabled)
      .map((entry) => entry.Name) as name}
      <button
        class:chosen={instance === name}
        onclick={() => (instance = instance === name ? '' : name)}
        ><span><i class="dot" class:bad={!!view.Errors[name]}></i>{name}</span
        ><span class="count"
          >{view.Items.filter((p) => p.Instance === name).length}</span
        ></button
      >
    {/each}
    <button class="subtle" onclick={openSettings}
      >{t('＋ Adicionar instância')}</button
    >
    <div class="eyebrow section-label">{t('VISÕES')}</div>
    {#each tabs as [label, value]}<button
        class:chosen={relation === value}
        onclick={() => (relation = value)}
        ><span>{label}</span><span class="count"
          >{filterItems(view.Items, kind, value, '', instance).length}</span
        ></button
      >{/each}
    <div class="sidebar-bottom">
      <span class="dot"></span>
      {demoMode
        ? t('Demonstração · dados fictícios')
        : t('Cache local · SQLite')}<small
        >{view.SyncedAt && !String(view.SyncedAt).startsWith('0001')
          ? `${t('Sincronizado ')}${relativeAge(String(view.SyncedAt), language)}`
          : t('Aguardando sincronização')}</small
      >
    </div>
  </aside>
  {#if wideLayout && sidebarVisible}
    <Splitter
      {language}
      label={t('Largura do menu lateral')}
      controls="workspace-sidebar"
      value={sidebarSize}
      min={180}
      max={sidebarMax}
      onresize={(value) => (sidebarWidth = value)}
      oncommit={saveLayout}
    />
  {/if}
  <main id="item-list" class="list-pane" bind:this={list}>
    <div class="list-heading">
      <div class="eyebrow">{t('SUA FILA DE TRABALHO')}</div>
      <h1>
        {kind === 0 ? 'Pull requests' : 'Issues'}<span class="total"
          >{visible.length}</span
        >
      </h1>
      <p>
        {instance || t('Todas as instâncias')} <span>·</span>
        {kind === 0
          ? t('por repositório · mais recentes primeiro')
          : t('mais recentes primeiro')}
      </p>
    </div>
    {#if demoMode}<div class="banner demo">
        {t('Demonstração visual — nenhuma ação altera dados.')}
      </div>{/if}
    {#if view.CacheError}<div class="banner error">
        {t('Cache indisponível:')}
        {view.CacheError}
      </div>{/if}
    {#each Object.entries(view.Errors) as [name, message]}<div
        class="banner error"
      >
        <strong>{name}</strong>: {message}
        {t('· exibindo dados anteriores.')}
      </div>{/each}
    <nav class="filters" aria-label={t('Filtrar por relação')}>
      {#each tabs as [label, value]}<button
          class:active={relation === value}
          onclick={() => {
            relation = value;
            detailOpen = false;
          }}
          >{label}<span
            >{filterItems(view.Items, kind, value, '', instance).length}</span
          ></button
        >{/each}
    </nav>
    <div class="table-head">
      <span>{t('REPOSITÓRIO / TÍTULO')}</span><span>{t('AUTOR')}</span><span
        >CI</span
      ><span>{t('ATUALIZADO ↓')}</span>
    </div>
    <div
      class="rows"
      aria-label={t('Lista de itens')}
      aria-busy={loading || refreshing}
    >
      {#if loading}<div class="empty">
          <div class="empty-symbol">
            <Activity compact label={t('Carregando sua fila')} />
          </div>
          <h2>{t('Carregando sua fila')}</h2>
          <p>{t('Buscando primeiro os dados do cache local.')}</p>
        </div>
      {:else if !visible.length}<div class="empty">
          <div class="empty-symbol">✓</div>
          <h2>{query ? t('Nenhum resultado') : t('Tudo em dia por aqui')}</h2>
          <p>
            {view.Items.length
              ? t('Experimente outra visão ou busca.')
              : t(
                  'Configure uma instância ou atualize para buscar seus PRs e issues.',
                )}
          </p>
          <button
            onclick={view.Items.length
              ? () => {
                  query = '';
                  relation = 0;
                  instance = '';
                }
              : openSettings}
            >{view.Items.length
              ? t('Limpar filtros')
              : t('Configurar instâncias')}</button
          >
        </div>
      {:else}{#each groups as group (group.key)}
          <div
            class="repository-group"
            role="group"
            aria-label={group.repo
              ? `${group.repo} · ${group.instance}`
              : t('Lista de itens')}
          >
            {#if kind === 0}
              <header class="repository-heading">
                <h2>
                  {group.repo}<span class="repository-count"
                    >{group.items.length}</span
                  >
                </h2>
                <small>{group.instance}</small>
              </header>
            {/if}
            {#each group.items as row (row.Key)}
              <button
                id={`row-${row.Key}`}
                class="item-row"
                class:selected={selected === row.Key}
                aria-pressed={selected === row.Key}
                aria-busy={!!busy[row.Key]}
                aria-label={`${row.Repo} ${row.Ref}: ${row.Title}`}
                onclick={() => select(row.Key)}
              >
                <span class="item-main"
                  ><span class="repo-line"
                    ><span class="branch-icon" class:draft={row.Draft}
                      >{row.Kind === 1 ? '◉' : row.Draft ? '▧' : '⑂'}</span
                    >{#if kind === 1}{row.Repo}{/if}<span class="ref"
                      >{row.Ref}</span
                    >{#if busy[row.Key]}<Activity
                        compact
                        label={busy[row.Key]}
                      />{/if}{#if row.Draft}<span class="pill"
                        >{t('Rascunho')}</span
                      >{/if}{#if row.Conflicts}<span
                        class="pill red"
                        title={t(
                          'Conflitos precisam ser resolvidos antes do merge.',
                        )}>{t('Conflitos')}</span
                      >{/if}</span
                  ><strong>{row.Title}</strong><span class="mobile-meta"
                    >{row.Author} <span>·</span>
                    {relativeAge(String(row.UpdatedAt), language)}
                    <span>·</span>
                    {row.Comments}
                    {t('comentários')}</span
                  ></span
                >
                <span class="author"
                  ><span class="avatar"
                    >{row.Author.slice(0, 2).toUpperCase()}</span
                  >{row.Author}</span
                >
                <span
                  class="ci"
                  class:success={row.CI === 'success'}
                  class:failure={row.CI === 'failure'}
                  title={row.CI
                    ? statusLabel(language, row.CI)
                    : t('Sem checks')}
                  >{row.CI === 'success'
                    ? '✓'
                    : row.CI === 'failure'
                      ? '×'
                      : row.CI === 'pending'
                        ? '◷'
                        : '—'}</span
                >
                <span class="updated"
                  >{relativeAge(String(row.UpdatedAt), language)}<small
                    >◯ {row.Comments}</small
                  ></span
                >
              </button>
            {/each}
          </div>
        {/each}{/if}
    </div>
    <footer class="list-footer">
      <span
        ><kbd>j</kbd><kbd>k</kbd>
        {t('navegar')} <kbd>Enter</kbd>
        {t('abrir')}</span
      ><button onclick={() => palette.showModal()}
        >{modifier}+K <span>{t('comandos')}</span></button
      >
    </footer>
  </main>
  {#if windowWidth >= 700}
    <Splitter
      {language}
      label={t('Largura dos detalhes')}
      controls="item-inspector"
      value={inspectorSize}
      min={300}
      max={inspectorMax}
      direction={-1}
      onresize={(value) => (inspectorWidth = value)}
      oncommit={saveLayout}
    />
  {/if}
  <section
    id="item-inspector"
    class="inspector"
    aria-label={t('Detalhes do item')}
  >
    <div class="inspector-heading">
      <span>INSPECTOR</span><button
        class="back"
        onclick={() => (detailOpen = false)}>{t('← Voltar')}</button
      >
    </div>
    {#if item}
      <div class="detail-scroll">
        <div class="detail-reference">
          <span class="branch-icon">⑂</span>
          {item.Ref}<span class="pill green"
            >{item.Draft ? t('Rascunho') : t('Aberto')}</span
          >
          {#if item.Kind === 0 && ['github', 'gitlab', 'gitea'].includes(item.Provider)}
            <button
              class="draft-toggle"
              disabled={!!busy[selected]}
              onclick={() =>
                ask(
                  item.Draft ? 'mark_ready' : 'mark_draft',
                  item.Draft ? t('Marcar como Ready') : t('Marcar como Draft'),
                )}
              >{item.Draft
                ? t('Marcar como Ready')
                : t('Marcar como Draft')}</button
            >
          {/if}
        </div>
        <h2 class="detail-title">{item.Title}</h2>
        <p class="detail-meta">{item.Repo} <span>·</span> {item.Author}</p>
        {#if item.Kind === 0}<div class="branches">
            <code>{item.SourceBranch || '—'}</code><span>→</span><code
              >{item.TargetBranch || '—'}</code
            >
            <button
              class="branch-update"
              disabled={!!busy[selected] || !item.Worktree}
              title={item.Worktree
                ? t(
                    'Buscar novos commits da branch de origem e atualizar o worktree local',
                  )
                : t('Crie um worktree para atualizar a branch local')}
              onclick={() =>
                ask('update_worktree', t('Atualizar branch local'))}
              >{t('Atualizar branch local')}</button
            >
          </div>{/if}
        {#if item.Kind === 0 && ['github', 'gitlab', 'gitea'].includes(item.Provider)}
          <div class="branch-sync">
            {#if item.Conflicts}<span
                class="pill red"
                title={t('Conflitos precisam ser resolvidos antes do merge.')}
                >{t('Conflitos')}</span
              >{/if}
            {#if item.BehindBase}<span class="pill"
                >{t('Branch desatualizada')}</span
              >{/if}
            <button
              class="secondary"
              disabled={!!busy[selected] ||
                !!pending['merge-options'] ||
                item.Conflicts ||
                (item.Provider === 'github' && !item.CanUpdateBranch)}
              title={item.Conflicts
                ? t('Resolva os conflitos antes de atualizar a branch')
                : item.Provider === 'github' && !item.CanUpdateBranch
                  ? t(
                      'Branch atualizada ou sem permissão para atualizar no GitHub',
                    )
                  : t(
                      'Incorporar a branch base na origem do PR/MR no servidor',
                    )}
              onclick={() =>
                ask('update_branch', t('Atualizar com branch base'))}
              >{t('Atualizar com branch base')}</button
            >
          </div>
        {/if}
        <div class="labels">
          {#each item.Labels ?? [] as label}<span>{label}</span>{/each}
        </div>
        <nav class="detail-tabs" aria-label={t('Conteúdo do detalhe')}>
          <button
            class:active={detailTab === 'details'}
            onclick={() => (detailTab = 'details')}>{t('Detalhes')}</button
          ><button
            class:active={detailTab === 'checks'}
            onclick={() => (detailTab = 'checks')}
            >Checks <span>{item.Checks?.length ?? 0}</span></button
          ><button
            class:active={detailTab === 'conversation'}
            onclick={() => (detailTab = 'conversation')}
            >{t('Conversa')} <span>{item.Comments}</span></button
          >
        </nav>
        {#if commentPresets.length}
          <section
            class="comment-presets"
            aria-label={t('Comentários predefinidos')}
          >
            <h3>{t('Comentários predefinidos')}</h3>
            <div class="preset-buttons">
              {#each commentPresets as preset}
                <button
                  disabled={!!busy[selected]}
                  onclick={() =>
                    ask(
                      'comment',
                      t('Enviar comentário'),
                      false,
                      false,
                      preset.Body,
                    )}>{preset.Label}</button
                >
              {/each}
            </div>
          </section>
        {/if}
        {#if detailLoading}<Activity
            label={t('Carregando conversa…')}
            progress
          />{/if}
        {#if detailError}<div class="banner error">
            {detailError}<button onclick={() => select(selected)}
              >{t('Tentar novamente')}</button
            >
          </div>{/if}
        {#if detailTab === 'details'}
          {#if thread}<article class="markdown">
              {@html markdown(thread.Body || t('_Sem descrição._'))}
            </article>{/if}
          {#if item.Kind === 0}<div class="detail-section">
              <h3>{t('Revisões e aprovações')}</h3>
              <p
                class="review-status"
                class:success={item.Review === 'approved'}
              >
                {item.Review === 'approved'
                  ? t('✓ Aprovado')
                  : item.Review === 'changes_requested'
                    ? t('Alterações solicitadas')
                    : t('Aguardando revisão')}
              </p>
              <p class="muted">
                {(item.ApprovedBy ?? []).join(', ') ||
                  t('Nenhuma aprovação informada')}
              </p>
              {#if item.Conflicts}<div class="banner error">
                  {t('Conflitos precisam ser resolvidos antes do merge.')}
                </div>{/if}
            </div>
            <div class="detail-section">
              <h3>
                {t('Arquivos alterados')}
                <span class="count">{item.Files}</span>
              </h3>
              <div class="diff-stats">
                <span class="success">+{item.Additions}</span><span
                  class="failure">−{item.Deletions}</span
                ><span class="diff-bar"
                  ><i
                    style={`width:${(item.Additions / Math.max(1, item.Additions + item.Deletions)) * 100}%`}
                  ></i></span
                >
              </div>
            </div>
            <div class="detail-section">
              <h3>Worktree</h3>
              <p class="path muted">
                {item.Worktree || t('Nenhum worktree local')}
              </p>
              <button
                class="secondary"
                disabled={!!busy[selected]}
                onclick={() =>
                  ask(
                    'update_worktree',
                    item?.Worktree
                      ? t('Atualizar worktree')
                      : t('Criar worktree'),
                  )}
                >{item.Worktree ? t('Atualizar worktree') : t('Criar worktree')}
                <kbd>w</kbd></button
              >
            </div>{/if}
        {:else if detailTab === 'checks'}
          <div class="detail-section">
            <h3>CI / Checks</h3>
            {#each item.Checks ?? [] as check}<div class="check-row">
                <span
                  class:success={check.State === 'success'}
                  class:failure={check.State === 'failure'}
                  >{check.State === 'success'
                    ? '✓'
                    : check.State === 'failure'
                      ? '×'
                      : '◷'}</span
                ><strong>{check.Name}</strong><span class="muted"
                  >{statusLabel(language, check.State)}</span
                >
              </div>{:else}<p class="muted">
                {t('Nenhum check informado pelo provider.')}
              </p>{/each}
          </div>
        {:else}
          {#if thread}<article class="markdown">
              {@html markdown(thread.Body || t('_Sem descrição._'))}
            </article>
            {#each thread.Comments ?? [] as comment}<article class="comment">
                <header>
                  <strong>{comment.Author}</strong><small
                    >{relativeAge(String(comment.CreatedAt), language)}</small
                  >
                </header>
                {#if comment.Review}<span class="pill"
                    >{statusLabel(language, comment.Review)}</span
                  >{/if}{#if comment.Path}<p class="path muted">
                    {comment.Path}:{comment.Line}
                  </p>{/if}
                <div class="markdown">{@html markdown(comment.Body)}</div>
              </article>{/each}{/if}
          <label class="composer"
            >{t('Adicionar comentário')}<textarea
              id="comment"
              bind:value={draft}
              placeholder={t('Escreva em Markdown…')}
              maxlength="65536"
              rows="5"></textarea></label
          >
          <div class="composer-footer">
            <small class="muted">{t('Rascunho salvo neste dispositivo')}</small
            ><button
              class="primary"
              disabled={!draft.trim() || !!busy[selected]}
              onclick={() => ask('comment', t('Enviar comentário'))}
              >{t('Revisar envio →')}</button
            >
          </div>
        {/if}
      </div>
      <div class="detail-actions">
        {#if item.Kind === 0}<button
            class="approve"
            disabled={!!busy[selected]}
            onclick={() => ask('approve', t('Aprovar pull request'))}
            >{t('✓ Aprovar')}</button
          ><button
            class="primary"
            disabled={!!busy[selected]}
            onclick={() => ask('merge', t('Fazer merge'))}>Merge</button
          >{/if}
        <details class="more">
          <summary>{t('Mais ações')}</summary>
          <div>
            <button disabled={!!pending['browser']} onclick={openBrowser}
              >{t('Abrir no navegador')}</button
            ><button
              disabled={(!item.Clone && !item.Worktree) || !!pending['folder']}
              onclick={openFolder}>{t('Abrir pasta local')}</button
            >{#if item.Kind === 0}<button
                disabled={!!busy[selected]}
                onclick={() =>
                  ask('prepare_terminal', t('Abrir terminal no worktree'))}
                >{t('Abrir terminal')}</button
              ><button
                disabled={!!busy[selected]}
                onclick={() => ask('prepare_diff', t('Abrir diff no terminal'))}
                >{t('Ver diff')}</button
              ><button
                disabled={!!busy[selected]}
                onclick={() =>
                  ask('checkout', t('Trocar branch do clone local'))}
                >{t('Checkout no clone')}</button
              ><button
                disabled={!!busy[selected]}
                onclick={() => ask('close', t('Fechar sem merge'))}
                >{t('Fechar sem merge')}</button
              >{#if item.Worktree}<button
                  disabled={!!busy[selected]}
                  onclick={() =>
                    ask(
                      'approve',
                      t('Aprovar e remover worktree'),
                      false,
                      true,
                    )}>{t('Aprovar e remover worktree')}</button
                ><button
                  disabled={!!busy[selected]}
                  onclick={() =>
                    ask('merge', t('Merge e remover worktree'), false, true)}
                  >{t('Merge e remover worktree')}</button
                ><button
                  disabled={!!busy[selected]}
                  onclick={() =>
                    ask(
                      'close',
                      t('Fechar sem merge e remover worktree'),
                      false,
                      true,
                    )}>{t('Fechar e remover worktree')}</button
                >
                <button
                  disabled={!!busy[selected]}
                  onclick={() => ask('remove_worktree', t('Remover worktree'))}
                  >{t('Remover worktree')}</button
                >{/if}{/if}
          </div>
        </details>
      </div>
    {:else}<div class="empty">
        <div class="empty-symbol">⑂</div>
        <h2>{t('Um pouco de contexto')}</h2>
        <p>{t('Selecione um item para ver detalhes, checks e conversa.')}</p>
      </div>{/if}
  </section>
</div>

<dialog bind:this={palette} class="palette" aria-label={t('Comandos')}>
  <form method="dialog">
    <button class="dialog-close" aria-label={t('Fechar comandos')}>×</button>
  </form>
  <label class="palette-search"
    >⌕ <input
      bind:value={paletteQuery}
      placeholder={t('Digite uma ação…')}
      aria-label={t('Buscar comando')}
    /></label
  >
  <div class="command-list">
    {#each commands as cmd}<button
        onclick={() => {
          palette.close();
          cmd.run();
        }}><span>{cmd.label}</span><kbd>{cmd.key}</kbd></button
      >{:else}<p>{t('Nenhum comando encontrado.')}</p>{/each}
  </div>
  <small class="muted">{modifier}{t('+K abre comandos · Esc fecha')}</small>
</dialog>
<dialog
  bind:this={confirmation}
  class="confirm-dialog"
  aria-labelledby="confirmation-title"
>
  <h2 id="confirmation-title">{confirmLabel}?</h2>
  <p>
    {confirmTarget}
  </p>
  {#if command?.Force}<p class="path">{confirmWorktree}</p>
    <div class="banner error">
      {t(
        'As alterações não commitadas serão perdidas. Esta operação não pode ser desfeita.',
      )}
    </div>{/if}{#if command?.Action === 'update_branch'}<p class="muted">
      {confirmBase} → {confirmSource}<br />
      {confirmProvider === 'gitlab'
        ? t(
            'Faz rebase da branch de origem sobre a branch base no servidor e publica a atualização.',
          )
        : t(
            'Incorpora os commits da branch base na branch de origem por merge no servidor e publica a atualização.',
          )}
    </p>{/if}{#if command?.Action === 'update_worktree'}<p class="muted">
      {t(
        'Busca os novos commits da branch de origem do PR/MR e atualiza o worktree local sem criar commits de merge. Alterações locais ou histórico divergente impedem a atualização.',
      )}
    </p>
    <label class="push-option">
      <input type="checkbox" bind:checked={command.Push} />
      {t('Fazer push após atualizar a branch')}
    </label>
    <p class="muted">
      {t(
        'Envia para a branch de origem no remoto configurado como upstream do worktree, sem force push.',
      )}
    </p>
  {/if}{#if command?.Action === 'comment'}<pre
      class="comment-preview">{command.Body}</pre>{/if}{#if command?.Action === 'merge'}<p
    >
      {t('Método:')} <strong>{mergeOptions?.Method}</strong><br />Auto-merge: {mergeOptions?.Auto
        ? t('sim')
        : t('não')}<br />{t('Apagar branch de origem:')}
      {mergeOptions?.DeleteBranch ? t('sim') : t('não')}
    </p>{/if}
  <div class="dialog-actions">
    <button onclick={() => confirmation.close()}>{t('Cancelar')}</button><button
      class:danger={command?.Force ||
        command?.Action === 'close' ||
        command?.Action === 'remove_worktree'}
      class="primary"
      onclick={execute}>{t('Confirmar')}</button
    >
  </div>
</dialog>
<dialog
  bind:this={settingsDialog}
  class="settings-dialog"
  aria-labelledby="settings-title"
>
  <form method="dialog">
    <button class="dialog-close" aria-label={t('Fechar configurações')}
      >×</button
    >
  </form>
  <div class="eyebrow">{t('PREFERÊNCIAS')}</div>
  <h2 id="settings-title">{t('Configurações')}</h2>
  {#each Object.entries(pending).filter( ([key]) => key.startsWith('settings-') ) as [key, label] (key)}<Activity
      {label}
      progress
    />{/each}
  {#if error}<div class="banner error" role="alert">{error}</div>{/if}<label
    >{t('Tema')}<select
      value={theme}
      onchange={(e) => changeTheme(e.currentTarget.value)}
      ><option value="system">{t('Sistema')}</option><option value="dark"
        >{t('Escuro')}</option
      ><option value="light">{t('Claro')}</option></select
    ></label
  >
  {#if settings}
    <label
      >{t('Arquivo de cores')}<input
        bind:value={settings.ThemeFile}
        placeholder="colors.toml"
      /></label
    >
    <p class="muted">
      {t('Vazio segue o tema do sistema; caminho relativo à configuração')}
    </p>
    <div class="actions">
      <button disabled={settingsBusy} onclick={pickThemeFile}
        >{t('Selecionar arquivo')}</button
      >
      <button disabled={settingsBusy} onclick={exportColors}
        >{t('Exportar esquema de cores')}</button
      >
    </div>
    <label
      >{t('Idioma')}<select bind:value={settings.Language}
        ><option value="system">{t('Sistema')}</option><option value="en"
          >English</option
        ><option value="pt">Português</option></select
      ></label
    >
    <div class="settings-grid">
      <label
        >{t('Intervalo de atualização')}<input
          bind:value={settings.RefreshInterval}
          placeholder="5m"
        /></label
      ><label
        >{t('Pasta de worktrees')}<input
          bind:value={settings.WorktreeDir}
          placeholder={t('Padrão da aplicação')}
        /></label
      >
    </div>
    <label
      >{t('Terminal desktop')}<input
        bind:value={settings.DesktopTerminal}
        placeholder={t('Detectar automaticamente (kitty, foot, wezterm…)')}
      /></label
    >
    <details>
      <summary>{t('Caminhos das ferramentas')}</summary
      >{#each ['git', 'gh', 'glab', 'tea', 'hunk'] as tool}<label
          >{tool}<input
            bind:value={settings.ToolPaths[tool]}
            placeholder={t('Detectar no PATH')}
          /></label
        >{/each}
    </details>
    <h3>{t('Instâncias')}</h3>
    {#each settings.Instances as inst, i}<fieldset>
        <legend>{t('Instância')} {i + 1}</legend>
        <div class="settings-grid">
          <label>{t('Nome')}<input bind:value={inst.Name} /></label><label
            >Provider<select bind:value={inst.Provider}
              ><option value="github">GitHub</option><option value="gitlab"
                >GitLab</option
              ><option value="gitea">Gitea</option></select
            ></label
          ><label
            >Host<input
              bind:value={inst.Host}
              placeholder="github.com"
            /></label
          ><label
            >{t('Método de merge')}<select bind:value={inst.MergeMethod}
              ><option value="">{t('Padrão (merge)')}</option><option
                value="merge">Merge</option
              ><option value="squash">Squash</option><option value="rebase"
                >Rebase</option
              ></select
            ></label
          >
        </div>
        <label class="checkbox"
          ><input type="checkbox" bind:checked={inst.Disabled} />{t(
            'Desativada',
          )}</label
        ><label class="checkbox"
          ><input type="checkbox" bind:checked={inst.AutoMerge} />{t(
            'Auto-merge quando o pipeline passar',
          )}</label
        ><label class="checkbox"
          ><input type="checkbox" bind:checked={inst.DeleteBranch} />{t(
            'Apagar branch de origem após merge',
          )}</label
        >
        <section
          class="preset-editor"
          aria-label={t('Comentários predefinidos')}
        >
          <h4>{t('Comentários predefinidos')}</h4>
          <p class="muted">
            {t(
              'Botões de comentário nos PRs/MRs desta instância. O envio sempre exige confirmação.',
            )}
          </p>
          {#each inst.CommentPresets ?? [] as preset, j}
            <fieldset>
              <legend>{t('Comentário')} {j + 1}</legend>
              <label
                >{t('Nome do botão')}<input bind:value={preset.Label} /></label
              >
              <label
                >{t('Texto do comentário')}<textarea
                  bind:value={preset.Body}
                  rows="3"
                  maxlength="65536"></textarea></label
              >
              <button
                class="text-danger"
                aria-label={`${t('Remover comentário')} ${j + 1}`}
                onclick={() =>
                  (inst.CommentPresets = inst.CommentPresets.filter(
                    (_, n) => n !== j,
                  ))}>{t('Remover comentário')}</button
              >
            </fieldset>
          {/each}
          <button
            onclick={() =>
              (inst.CommentPresets = [
                ...(inst.CommentPresets ?? []),
                { Label: '', Body: '' },
              ])}>{t('Adicionar comentário predefinido')}</button
          >
        </section>
        <button
          class="text-danger"
          onclick={() => {
            if (settings) {
              settings.Repos = settings.Repos.filter(
                (repo) => repo.Instance !== inst.Name,
              );
              settings.Instances = settings.Instances.filter((_, n) => n !== i);
            }
          }}>{t('Remover da configuração')}</button
        >
      </fieldset>{/each}
    <button
      onclick={() => {
        if (settings)
          settings.Instances = [
            ...settings.Instances,
            {
              Name: '',
              Provider: 'github',
              Host: '',
              MergeMethod: 'merge',
              AutoMerge: false,
              DeleteBranch: false,
              Disabled: false,
              CommentPresets: [],
            } as config.Instance,
          ];
      }}>{t('＋ Adicionar instância')}</button
    >
    <h3>{t('Repositórios locais')}</h3>
    {#each settings.Repos as repo, i}<fieldset>
        <div class="settings-grid">
          <label
            >{t('Instância')}<select bind:value={repo.Instance}
              >{#each settings.Instances as inst}<option value={inst.Name}
                  >{inst.Name}</option
                >{/each}</select
            ></label
          ><label
            >{t('Repositório')}<input
              bind:value={repo.Name}
              placeholder={t('organização/repo')}
            /></label
          ><label
            >{t('Pasta do clone')}<input
              bind:value={repo.Path}
              placeholder="/home/…"
            /><button disabled={settingsBusy} onclick={() => pickClone(repo)}
              >{t('Selecionar pasta…')}</button
            ></label
          ><label
            >Remote<input
              bind:value={repo.Remote}
              placeholder="origin"
            /></label
          >
        </div>
        <label class="checkbox"
          ><input type="checkbox" bind:checked={repo.TrackAll} />{t(
            'Acompanhar todos os PRs',
          )}</label
        ><button
          class="text-danger"
          onclick={() => {
            if (settings)
              settings.Repos = settings.Repos.filter((_, n) => n !== i);
          }}>{t('Remover mapeamento')}</button
        >
      </fieldset>{/each}<button
      onclick={() => {
        if (settings)
          settings.Repos = [
            ...settings.Repos,
            {
              Instance: settings.Instances[0]?.Name ?? '',
              Name: '',
              Path: '',
              Remote: '',
              TrackAll: false,
              MergeMethod: '',
            } as config.Repo,
          ];
      }}>{t('＋ Mapear repositório')}</button
    >
    <h3>{t('Ferramentas e autenticação')}</h3>
    <button disabled={settingsBusy} onclick={diagnose}
      >{pending['settings-diagnose']
        ? t('Verificando…')
        : t('Executar diagnóstico')}</button
    >{#if diagnostics}{#each diagnostics.Tools as tool}<p class="diagnostic">
          <span class:success={!!tool.Path} class:failure={!tool.Path}
            >{tool.Path ? '✓' : '×'}</span
          ><strong>{tool.Name}</strong><span
            >{tool.Path || t('Não encontrada no PATH')}</span
          >
        </p>{/each}{#each diagnostics.Instances as inst}<p class="diagnostic">
          {inst.Name}: {inst.Disabled
            ? t('desativada')
            : inst.Error || t('autenticada')}
        </p>{/each}{/if}
    <div class="dialog-actions">
      <button onclick={() => settingsDialog.close()}>{t('Cancelar')}</button
      ><button class="primary" disabled={settingsBusy} onclick={saveSettings}
        >{pending['settings-save']
          ? t('Salvando…')
          : t('Salvar configurações')}</button
      >
    </div>{/if}
</dialog>
