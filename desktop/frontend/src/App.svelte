<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { marked } from 'marked';
  import DOMPurify from 'dompurify';
  import { api, demoMode } from './lib/api';
  import type { View } from './lib/list';
  import { filterItems, nextSelection, relativeAge } from './lib/list';
  import type { main, provider, config, app } from '../wailsjs/go/models';
  let view: View = {
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
  const isMac =
    typeof navigator !== 'undefined' && /Mac/.test(navigator.platform);
  const modifier = isMac ? '⌘' : 'Ctrl';
  const prTabs = [
    ['Todos', 0],
    ['Revisar', 1],
    ['Meus', 2],
    ['Atribuídos', 4],
  ] as const;
  const issueTabs = [
    ['Todas', 0],
    ['Atribuídas', 4],
    ['Criadas', 2],
    ['Menções', 8],
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
    { label: 'Atualizar agora', key: 'r', run: () => refresh() },
    { label: 'Buscar itens', key: '/', run: () => search?.focus() },
    {
      label: 'Ver conversa',
      key: 'v',
      run: () => {
        detailTab = 'conversation';
        detailOpen = true;
      },
    },
    {
      label: 'Comentar',
      key: 'n',
      run: () => {
        detailTab = 'conversation';
        detailOpen = true;
        void tick().then(() => document.getElementById('comment')?.focus());
      },
    },
    { label: 'Abrir no navegador', key: 'o', run: () => openBrowser() },
    {
      label: 'Criar / atualizar worktree',
      key: 'w',
      run: () => ask('update_worktree', 'Criar ou atualizar worktree'),
    },
    {
      label: 'Abrir terminal',
      key: 't',
      run: () => ask('prepare_terminal', 'Abrir terminal no worktree'),
    },
    {
      label: 'Abrir diff',
      key: 'd',
      run: () => ask('prepare_diff', 'Abrir diff no terminal'),
    },
    {
      label: 'Aprovar',
      key: 'a',
      run: () => ask('approve', 'Aprovar pull request'),
    },
    { label: 'Merge', key: 'm', run: () => ask('merge', 'Fazer merge') },
    { label: 'Configurações', key: ',', run: () => openSettings() },
  ].filter((c) =>
    c.label.toLocaleLowerCase().includes(paletteQuery.toLocaleLowerCase()),
  );
  const textError = (e: unknown) =>
    e instanceof Error ? e.message : String(e);
  const markdown = (value: string) =>
    DOMPurify.sanitize(marked.parse(value, { async: false }) as string);
  function setView(next: View) {
    if (next.Revision < view.Revision) return;
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
      notice = 'Ação disponível apenas para pull/merge requests.';
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
        confirmLabel = 'Descartar alterações locais e remover worktree';
        confirmation.showModal();
      }
      if (result.NeedsClone) {
        await openSettings();
        notice =
          'Defina a pasta local do repositório nas configurações e repita a ação.';
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
      settingsDialog.close();
      notice = 'Configurações salvas.';
    } catch (e) {
      error = textError(e);
    } finally {
      settingsBusy = false;
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
    theme = value;
    document.documentElement.dataset.theme = value;
    try {
      localStorage.setItem('pr-tracker-theme', value);
    } catch {
      /* storage can be disabled */
    }
  }
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
      detailOpen = false;
      (document.activeElement as HTMLElement)?.blur();
      return;
    }
    if (
      e.target instanceof HTMLElement &&
      e.target.closest('input,textarea,select,[contenteditable="true"]')
    )
      return;
    if (e.ctrlKey || e.metaKey || e.altKey) return;
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
      ask('approve', 'Aprovar pull request');
    } else if (e.key === 'm') {
      ask('merge', 'Fazer merge');
    } else if (e.key === 't') {
      void ask('prepare_terminal', 'Abrir terminal no worktree');
    } else if (e.key === 'd') {
      void ask('prepare_diff', 'Abrir diff no terminal');
    } else if (e.key === 'w') {
      ask('update_worktree', 'Criar ou atualizar worktree');
    }
  }
  onMount(() => {
    mounted = true;
    try {
      changeTheme(localStorage.getItem('pr-tracker-theme') ?? 'system');
      drafts = JSON.parse(localStorage.getItem('pr-tracker-drafts') ?? '{}');
    } catch {
      drafts = {};
    }
    let stopped = false;
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
      saveDrafts();
      window.removeEventListener('beforeunload', saveDrafts);
    };
  });
</script>

