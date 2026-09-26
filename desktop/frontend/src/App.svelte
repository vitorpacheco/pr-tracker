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
  import { filterItems, nextSelection, relativeAge } from './lib/list';
  import Splitter from './lib/Splitter.svelte';
  import type { main, provider, config, app } from '../wailsjs/go/models';
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
  let busy: Record<string, boolean> = {};
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
    confirmWorktree = '';
  let draft = '',
    drafts: Record<string, string> = {};
  let settings: config.Config | null = null,
    diagnostics: app.Diagnostics | null = null,
    settingsBusy = false;
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
  $: visible = filterItems(view.Items, kind, relation, query, instance);
  $: item = view.Items.find((p) => p.Key === selected);
  $: if (mounted && visible.length && !visible.some((p) => p.Key === selected))
    select(visible[0].Key, false);
  $: if (mounted && !visible.length && selected) {
    drafts[selected] = draft;
    selected = '';
    thread = null;
    detailOpen = false;
    detailGeneration++;
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
    if (refreshing) return;
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
    if (!item) return;
    try {
      await api.openItem(item.Key);
    } catch (e) {
      error = textError(e);
    }
  }
  async function ask(
    action: string,
    label: string,
    force = false,
    removeAfter = false,
  ) {
    if (!item || busy[item.Key]) return;
    if (item.Kind === 1 && action !== 'comment') {
      notice = t('Ação disponível apenas para pull/merge requests.');
      return;
    }
    confirmTarget = `${item.Repo} ${item.Ref}`;
    confirmWorktree = item.Worktree;
    command = {
      Key: item.Key,
      Action: action,
      Body: draft,
      Force: force,
      RemoveAfter: removeAfter,
    } as main.Request;
    mergeOptions = null;
    if (action === 'merge') {
      try {
        mergeOptions = await api.mergeOptions(item.Key);
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
    const target = view.Items.find((entry) => entry.Key === request.Key);
    confirmation.close();
    busy[request.Key] = true;
    error = '';
    notice = '';
    try {
      const result = await api.execute(request);
      setView(result.View);
      notice = result.Message;
      error = result.Error;
      if (result.ReloadThread && selected === request.Key) {
        draft = '';
        delete drafts[request.Key];
        await select(request.Key);
      }
      if (result.Dirty) {
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
    try {
      settings = await api.settings();
      settings.Instances ??= [];
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
    if (!settings) return;
    settingsBusy = true;
    try {
      setView(await api.saveSettings(settings));
      systemPalette = await api.theme();
      applyTheme(document.documentElement, preference(theme), systemPalette);
      settingsDialog.close();
      notice = translator(language)('Configurações salvas.');
    } catch (e) {
      error = textError(e);
    } finally {
      settingsBusy = false;
    }
  }
  async function pickThemeFile() {
    try {
      const path = await api.pickTheme();
      if (path && settings) settings.ThemeFile = path;
    } catch (e) {
      error = textError(e);
    }
  }
  async function exportColors() {
    try {
      const path = await api.exportTheme(
        snapshotTheme(document.documentElement, systemPalette),
      );
      if (path) notice = t('Esquema de cores exportado') + ': ' + path;
    } catch (e) {
      error = textError(e);
    }
  }
  async function diagnose() {
    settingsBusy = true;
    try {
      diagnostics = await api.diagnose();
    } catch (e) {
      error = textError(e);
    } finally {
      settingsBusy = false;
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
      class:spinning={refreshing}
      disabled={refreshing}
      title={t('Atualizar (r)')}
      aria-label={t('Atualizar')}
      onclick={refresh}>↻</button
    >
    <button
      class="icon-button"
      title={t('Configurações')}
      aria-label={t('Configurações')}
      onclick={openSettings}>⚙</button
    >
  </header>
  <div class="notifications">
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
        {t('mais recentes primeiro')}
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
    <div class="rows" aria-label={t('Lista de itens')}>
      {#if loading}<div class="empty">
          <div class="empty-symbol">↻</div>
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
      {:else}{#each visible as row (row.Key)}
          <button
            id={`row-${row.Key}`}
            class="item-row"
            class:selected={selected === row.Key}
            aria-pressed={selected === row.Key}
            onclick={() => select(row.Key)}
          >
            <span class="item-main"
              ><span class="repo-line"
                ><span class="branch-icon" class:draft={row.Draft}
                  >{row.Kind === 1 ? '◉' : row.Draft ? '▧' : '⑂'}</span
                >{row.Repo}<span class="ref">{row.Ref}</span
                >{#if row.Draft}<span class="pill">{t('Rascunho')}</span
                  >{/if}</span
              ><strong>{row.Title}</strong><span class="mobile-meta"
                >{row.Author} <span>·</span>
                {relativeAge(String(row.UpdatedAt), language)} <span>·</span>
                {row.Comments}
                {t('comentários')}</span
              ></span
            >
            <span class="author"
              ><span class="avatar">{row.Author.slice(0, 2).toUpperCase()}</span
              >{row.Author}</span
            >
            <span
              class="ci"
              class:success={row.CI === 'success'}
              class:failure={row.CI === 'failure'}
              title={row.CI ? statusLabel(language, row.CI) : t('Sem checks')}
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
        </div>
        <h2 class="detail-title">{item.Title}</h2>
        <p class="detail-meta">{item.Repo} <span>·</span> {item.Author}</p>
        {#if item.Kind === 0}<div class="branches">
            <code>{item.SourceBranch || '—'}</code><span>→</span><code
              >{item.TargetBranch || '—'}</code
            >
          </div>{/if}
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
        {#if detailLoading}<p class="muted" role="status">
            {t('Carregando conversa…')}
          </p>{/if}
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
            <button onclick={openBrowser}>{t('Abrir no navegador')}</button
            ><button
              disabled={!item.Clone && !item.Worktree}
              onclick={() => {
                if (item)
                  void api
                    .openFolder(item.Key)
                    .catch((e) => (error = textError(e)));
              }}>{t('Abrir pasta local')}</button
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
    </div>{/if}{#if command?.Action === 'comment'}<pre
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
      <button onclick={pickThemeFile}>{t('Selecionar arquivo')}</button>
      <button onclick={exportColors}>{t('Exportar esquema de cores')}</button>
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
        ><button
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
            /><button
              onclick={async () => {
                try {
                  const path = await api.pickFolder();
                  if (path) repo.Path = path;
                } catch (e) {
                  error = textError(e);
                }
              }}>{t('Selecionar pasta…')}</button
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
      >{settingsBusy ? t('Verificando…') : t('Executar diagnóstico')}</button
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
        >{settingsBusy ? t('Salvando…') : t('Salvar configurações')}</button
      >
    </div>{/if}
</dialog>