<svelte:window onkeydown={keydown} />
<div class="shell" class:route-detail={detailOpen}>
  <header class="topbar">
    <div class="brand">
      <span class="brand-icon">⑂</span> pr-tracker
      <span class="desktop-label">DESKTOP</span>
    </div>
    <nav class="kind-tabs" aria-label="Tipo de item">
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
        placeholder="Buscar título, repo, autor…"
        aria-label="Buscar itens"
      /><kbd>/</kbd></label
    >
    <button
      class="icon-button"
      title={`Comandos (${modifier}+K)`}
      aria-label="Abrir comandos"
      onclick={() => {
        paletteQuery = '';
        palette.showModal();
      }}>⌘</button
    >
    <button
      class="icon-button"
      class:spinning={refreshing}
      disabled={refreshing}
      title="Atualizar (r)"
      aria-label="Atualizar"
      onclick={refresh}>↻</button
    >
    <button
      class="icon-button"
      title="Configurações"
      aria-label="Configurações"
      onclick={openSettings}>⚙</button
    >
  </header>
  <div class="notifications">
    {#if error}<div class="banner error" role="alert">
        {error}<button aria-label="Fechar erro" onclick={() => (error = '')}
          >×</button
        >
      </div>{/if}
    {#if notice}<div class="banner" role="status">
        {notice}<button aria-label="Fechar aviso" onclick={() => (notice = '')}
          >×</button
        >
      </div>{/if}
  </div>
  <aside class="sidebar">
    <div class="eyebrow">WORKSPACE</div>
    <h2>Seu trabalho,<br />em um lugar.</h2>
    <button class:chosen={!instance} onclick={() => (instance = '')}
      ><span>◈ &nbsp; Todas as instâncias</span><span class="count"
        >{view.Items.length}</span
      ></button
    >
    <div class="eyebrow section-label">INSTÂNCIAS</div>
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
    <button class="subtle" onclick={openSettings}>＋ Adicionar instância</button
    >
    <div class="eyebrow section-label">VISÕES</div>
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
        ? 'Demonstração · dados fictícios'
        : 'Cache local · SQLite'}<small
        >{view.SyncedAt && !String(view.SyncedAt).startsWith('0001')
          ? `Sincronizado ${relativeAge(String(view.SyncedAt))}`
          : 'Aguardando sincronização'}</small
      >
    </div>
  </aside>
  <main class="list-pane" bind:this={list}>
    <div class="list-heading">
      <div class="eyebrow">SUA FILA DE TRABALHO</div>
      <h1>
        {kind === 0 ? 'Pull requests' : 'Issues'}<span class="total"
          >{visible.length}</span
        >
      </h1>
      <p>
        {instance || 'Todas as instâncias'} <span>·</span> mais recentes primeiro
      </p>
    </div>
    {#if demoMode}<div class="banner demo">
        Demonstração visual — nenhuma ação altera dados.
      </div>{/if}
    {#if view.CacheError}<div class="banner error">
        Cache indisponível: {view.CacheError}
      </div>{/if}
    {#each Object.entries(view.Errors) as [name, message]}<div
        class="banner error"
      >
        <strong>{name}</strong>: {message} · exibindo dados anteriores.
      </div>{/each}
    <nav class="filters" aria-label="Filtrar por relação">
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
      <span>REPOSITÓRIO / TÍTULO</span><span>AUTOR</span><span>CI</span><span
        >ATUALIZADO ↓</span
      >
    </div>
    <div class="rows" aria-label="Lista de itens">
      {#if loading}<div class="empty">
          <div class="empty-symbol">↻</div>
          <h2>Carregando sua fila</h2>
          <p>Buscando primeiro os dados do cache local.</p>
        </div>
      {:else if !visible.length}<div class="empty">
          <div class="empty-symbol">✓</div>
          <h2>{query ? 'Nenhum resultado' : 'Tudo em dia por aqui'}</h2>
          <p>
            {view.Items.length
              ? 'Experimente outra visão ou busca.'
              : 'Configure uma instância ou atualize para buscar seus PRs e issues.'}
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
              ? 'Limpar filtros'
              : 'Configurar instâncias'}</button
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
                >{#if row.Draft}<span class="pill">Rascunho</span>{/if}</span
              ><strong>{row.Title}</strong><span class="mobile-meta"
                >{row.Author} <span>·</span>
                {relativeAge(String(row.UpdatedAt))} <span>·</span>
                {row.Comments} comentários</span
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
              title={row.CI || 'Sem checks'}
              >{row.CI === 'success'
                ? '✓'
                : row.CI === 'failure'
                  ? '×'
                  : row.CI === 'pending'
                    ? '◷'
                    : '—'}</span
            >
            <span class="updated"
              >{relativeAge(String(row.UpdatedAt))}<small
                >◯ {row.Comments}</small
              ></span
            >
          </button>
        {/each}{/if}
    </div>
    <footer class="list-footer">
      <span><kbd>j</kbd><kbd>k</kbd> navegar <kbd>Enter</kbd> abrir</span
      ><button onclick={() => palette.showModal()}
        >{modifier}+K <span>comandos</span></button
      >
    </footer>
  </main>
  <section class="inspector" aria-label="Detalhes do item">
    <div class="inspector-heading">
      <span>INSPECTOR</span><button
        class="back"
        onclick={() => (detailOpen = false)}>← Voltar</button
      >
    </div>
    {#if item}
      <div class="detail-scroll">
        <div class="detail-reference">
          <span class="branch-icon">⑂</span>
          {item.Ref}<span class="pill green"
            >{item.Draft ? 'Rascunho' : 'Aberto'}</span
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
        <nav class="detail-tabs" aria-label="Conteúdo do detalhe">
          <button
            class:active={detailTab === 'details'}
            onclick={() => (detailTab = 'details')}>Detalhes</button
          ><button
            class:active={detailTab === 'checks'}
            onclick={() => (detailTab = 'checks')}
            >Checks <span>{item.Checks?.length ?? 0}</span></button
          ><button
            class:active={detailTab === 'conversation'}
            onclick={() => (detailTab = 'conversation')}
            >Conversa <span>{item.Comments}</span></button
          >
        </nav>
        {#if detailLoading}<p class="muted" role="status">
            Carregando conversa…
          </p>{/if}
        {#if detailError}<div class="banner error">
            {detailError}<button onclick={() => select(selected)}
              >Tentar novamente</button
            >
          </div>{/if}
        {#if detailTab === 'details'}
          {#if thread}<article class="markdown">
              {@html markdown(thread.Body || '_Sem descrição._')}
            </article>{/if}
          {#if item.Kind === 0}<div class="detail-section">
              <h3>Revisões e aprovações</h3>
              <p
                class="review-status"
                class:success={item.Review === 'approved'}
              >
                {item.Review === 'approved'
                  ? '✓ Aprovado'
                  : item.Review === 'changes_requested'
                    ? 'Alterações solicitadas'
                    : 'Aguardando revisão'}
              </p>
              <p class="muted">
                {(item.ApprovedBy ?? []).join(', ') ||
                  'Nenhuma aprovação informada'}
              </p>
              {#if item.Conflicts}<div class="banner error">
                  Conflitos precisam ser resolvidos antes do merge.
                </div>{/if}
            </div>
            <div class="detail-section">
              <h3>
                Arquivos alterados <span class="count">{item.Files}</span>
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
                {item.Worktree || 'Nenhum worktree local'}
              </p>
              <button
                class="secondary"
                disabled={!!busy[selected]}
                onclick={() =>
                  ask(
                    'update_worktree',
                    item?.Worktree ? 'Atualizar worktree' : 'Criar worktree',
                  )}
                >{item.Worktree ? 'Atualizar worktree' : 'Criar worktree'}
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
                  >{check.State}</span
                >
              </div>{:else}<p class="muted">
                Nenhum check informado pelo provider.
              </p>{/each}
          </div>
        {:else}
          {#if thread}<article class="markdown">
              {@html markdown(thread.Body || '_Sem descrição._')}
            </article>
            {#each thread.Comments ?? [] as comment}<article class="comment">
                <header>
                  <strong>{comment.Author}</strong><small
                    >{relativeAge(String(comment.CreatedAt))}</small
                  >
                </header>
                {#if comment.Review}<span class="pill">{comment.Review}</span
                  >{/if}{#if comment.Path}<p class="path muted">
                    {comment.Path}:{comment.Line}
                  </p>{/if}
                <div class="markdown">{@html markdown(comment.Body)}</div>
              </article>{/each}{/if}
          <label class="composer"
            >Adicionar comentário<textarea
              id="comment"
              bind:value={draft}
              placeholder="Escreva em Markdown…"
              maxlength="65536"
              rows="5"></textarea></label
          >
          <div class="composer-footer">
            <small class="muted">Rascunho salvo neste dispositivo</small><button
              class="primary"
              disabled={!draft.trim() || !!busy[selected]}
              onclick={() => ask('comment', 'Enviar comentário')}
              >Revisar envio →</button
            >
          </div>
        {/if}
      </div>
      <div class="detail-actions">
        {#if item.Kind === 0}<button
            class="approve"
            disabled={!!busy[selected]}
            onclick={() => ask('approve', 'Aprovar pull request')}
            >✓ Aprovar</button
          ><button
            class="primary"
            disabled={!!busy[selected]}
            onclick={() => ask('merge', 'Fazer merge')}>Merge</button
          >{/if}
        <details class="more">
          <summary>Mais ações</summary>
          <div>
            <button onclick={openBrowser}>Abrir no navegador</button><button
              disabled={!item.Clone && !item.Worktree}
              onclick={() => {
                if (item)
                  void api
                    .openFolder(item.Key)
                    .catch((e) => (error = textError(e)));
              }}>Abrir pasta local</button
            >{#if item.Kind === 0}<button
                disabled={!!busy[selected]}
                onclick={() =>
                  ask('prepare_terminal', 'Abrir terminal no worktree')}
                >Abrir terminal</button
              ><button
                disabled={!!busy[selected]}
                onclick={() => ask('prepare_diff', 'Abrir diff no terminal')}
                >Ver diff</button
              ><button
                disabled={!!busy[selected]}
                onclick={() => ask('checkout', 'Trocar branch do clone local')}
                >Checkout no clone</button
              ><button
                disabled={!!busy[selected]}
                onclick={() => ask('close', 'Fechar sem merge')}
                >Fechar sem merge</button
              >{#if item.Worktree}<button
                  disabled={!!busy[selected]}
                  onclick={() =>
                    ask('approve', 'Aprovar e remover worktree', false, true)}
                  >Aprovar e remover worktree</button
                ><button
                  disabled={!!busy[selected]}
                  onclick={() =>
                    ask('merge', 'Merge e remover worktree', false, true)}
                  >Merge e remover worktree</button
                ><button
                  disabled={!!busy[selected]}
                  onclick={() =>
                    ask(
                      'close',
                      'Fechar sem merge e remover worktree',
                      false,
                      true,
                    )}>Fechar e remover worktree</button
                >
                <button
                  disabled={!!busy[selected]}
                  onclick={() => ask('remove_worktree', 'Remover worktree')}
                  >Remover worktree</button
                >{/if}{/if}
          </div>
        </details>
      </div>
    {:else}<div class="empty">
        <div class="empty-symbol">⑂</div>
        <h2>Um pouco de contexto</h2>
        <p>Selecione um item para ver detalhes, checks e conversa.</p>
      </div>{/if}
  </section>
</div>

<dialog bind:this={palette} class="palette" aria-label="Comandos">
  <form method="dialog">
    <button class="dialog-close" aria-label="Fechar comandos">×</button>
  </form>
  <label class="palette-search"
    >⌕ <input
      bind:value={paletteQuery}
      placeholder="Digite uma ação…"
      aria-label="Buscar comando"
    /></label
  >
  <div class="command-list">
    {#each commands as cmd}<button
        onclick={() => {
          palette.close();
          cmd.run();
        }}><span>{cmd.label}</span><kbd>{cmd.key}</kbd></button
      >{:else}<p>Nenhum comando encontrado.</p>{/each}
  </div>
  <small class="muted">{modifier}+K abre comandos · Esc fecha</small>
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
      As alterações não commitadas serão perdidas. Esta operação não pode ser
      desfeita.
    </div>{/if}{#if command?.Action === 'comment'}<pre
      class="comment-preview">{command.Body}</pre>{/if}{#if command?.Action === 'merge'}<p
    >
      Método: <strong>{mergeOptions?.Method}</strong><br />Auto-merge: {mergeOptions?.Auto
        ? 'sim'
        : 'não'}<br />Apagar branch de origem: {mergeOptions?.DeleteBranch
        ? 'sim'
        : 'não'}
    </p>{/if}
  <div class="dialog-actions">
    <button onclick={() => confirmation.close()}>Cancelar</button><button
      class:danger={command?.Force ||
        command?.Action === 'close' ||
        command?.Action === 'remove_worktree'}
      class="primary"
      onclick={execute}>Confirmar</button
    >
  </div>
</dialog>
<dialog
  bind:this={settingsDialog}
  class="settings-dialog"
  aria-labelledby="settings-title"
>
  <form method="dialog">
    <button class="dialog-close" aria-label="Fechar configurações">×</button>
  </form>
  <div class="eyebrow">PREFERÊNCIAS</div>
  <h2 id="settings-title">Configurações</h2>
  {#if error}<div class="banner error" role="alert">{error}</div>{/if}<label
    >Tema<select
      value={theme}
      onchange={(e) => changeTheme(e.currentTarget.value)}
      ><option value="system">Sistema</option><option value="dark"
        >Escuro</option
      ><option value="light">Claro</option></select
    ></label
  >
  {#if settings}<div class="settings-grid">
      <label
        >Intervalo de atualização<input
          bind:value={settings.RefreshInterval}
          placeholder="5m"
        /></label
      ><label
        >Pasta de worktrees<input
          bind:value={settings.WorktreeDir}
          placeholder="Padrão da aplicação"
        /></label
      >
    </div>
    <label
      >Terminal desktop<input
        bind:value={settings.DesktopTerminal}
        placeholder="Detectar automaticamente (kitty, foot, wezterm…)"
      /></label
    >
    <details>
      <summary>Caminhos das ferramentas</summary
      >{#each ['git', 'gh', 'glab', 'tea', 'hunk'] as tool}<label
          >{tool}<input
            bind:value={settings.ToolPaths[tool]}
            placeholder="Detectar no PATH"
          /></label
        >{/each}
    </details>
    <h3>Instâncias</h3>
    {#each settings.Instances as inst, i}<fieldset>
        <legend>Instância {i + 1}</legend>
        <div class="settings-grid">
          <label>Nome<input bind:value={inst.Name} /></label><label
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
            >Método de merge<select bind:value={inst.MergeMethod}
              ><option value="">Padrão (merge)</option><option value="merge"
                >Merge</option
              ><option value="squash">Squash</option><option value="rebase"
                >Rebase</option
              ></select
            ></label
          >
        </div>
        <label class="checkbox"
          ><input
            type="checkbox"
            bind:checked={inst.Disabled}
          />Desativada</label
        ><label class="checkbox"
          ><input type="checkbox" bind:checked={inst.AutoMerge} />Auto-merge
          quando o pipeline passar</label
        ><label class="checkbox"
          ><input type="checkbox" bind:checked={inst.DeleteBranch} />Apagar
          branch de origem após merge</label
        ><button
          class="text-danger"
          onclick={() => {
            if (settings) {
              settings.Repos = settings.Repos.filter(
                (repo) => repo.Instance !== inst.Name,
              );
              settings.Instances = settings.Instances.filter((_, n) => n !== i);
            }
          }}>Remover da configuração</button
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
      }}>＋ Adicionar instância</button
    >
    <h3>Repositórios locais</h3>
    {#each settings.Repos as repo, i}<fieldset>
        <div class="settings-grid">
          <label
            >Instância<select bind:value={repo.Instance}
              >{#each settings.Instances as inst}<option value={inst.Name}
                  >{inst.Name}</option
                >{/each}</select
            ></label
          ><label
            >Repositório<input
              bind:value={repo.Name}
              placeholder="organização/repo"
            /></label
          ><label
            >Pasta do clone<input
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
              }}>Selecionar pasta…</button
            ></label
          ><label
            >Remote<input
              bind:value={repo.Remote}
              placeholder="origin"
            /></label
          >
        </div>
        <label class="checkbox"
          ><input type="checkbox" bind:checked={repo.TrackAll} />Acompanhar
          todos os PRs</label
        ><button
          class="text-danger"
          onclick={() => {
            if (settings)
              settings.Repos = settings.Repos.filter((_, n) => n !== i);
          }}>Remover mapeamento</button
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
      }}>＋ Mapear repositório</button
    >
    <h3>Ferramentas e autenticação</h3>
    <button disabled={settingsBusy} onclick={diagnose}
      >{settingsBusy ? 'Verificando…' : 'Executar diagnóstico'}</button
    >{#if diagnostics}{#each diagnostics.Tools as tool}<p class="diagnostic">
          <span class:success={!!tool.Path} class:failure={!tool.Path}
            >{tool.Path ? '✓' : '×'}</span
          ><strong>{tool.Name}</strong><span
            >{tool.Path || 'Não encontrada no PATH'}</span
          >
        </p>{/each}{#each diagnostics.Instances as inst}<p class="diagnostic">
          {inst.Name}: {inst.Disabled
            ? 'desativada'
            : inst.Error || 'autenticada'}
        </p>{/each}{/if}
    <div class="dialog-actions">
      <button onclick={() => settingsDialog.close()}>Cancelar</button><button
        class="primary"
        disabled={settingsBusy}
        onclick={saveSettings}
        >{settingsBusy ? 'Salvando…' : 'Salvar configurações'}</button
      >
    </div>{/if}
</dialog>
