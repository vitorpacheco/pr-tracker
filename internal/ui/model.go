// Package ui implements the pr-tracker terminal interface.
package ui

import (
	"context"
	"errors"
	"fmt"
	"slices"
	"sort"
	"strconv"
	"strings"
	"sync"
	"time"

	"charm.land/bubbles/v2/textarea"
	"charm.land/bubbles/v2/textinput"
	tea "charm.land/bubbletea/v2"

	"github.com/vitorpacheco/pr-tracker/internal/cache"
	"github.com/vitorpacheco/pr-tracker/internal/config"
	"github.com/vitorpacheco/pr-tracker/internal/gitops"
	"github.com/vitorpacheco/pr-tracker/internal/launch"
	"github.com/vitorpacheco/pr-tracker/internal/provider"
)

type screen int

const (
	screenPRs screen = iota
	screenInstances
	screenThread
)

type tabDef struct {
	label string
	kind  provider.Kind
	rel   provider.Relation // 0 = all of the kind
}

// firstIssueTab is the index of the first issues tab.
const firstIssueTab = 4

var tabs = []tabDef{
	{"Revisar", provider.KindPR, provider.ReviewRequested},
	{"Meus", provider.KindPR, provider.Authored},
	{"Atribuídos", provider.KindPR, provider.Assigned},
	{"Todos", provider.KindPR, 0},
	{"Atribuídas", provider.KindIssue, provider.Assigned},
	{"Criadas", provider.KindIssue, provider.Authored},
	{"Menções", provider.KindIssue, provider.Mentioned},
}

type modalKind int

const (
	modalNone modalKind = iota
	modalMenu
	modalConfirm
	modalForm
	modalHelp
	modalMessage
	modalCompose
)

type menuItem struct {
	key, label string
	disabled   string // reason, when not available
}

type confirm struct {
	title string
	body  []string
	yes   string
	onYes func() tea.Cmd
}

type statusKind int

const (
	stInfo statusKind = iota
	stOK
	stErr
)

// Model is the root Bubble Tea model.
type Model struct {
	cfg     *config.Config
	clients map[string]provider.Client
	cache   *cache.Store

	prs      []provider.Item
	instErr  map[string]error
	clones   map[string]string // PR key -> local clone path
	wts      map[string]string // PR key -> existing worktree path
	loading  bool
	gen      int
	lastSync time.Time
	nextSync time.Time
	frame    int

	screen     screen
	tab        int
	tabTouched bool
	cursor     int
	offset     int
	selected   string // PR key, kept across refreshes
	instCur    int
	filter     textinput.Model
	filtering  bool

	modal     modalKind
	menu      []menuItem
	menuCur   int
	menuTitle string
	confirm   *confirm
	form      *form
	message   []string

	thread     *threadView
	compose    *textarea.Model
	composeFor provider.Item

	pending map[string]string // PR key -> running action label

	status     string
	statusKind statusKind
	statusAt   time.Time
	cacheErr   error

	mode        launch.Mode
	afterSubmit tea.Cmd

	width, height int
	zones         zones
	listTop       int // screen row of the first list row
	listRows      int
}

// New creates the model.
func New(cfg *config.Config, store *cache.Store) *Model {
	fi := textinput.New()
	fi.Prompt = "/ "
	fi.Placeholder = "filtrar por título, repo, autor…"
	fi.SetWidth(40)
	m := &Model{
		cfg:     cfg,
		cache:   store,
		instErr: map[string]error{},
		clones:  map[string]string{},
		wts:     map[string]string{},
		pending: map[string]string{},
		filter:  fi,
		mode:    launch.Resolve(cfg.Terminal),
	}
	m.rebuildClients()
	return m
}

func (m *Model) rebuildClients() {
	m.clients = map[string]provider.Client{}
	for _, in := range m.cfg.Instances {
		m.clients[in.Name] = provider.New(in)
	}
}

// Messages.
type (
	clockMsg       struct{}
	autoRefreshMsg struct{ gen int }
	cacheMsg       struct {
		prs      map[string][]provider.Item
		syncedAt time.Time
		err      error
	}
	refreshMsg struct {
		gen       int
		prs       map[string][]provider.Item
		errs      map[string]error
		cacheErrs map[string]error
		clones    map[string]string
	}
	actionDoneMsg struct {
		key     string
		ok      string
		err     error
		refresh bool
		dirty   *provider.Item // worktree removal blocked by local changes
		// reloadThread reloads the open conversation of key.
		reloadThread bool
	}
	launchMsg struct {
		key, dir, label string
		argv            []string
		note            string
	}
	needPathMsg struct {
		pr     provider.Item
		action string
	}
	execDoneMsg struct{ err error }
)

func clock() tea.Cmd {
	return tea.Tick(time.Second, func(time.Time) tea.Msg { return clockMsg{} })
}

// Init loads the local snapshot before starting the first remote refresh.
func (m *Model) Init() tea.Cmd {
	return tea.Batch(clock(), tea.Sequence(m.loadCache(), m.refresh()))
}

func (m *Model) loadCache() tea.Cmd {
	store := m.cache
	instances := append([]config.Instance(nil), m.cfg.Instances...)
	return func() tea.Msg {
		msg := cacheMsg{prs: map[string][]provider.Item{}}
		if store == nil {
			return msg
		}
		ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
		defer cancel()
		var errs []error
		for _, in := range instances {
			if in.Disabled {
				continue
			}
			snapshot, err := store.Load(ctx, in)
			if err != nil {
				errs = append(errs, err)
				continue
			}
			if snapshot.SyncedAt.IsZero() {
				continue
			}
			msg.prs[in.Name] = snapshot.Items
			if snapshot.SyncedAt.After(msg.syncedAt) {
				msg.syncedAt = snapshot.SyncedAt
			}
		}
		msg.err = errors.Join(errs...)
		return msg
	}
}

func (m *Model) refresh() tea.Cmd {
	if m.loading {
		return nil
	}
	m.loading = true
	m.gen++
	gen := m.gen
	cfg := m.cfg
	store := m.cache
	var clients []provider.Client
	for _, in := range m.cfg.Instances {
		if !in.Disabled {
			clients = append(clients, m.clients[in.Name])
		}
	}
	return func() tea.Msg {
		ctx, cancel := context.WithTimeout(context.Background(), 2*time.Minute)
		defer cancel()
		res := refreshMsg{
			gen: gen, prs: map[string][]provider.Item{}, errs: map[string]error{},
			cacheErrs: map[string]error{}, clones: map[string]string{},
		}
		var mu sync.Mutex
		var wg sync.WaitGroup
		for _, c := range clients {
			wg.Go(func() {
				prs, err := c.List(ctx)
				var cacheErr error
				if err == nil && store != nil {
					cacheErr = store.Replace(ctx, c.Instance(), prs, time.Now())
				}
				clones := map[string]string{}
				for i := range prs {
					if p, _, ok := gitops.ResolveClone(ctx, cfg, &prs[i]); ok {
						clones[prs[i].Key()] = p
					}
				}
				mu.Lock()
				defer mu.Unlock()
				name := c.Instance().Name
				if err != nil {
					res.errs[name] = err
					return
				}
				res.prs[name] = prs
				if cacheErr != nil {
					res.cacheErrs[name] = cacheErr
				}
				for k, v := range clones {
					res.clones[k] = v
				}
			})
		}
		wg.Wait()
		return res
	}
}

func (m *Model) scheduleRefresh() tea.Cmd {
	gen := m.gen
	d := m.cfg.Interval()
	m.nextSync = time.Now().Add(d)
	return tea.Tick(d, func(time.Time) tea.Msg { return autoRefreshMsg{gen: gen} })
}

func (m *Model) applyRefresh(msg refreshMsg) {
	byInst := map[string][]provider.Item{}
	for _, pr := range m.prs {
		byInst[pr.Instance] = append(byInst[pr.Instance], pr)
	}
	m.instErr = map[string]error{}
	for _, in := range m.cfg.Instances {
		if in.Disabled {
			delete(byInst, in.Name)
			continue
		}
		if prs, ok := msg.prs[in.Name]; ok {
			byInst[in.Name] = prs
		}
		if err, ok := msg.errs[in.Name]; ok {
			m.instErr[in.Name] = err // keep the previous PRs of a failing instance
		}
	}
	m.prs = m.prs[:0]
	for _, in := range m.cfg.Instances {
		m.prs = append(m.prs, byInst[in.Name]...)
	}
	sort.SliceStable(m.prs, func(i, j int) bool { return m.prs[i].UpdatedAt.After(m.prs[j].UpdatedAt) })
	for k, v := range m.clones {
		if pr := m.find(k); pr != nil && msg.errs[pr.Instance] != nil {
			msg.clones[k] = v
		}
	}
	m.clones = msg.clones
	m.scanWorktrees()
	var cacheErrs []error
	for _, in := range m.cfg.Instances {
		if err := msg.cacheErrs[in.Name]; err != nil {
			cacheErrs = append(cacheErrs, err)
		}
	}
	m.cacheErr = errors.Join(cacheErrs...)
	if len(msg.prs) > 0 {
		m.lastSync = time.Now()
	}
	if !m.tabTouched && m.count(tabs[m.tab]) == 0 {
		for i, t := range tabs {
			if m.count(t) > 0 {
				m.tab = i
				break
			}
		}
	}
	m.restoreCursor()
}

func (m *Model) scanWorktrees() {
	m.wts = map[string]string{}
	for i := range m.prs {
		if p, ok := gitops.WorktreeExists(m.cfg, &m.prs[i]); ok {
			m.wts[m.prs[i].Key()] = p
		}
	}
}

// visible returns the PRs of the current tab matching the filter.
func (m *Model) visible() []*provider.Item {
	tab := tabs[m.tab]
	q := strings.ToLower(strings.TrimSpace(m.filter.Value()))
	var out []*provider.Item
	for i := range m.prs {
		pr := &m.prs[i]
		if !tab.match(pr) {
			continue
		}
		if q != "" {
			hay := strings.ToLower(pr.Title + " " + pr.Repo + " " + pr.Author + " " + pr.Ref() + " " + pr.SourceBranch + " " + pr.Instance + " " + strings.Join(pr.Labels, " "))
			if !strings.Contains(hay, q) {
				continue
			}
		}
		out = append(out, pr)
	}
	return out
}

func (t tabDef) match(it *provider.Item) bool {
	return it.Kind == t.kind && (t.rel == 0 || it.Relations&t.rel != 0)
}

func (m *Model) count(t tabDef) int {
	n := 0
	for i := range m.prs {
		if t.match(&m.prs[i]) {
			n++
		}
	}
	return n
}

func (m *Model) current() *provider.Item {
	v := m.visible()
	if m.cursor >= 0 && m.cursor < len(v) {
		return v[m.cursor]
	}
	return nil
}

func (m *Model) setCursor(i int) {
	n := len(m.visible())
	m.cursor = max(0, min(i, n-1))
	if pr := m.current(); pr != nil {
		m.selected = pr.Key()
	}
	m.clampOffset()
}

func (m *Model) restoreCursor() {
	for i, pr := range m.visible() {
		if pr.Key() == m.selected {
			m.cursor = i
			m.clampOffset()
			return
		}
	}
	m.setCursor(m.cursor)
}

func (m *Model) clampOffset() {
	rows := max(m.listRows, 1)
	if m.cursor < m.offset {
		m.offset = m.cursor
	}
	if m.cursor >= m.offset+rows {
		m.offset = m.cursor - rows + 1
	}
	m.offset = max(0, min(m.offset, max(0, len(m.visible())-rows)))
}

func (m *Model) setStatus(kind statusKind, s string) {
	m.status, m.statusKind, m.statusAt = oneLine(s), kind, time.Now()
}

func oneLine(s string) string { return strings.Join(strings.Fields(s), " ") }

// Update handles messages.
func (m *Model) Update(msg tea.Msg) (tea.Model, tea.Cmd) {
	switch msg := msg.(type) {
	case tea.WindowSizeMsg:
		m.width, m.height = msg.Width, msg.Height
		m.clampOffset()
		return m, nil

	case clockMsg:
		m.frame++
		if m.status != "" && m.statusKind != stErr && time.Since(m.statusAt) > 8*time.Second {
			m.status = ""
		}
		return m, clock()

	case autoRefreshMsg:
		if msg.gen != m.gen {
			return m, nil
		}
		return m, m.refresh()

	case cacheMsg:
		m.cacheErr = msg.err
		m.prs = m.prs[:0]
		for _, in := range m.cfg.Instances {
			m.prs = append(m.prs, msg.prs[in.Name]...)
		}
		sort.SliceStable(m.prs, func(i, j int) bool { return m.prs[i].UpdatedAt.After(m.prs[j].UpdatedAt) })
		m.lastSync = msg.syncedAt
		m.scanWorktrees()
		if !m.tabTouched && m.count(tabs[m.tab]) == 0 {
			for i, tab := range tabs {
				if m.count(tab) > 0 {
					m.tab = i
					break
				}
			}
		}
		m.restoreCursor()
		return m, nil

	case refreshMsg:
		m.loading = false
		if msg.gen != m.gen {
			return m, nil
		}
		m.applyRefresh(msg)
		return m, m.scheduleRefresh()

	case threadMsg:
		m.applyThread(msg)
		return m, nil

	case actionDoneMsg:
		delete(m.pending, msg.key)
		m.scanWorktrees()
		var cmds []tea.Cmd
		if msg.reloadThread && msg.err == nil && m.thread != nil && m.thread.item.Key() == msg.key {
			cmds = append(cmds, m.loadThread())
		}
		if msg.err != nil {
			m.showMessage("Erro", msg.err.Error())
			m.setStatus(stErr, msg.err.Error())
		} else if msg.ok != "" {
			m.setStatus(stOK, msg.ok)
		}
		if msg.dirty != nil {
			m.confirmForceRemove(*msg.dirty)
		}
		if msg.refresh {
			cmds = append(cmds, m.refresh())
		}
		return m, tea.Batch(cmds...)

	case needPathMsg:
		delete(m.pending, msg.pr.Key())
		m.openRepoPathForm(msg.pr, msg.action)
		return m, m.form.setFocus(0)

	case launchMsg:
		delete(m.pending, msg.key)
		if msg.note != "" {
			m.setStatus(stInfo, msg.note)
		}
		cmd, err := launch.Open(m.mode, msg.dir, msg.label, msg.argv)
		if err != nil {
			m.showMessage("Erro ao abrir", err.Error())
			return m, nil
		}
		if cmd != nil {
			return m, tea.ExecProcess(cmd, func(err error) tea.Msg { return execDoneMsg{err} })
		}
		m.setStatus(stOK, "aberto em nova aba ("+string(m.mode)+"): "+msg.label)
		return m, nil

	case execDoneMsg:
		m.scanWorktrees()
		if msg.err != nil {
			m.setStatus(stErr, msg.err.Error())
		}
		return m, nil

	case tea.MouseClickMsg:
		return m, m.click(msg.Mouse())

	case tea.MouseWheelMsg:
		mouse := msg.Mouse()
		if m.modal == modalMenu {
			if mouse.Button == tea.MouseWheelUp {
				m.menuCur = max(0, m.menuCur-1)
			} else if mouse.Button == tea.MouseWheelDown {
				m.menuCur = min(len(m.menu)-1, m.menuCur+1)
			}
			return m, nil
		}
		if m.modal != modalNone {
			return m, nil
		}
		delta := 0
		switch mouse.Button {
		case tea.MouseWheelUp:
			delta = -1
		case tea.MouseWheelDown:
			delta = 1
		}
		switch m.screen {
		case screenInstances:
			m.instCur = max(0, min(len(m.cfg.Instances)-1, m.instCur+delta))
		case screenThread:
			m.thread.offset += 3 * delta
		default:
			m.setCursor(m.cursor + delta)
		}
		return m, nil

	case tea.KeyPressMsg:
		return m, m.key(msg)
	}

	if m.modal == modalCompose && m.compose != nil {
		var cmd tea.Cmd
		*m.compose, cmd = m.compose.Update(msg)
		return m, cmd
	}
	if m.modal == modalForm && m.form != nil {
		// Forward non-key messages (cursor blink) to the focused input.
		fl := m.form.fields[m.form.focus]
		if fl.kind == fieldText {
			var cmd tea.Cmd
			fl.input, cmd = fl.input.Update(msg)
			return m, cmd
		}
	}
	if m.filtering {
		var cmd tea.Cmd
		m.filter, cmd = m.filter.Update(msg)
		return m, cmd
	}
	return m, nil
}

func (m *Model) key(msg tea.KeyPressMsg) tea.Cmd {
	k := msg.String()
	if k == "ctrl+c" {
		return tea.Quit
	}
	switch m.modal {
	case modalForm:
		res, cmd := m.form.update(msg)
		return tea.Batch(cmd, m.formResult(res))
	case modalCompose:
		return m.composeKey(msg)
	case modalNone:
	default:
		return m.modalKey(k)
	}
	if m.filtering {
		switch k {
		case "esc":
			m.filtering = false
			m.filter.Blur()
			m.filter.SetValue("")
			m.restoreCursor()
			return nil
		case "enter", "down", "up":
			m.filtering = false
			m.filter.Blur()
			return nil
		}
		var cmd tea.Cmd
		m.filter, cmd = m.filter.Update(msg)
		m.setCursor(0)
		return cmd
	}
	return m.press(k)
}

// press handles a shortcut; also used by clickable hints.
func (m *Model) press(k string) tea.Cmd {
	if m.screen == screenThread && m.thread != nil {
		if cmd, ok := m.threadKey(k); ok {
			return cmd
		}
	}
	switch k {
	case "q":
		if m.screen == screenInstances {
			m.screen = screenPRs
			return nil
		}
		return tea.Quit
	case "?":
		m.modal = modalHelp
		return nil
	case "r":
		if m.loading {
			return nil
		}
		m.setStatus(stInfo, "atualizando…")
		return m.refresh()
	case "i":
		m.thread = nil
		if m.screen == screenInstances {
			m.screen = screenPRs
		} else {
			m.screen = screenInstances
		}
		return nil
	case "s":
		m.openSettingsForm()
		return m.form.setFocus(0)
	}
	if m.screen == screenInstances {
		return m.instancesKey(k)
	}
	switch k {
	case "esc":
		if m.filter.Value() != "" {
			m.filter.SetValue("")
			m.restoreCursor()
		}
	case "/":
		m.filtering = true
		return m.filter.Focus()
	case "1", "2", "3", "4", "5", "6", "7":
		n, _ := strconv.Atoi(k)
		m.switchTab(n - 1)
	case "tab", "right", "l":
		m.switchTab((m.tab + 1) % len(tabs))
	case "shift+tab", "left", "h":
		m.switchTab((m.tab - 1 + len(tabs)) % len(tabs))
	case "up", "k":
		m.setCursor(m.cursor - 1)
	case "down", "j":
		m.setCursor(m.cursor + 1)
	case "pgup", "ctrl+u":
		m.setCursor(m.cursor - max(m.listRows-1, 1))
	case "pgdown", "ctrl+d":
		m.setCursor(m.cursor + max(m.listRows-1, 1))
	case "home", "g":
		m.setCursor(0)
	case "end", "G":
		m.setCursor(len(m.visible()) - 1)
	case "enter", "space":
		if pr := m.current(); pr != nil {
			m.openMenu(pr)
		}
	default:
		if pr := m.current(); pr != nil {
			return m.prAction(pr, k)
		}
	}
	return nil
}

func (m *Model) switchTab(i int) {
	m.tabTouched = true
	m.tab = i
	m.offset = 0
	m.restoreCursor()
}

func (m *Model) modalKey(k string) tea.Cmd {
	switch m.modal {
	case modalHelp, modalMessage:
		if k == "esc" || k == "enter" || k == "q" || k == "?" || k == "space" {
			m.modal = modalNone
		}
	case modalConfirm:
		switch k {
		case "y", "s", "enter":
			c := m.confirm
			m.modal, m.confirm = modalNone, nil
			return c.onYes()
		case "n", "esc", "q":
			m.modal, m.confirm = modalNone, nil
		}
	case modalMenu:
		switch k {
		case "esc", "q":
			m.modal = modalNone
		case "up", "k":
			m.menuCur = (m.menuCur - 1 + len(m.menu)) % len(m.menu)
		case "down", "j", "tab":
			m.menuCur = (m.menuCur + 1) % len(m.menu)
		case "enter", "space":
			return m.menuSelect(m.menuCur)
		default:
			for i, it := range m.menu {
				if it.key == k {
					return m.menuSelect(i)
				}
			}
		}
	}
	return nil
}

func (m *Model) menuSelect(i int) tea.Cmd {
	it := m.menu[i]
	if it.disabled != "" {
		m.setStatus(stErr, it.disabled)
		return nil
	}
	m.modal = modalNone
	if pr := m.current(); pr != nil {
		return m.prAction(pr, it.key)
	}
	return nil
}

func (m *Model) showMessage(title, body string) {
	m.modal = modalMessage
	m.message = append([]string{title}, strings.Split(strings.TrimSpace(body), "\n")...)
}

func (m *Model) ask(title, yes string, onYes func() tea.Cmd, body ...string) {
	m.modal = modalConfirm
	m.confirm = &confirm{title: title, body: body, yes: yes, onYes: onYes}
}

func (m *Model) formResult(res formResult) tea.Cmd {
	switch res {
	case formCancel:
		m.modal, m.form = modalNone, nil
	case formSubmit:
		f := m.form
		if err := f.submit(f); err != nil {
			f.err = oneLine(err.Error())
			return nil
		}
		// submit may have opened another modal or chained an action.
		if m.form == f {
			m.modal, m.form = modalNone, nil
		}
		cmd := m.afterSubmit
		m.afterSubmit = nil
		return cmd
	}
	return nil
}

func (m *Model) click(mouse tea.Mouse) tea.Cmd {
	if mouse.Button != tea.MouseLeft {
		return nil
	}
	id, ok := m.zones.hit(mouse.X, mouse.Y)
	if !ok {
		if m.modal == modalMenu || m.modal == modalHelp || m.modal == modalMessage {
			m.modal = modalNone
		}
		return nil
	}
	switch {
	case strings.HasPrefix(id, "key:"):
		k := strings.TrimPrefix(id, "key:")
		if m.modal != modalNone {
			return m.modalKey(k)
		}
		return m.press(k)
	case strings.HasPrefix(id, "tab:"):
		n, _ := strconv.Atoi(strings.TrimPrefix(id, "tab:"))
		m.screen, m.thread = screenPRs, nil
		m.switchTab(n)
	case strings.HasPrefix(id, "row:"):
		n, _ := strconv.Atoi(strings.TrimPrefix(id, "row:"))
		if n == m.cursor {
			if pr := m.current(); pr != nil {
				m.openMenu(pr)
			}
			return nil
		}
		m.setCursor(n)
	case strings.HasPrefix(id, "inst:"):
		n, _ := strconv.Atoi(strings.TrimPrefix(id, "inst:"))
		if n == m.instCur {
			return m.instancesKey("e")
		}
		m.instCur = n
	case strings.HasPrefix(id, "menu:"):
		n, _ := strconv.Atoi(strings.TrimPrefix(id, "menu:"))
		m.menuCur = n
		return m.menuSelect(n)
	case id == "confirm:yes":
		return m.modalKey("y")
	case id == "confirm:no":
		return m.modalKey("n")
	case id == "close":
		m.modal = modalNone
	case id == "compose:send":
		return m.sendComment()
	case id == "compose:cancel":
		m.modal, m.compose = modalNone, nil
	case id == "banner":
		m.screen = screenInstances
	case strings.HasPrefix(id, "form:"):
		if m.form == nil {
			return nil
		}
		idx := -1
		if strings.HasPrefix(id, "form:field:") {
			idx, _ = strconv.Atoi(strings.TrimPrefix(id, "form:field:"))
			id = "form:field"
		}
		res, cmd := m.form.click(id, idx)
		return tea.Batch(cmd, m.formResult(res))
	}
	return nil
}

// ---------- PR actions ----------

func (m *Model) openMenu(pr *provider.Item) {
	m.menuTitle = pr.Repo + " " + pr.Ref() + " — " + pr.Title
	m.menuCur = 0
	m.modal = modalMenu
	conversation := []menuItem{
		{key: "v", label: fmt.Sprintf("Ver conversa (%d comentários)", pr.Comments)},
		{key: "n", label: "Comentar"},
	}
	if pr.IsIssue() {
		m.menu = append(conversation, menuItem{key: "o", label: "Abrir no navegador"})
		return
	}
	_, hasWT := m.wts[pr.Key()]
	noWT := ""
	if !hasWT {
		noWT = "não há worktree para este PR"
	}
	client := m.clients[pr.Instance]
	noTool := ""
	if client != nil && !provider.ToolAvailable(client.Tool()) {
		noTool = client.Tool() + " não instalado"
	}
	wtLabel := "Checkout em worktree"
	if hasWT {
		wtLabel = "Atualizar worktree"
	}
	m.menu = append(conversation, []menuItem{
		{key: "w", label: wtLabel},
		{key: "c", label: "Checkout no clone local", disabled: noTool},
		{key: "d", label: "Ver diff (" + m.diffToolName() + ")"},
		{key: "t", label: "Abrir terminal no worktree (" + string(m.mode) + ")"},
		{key: "a", label: "Aprovar", disabled: noTool},
		{key: "A", label: "Aprovar e remover worktree", disabled: firstNonEmpty(noTool, noWT)},
		{key: "m", label: "Merge", disabled: noTool},
		{key: "M", label: "Merge e remover worktree", disabled: firstNonEmpty(noTool, noWT)},
		{key: "x", label: "Remover worktree (sem aprovar)", disabled: noWT},
		{key: "o", label: "Abrir no navegador"},
		{key: "p", label: "Definir pasta local do repositório"},
	}...)
}

func (m *Model) openBrowser(url string) {
	if err := launch.Browser(url); err != nil {
		m.setStatus(stErr, err.Error())
	}
}

func firstNonEmpty(s ...string) string {
	for _, v := range s {
		if v != "" {
			return v
		}
	}
	return ""
}

func (m *Model) diffToolName() string {
	if m.cfg.DiffTool == "hunk" && provider.ToolAvailable("hunk") {
		return "hunk"
	}
	return "git diff"
}

func (m *Model) prAction(pr *provider.Item, k string) tea.Cmd {
	if _, busy := m.pending[pr.Key()]; busy && k != "o" && k != "p" {
		m.setStatus(stInfo, "aguarde: "+m.pending[pr.Key()])
		return nil
	}
	client := m.clients[pr.Instance]
	if client == nil {
		return nil
	}
	switch k {
	case "v":
		return m.openThread(pr)
	case "n":
		return m.openCompose(pr)
	case "o":
		m.openBrowser(pr.URL)
		return nil
	}
	if pr.IsIssue() {
		if strings.Contains("wcdtaAmMxp", k) {
			m.setStatus(stInfo, "ação disponível apenas para pull/merge requests")
		}
		return nil
	}
	p := *pr // copy: m.prs is replaced on refresh
	key := p.Key()
	_, hasWT := m.wts[key]
	start := func(label string, fn func(ctx context.Context) tea.Msg) tea.Cmd {
		m.pending[key] = label
		m.setStatus(stInfo, label+" "+p.Repo+" "+p.Ref()+"…")
		return func() tea.Msg {
			ctx, cancel := context.WithTimeout(context.Background(), 5*time.Minute)
			defer cancel()
			return fn(ctx)
		}
	}
	cfg := m.cfg
	switch k {
	case "p":
		m.openRepoPathForm(p, "")
		return m.form.setFocus(0)
	case "w":
		return start("criando worktree", func(ctx context.Context) tea.Msg {
			wt, msg := ensureWorktree(ctx, cfg, client, &p, "w")
			if msg != nil {
				return msg
			}
			return actionDoneMsg{key: key, ok: "worktree pronto: " + wt}
		})
	case "t", "d":
		label := "preparando worktree"
		return start(label, func(ctx context.Context) tea.Msg {
			wt, msg := ensureWorktree(ctx, cfg, client, &p, k)
			if msg != nil {
				return msg
			}
			name := filepathBase(p.Repo) + "-" + strconv.Itoa(p.Number)
			if k == "t" {
				return launchMsg{key: key, dir: wt, label: name}
			}
			argv, note := diffCommand(ctx, cfg, wt, &p)
			return launchMsg{key: key, dir: wt, label: "diff " + name, argv: argv, note: note}
		})
	case "c":
		m.ask("Checkout no clone local?", "Checkout", func() tea.Cmd {
			return start("checkout", func(ctx context.Context) tea.Msg {
				clone, _, ok := gitops.ResolveClone(ctx, cfg, &p)
				if !ok {
					return needPathMsg{pr: p, action: "c"}
				}
				if err := client.Checkout(ctx, &p, clone); err != nil {
					return actionDoneMsg{key: key, err: err}
				}
				return actionDoneMsg{key: key, ok: "branch " + p.SourceBranch + " em " + clone}
			})
		}, "Troca a branch atual de "+firstNonEmpty(m.clones[key], "(clone não configurado)"), "para a branch do PR usando "+client.Tool()+".")
	case "a", "A":
		if k == "A" && !hasWT {
			return nil
		}
		title := "Aprovar " + p.Ref() + "?"
		if k == "A" {
			title = "Aprovar " + p.Ref() + " e remover o worktree?"
		}
		m.ask(title, "Aprovar", func() tea.Cmd {
			return start("aprovando", func(ctx context.Context) tea.Msg {
				if err := client.Approve(ctx, &p); err != nil {
					return actionDoneMsg{key: key, err: err}
				}
				return removeAfter(ctx, cfg, &p, k == "A", "aprovado "+p.Ref())
			})
		}, p.Repo, p.Title)
	case "m", "M":
		if k == "M" && !hasWT {
			return nil
		}
		opts := m.mergeOptions(&p)
		title := "Fazer merge de " + p.Ref() + "?"
		if k == "M" {
			title = "Fazer merge de " + p.Ref() + " e remover o worktree?"
		}
		body := []string{p.Repo, p.Title, "", "método: " + opts.Method}
		if opts.Auto {
			body = append(body, "auto-merge: sim (aguarda pipeline)")
		}
		if opts.DeleteBranch {
			body = append(body, "apagar branch de origem: sim")
		}
		if p.CI == provider.CIFailure {
			body = append(body, sRed.Render("atenção: pipeline falhou"))
		}
		m.ask(title, "Merge", func() tea.Cmd {
			return start("fazendo merge", func(ctx context.Context) tea.Msg {
				if err := client.Merge(ctx, &p, opts); err != nil {
					return actionDoneMsg{key: key, err: err}
				}
				return removeAfter(ctx, cfg, &p, k == "M", "merge de "+p.Ref()+" feito")
			})
		}, body...)
	case "x":
		if !hasWT {
			return nil
		}
		m.ask("Remover o worktree de "+p.Ref()+"?", "Remover", func() tea.Cmd {
			return start("removendo worktree", func(ctx context.Context) tea.Msg {
				return removeAfter(ctx, cfg, &p, true, "")
			})
		}, m.wts[key])
	}
	return nil
}

func (m *Model) mergeOptions(pr *provider.Item) provider.MergeOptions {
	in, _ := m.cfg.Instance(pr.Instance)
	opts := provider.MergeOptions{Method: "merge"}
	if in != nil {
		opts = provider.MergeOptions{Method: firstNonEmpty(in.MergeMethod, "merge"), Auto: in.AutoMerge, DeleteBranch: in.DeleteBranch}
	}
	if r, ok := m.cfg.Repo(pr.Instance, pr.Repo); ok && r.MergeMethod != "" {
		opts.Method = r.MergeMethod
	}
	return opts
}

func ensureWorktree(ctx context.Context, cfg *config.Config, client provider.Client, pr *provider.Item, action string) (string, tea.Msg) {
	if wt, ok := gitops.WorktreeExists(cfg, pr); ok && action != "w" {
		return wt, nil
	}
	clone, remote, ok := gitops.ResolveClone(ctx, cfg, pr)
	if !ok {
		return "", needPathMsg{pr: *pr, action: action}
	}
	wt, err := gitops.CreateWorktree(ctx, cfg, client, pr, clone, remote)
	if err != nil {
		return "", actionDoneMsg{key: pr.Key(), err: err}
	}
	return wt, nil
}

func removeAfter(ctx context.Context, cfg *config.Config, pr *provider.Item, remove bool, ok string) tea.Msg {
	if !remove {
		return actionDoneMsg{key: pr.Key(), ok: ok, refresh: true}
	}
	err := gitops.RemoveWorktree(ctx, cfg, pr, false)
	switch {
	case errors.Is(err, gitops.ErrDirty):
		return actionDoneMsg{key: pr.Key(), ok: ok, refresh: ok != "", dirty: pr}
	case err != nil:
		return actionDoneMsg{key: pr.Key(), err: err, refresh: ok != ""}
	}
	return actionDoneMsg{key: pr.Key(), ok: strings.TrimPrefix(ok+" · worktree removido", " · "), refresh: ok != ""}
}

func (m *Model) confirmForceRemove(pr provider.Item) {
	cfg := m.cfg
	key := pr.Key()
	m.ask("O worktree tem alterações locais", "Remover mesmo assim", func() tea.Cmd {
		m.pending[key] = "removendo worktree"
		return func() tea.Msg {
			ctx, cancel := context.WithTimeout(context.Background(), time.Minute)
			defer cancel()
			if err := gitops.RemoveWorktree(ctx, cfg, &pr, true); err != nil {
				return actionDoneMsg{key: key, err: err}
			}
			return actionDoneMsg{key: key, ok: "worktree removido (alterações descartadas)"}
		}
	}, m.wts[key], "", sRed.Render("As alterações não commitadas serão perdidas."))
}

func diffCommand(ctx context.Context, cfg *config.Config, wt string, pr *provider.Item) ([]string, string) {
	base := "HEAD~1"
	if up, err := gitops.Git(ctx, wt, "rev-parse", "--abbrev-ref", "@{upstream}"); err == nil || pr.TargetBranch != "" {
		remote := "origin"
		if i := strings.Index(up, "/"); err == nil && i > 0 {
			remote = up[:i]
		}
		if mb, err := gitops.Git(ctx, wt, "merge-base", remote+"/"+pr.TargetBranch, "HEAD"); err == nil {
			base = mb
		}
	}
	if cfg.DiffTool == "hunk" {
		if provider.ToolAvailable("hunk") {
			return []string{"hunk", "diff", base}, ""
		}
		return []string{"git", "diff", base}, "hunk não instalado (" + provider.InstallHint("hunk") + "); usando git diff"
	}
	return []string{"git", "diff", base}, ""
}

func filepathBase(p string) string {
	if i := strings.LastIndex(p, "/"); i >= 0 {
		return p[i+1:]
	}
	return p
}

// ---------- forms ----------

func (m *Model) openRepoPathForm(pr provider.Item, then string) {
	cur := m.clones[pr.Key()]
	remote := ""
	if r, ok := m.cfg.Repo(pr.Instance, pr.Repo); ok {
		cur, remote = r.Path, r.Remote
	}
	f := &form{
		title: "Pasta local de " + pr.Repo + " (" + pr.Instance + ")",
		fields: []*field{
			textField("Pasta do clone", cur, "~/code/"+filepathBase(pr.Repo), "caminho de um clone existente; vazio remove o mapeamento"),
			textField("Remote", remote, "detectar automaticamente", "remote que aponta para o repositório (ex.: origin, upstream)"),
		},
	}
	f.submit = func(f *form) error {
		path := f.get("Pasta do clone").value()
		if path != "" {
			if err := gitops.ValidateClone(context.Background(), path); err != nil {
				return err
			}
		}
		m.cfg.SetRepoPath(pr.Instance, pr.Repo, path)
		if r, ok := m.cfg.Repo(pr.Instance, pr.Repo); ok {
			r.Remote = f.get("Remote").value()
		}
		if err := m.cfg.Save(); err != nil {
			return err
		}
		if path == "" {
			delete(m.clones, pr.Key())
		} else {
			for i := range m.prs {
				if m.prs[i].Instance == pr.Instance && m.prs[i].Repo == pr.Repo {
					m.clones[m.prs[i].Key()] = config.ExpandHome(path)
				}
			}
		}
		m.setStatus(stOK, "pasta local salva para "+pr.Repo)
		if then != "" {
			m.modal, m.form = modalNone, nil
			if cur := m.find(pr.Key()); cur != nil {
				m.afterSubmit = m.prAction(cur, then)
			}
		}
		return nil
	}
	m.form = f
	m.modal = modalForm
}

func (m *Model) find(key string) *provider.Item {
	for i := range m.prs {
		if m.prs[i].Key() == key {
			return &m.prs[i]
		}
	}
	return nil
}

func (m *Model) openSettingsForm() {
	c := m.cfg
	wtDir, _ := c.Worktrees()
	f := &form{
		title: "Configurações — " + c.FilePath(),
		fields: []*field{
			textField("Atualização", c.RefreshInterval, "5m", "intervalo de atualização automática (ex.: 90s, 5m, 1h)"),
			choiceField("Terminal", []string{"auto", "herdr", "tmux", "inline"}, c.Terminal, "onde abrir terminal/diff; auto = herdr > tmux > inline"),
			choiceField("Diff", []string{"hunk", "git"}, c.DiffTool, "ferramenta de revisão do diff"),
			textField("Worktrees", firstNonEmpty(c.WorktreeDir, wtDir), "~/.pr-tracker/worktrees", "pasta onde os worktrees são criados"),
			textField("Clone roots", strings.Join(c.CloneRoots, ", "), "~/code, ~/work", "pastas onde procurar clones automaticamente (separadas por vírgula)"),
		},
	}
	f.submit = func(f *form) error {
		next := *c
		next.RefreshInterval = f.get("Atualização").value()
		next.Terminal = f.get("Terminal").value()
		next.DiffTool = f.get("Diff").value()
		next.WorktreeDir = f.get("Worktrees").value()
		if next.WorktreeDir == wtDir {
			next.WorktreeDir = c.WorktreeDir
		}
		next.CloneRoots = nil
		for _, r := range strings.Split(f.get("Clone roots").value(), ",") {
			if r = strings.TrimSpace(r); r != "" {
				next.CloneRoots = append(next.CloneRoots, r)
			}
		}
		if err := next.Validate(); err != nil {
			return err
		}
		*c = next
		if err := c.Save(); err != nil {
			return err
		}
		m.mode = launch.Resolve(c.Terminal)
		m.setStatus(stOK, "configurações salvas")
		m.afterSubmit = m.refresh()
		return nil
	}
	m.form = f
	m.modal = modalForm
}

// ---------- instances screen ----------

func (m *Model) instancesKey(k string) tea.Cmd {
	n := len(m.cfg.Instances)
	switch k {
	case "esc":
		m.screen = screenPRs
	case "up", "k":
		m.instCur = max(0, m.instCur-1)
	case "down", "j":
		m.instCur = min(n-1, m.instCur+1)
	case "n", "a":
		m.openInstanceForm(nil)
		return m.form.setFocus(0)
	case "e", "enter":
		if m.instCur < n {
			in := m.cfg.Instances[m.instCur]
			m.openInstanceForm(&in)
			return m.form.setFocus(0)
		}
	case "space":
		if m.instCur < n {
			in := &m.cfg.Instances[m.instCur]
			in.Disabled = !in.Disabled
			if err := m.cfg.Save(); err != nil {
				m.setStatus(stErr, err.Error())
			}
			m.rebuildClients()
			return m.refresh()
		}
	case "D", "delete":
		if m.instCur < n {
			name := m.cfg.Instances[m.instCur].Name
			m.ask("Remover a instância "+name+"?", "Remover", func() tea.Cmd {
				m.cfg.RemoveInstance(name)
				if err := m.cfg.Save(); err != nil {
					m.setStatus(stErr, err.Error())
				}
				m.instCur = max(0, min(m.instCur, len(m.cfg.Instances)-1))
				m.rebuildClients()
				m.prs = slices.DeleteFunc(m.prs, func(p provider.Item) bool { return p.Instance == name })
				return m.refresh()
			}, "Os mapeamentos de pastas locais desta instância também serão removidos.")
		}
	case "t":
		if m.instCur < n {
			c := provider.New(m.cfg.Instances[m.instCur])
			m.setStatus(stInfo, "verificando autenticação de "+c.Instance().Name+"…")
			return func() tea.Msg {
				ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
				defer cancel()
				if err := c.AuthStatus(ctx); err != nil {
					return actionDoneMsg{err: err}
				}
				return actionDoneMsg{ok: c.Tool() + " autenticado em " + c.Instance().Host}
			}
		}
	}
	return nil
}

func (m *Model) openInstanceForm(in *config.Instance) {
	old := ""
	cur := config.Instance{Provider: config.GitHub, MergeMethod: "merge"}
	if in != nil {
		old, cur = in.Name, *in
	}
	providers := []string{string(config.GitHub), string(config.GitLab), string(config.Bitbucket)}
	title := "Nova instância"
	if old != "" {
		title = "Editar instância " + old
	}
	f := &form{
		title: title,
		fields: []*field{
			choiceField("Provider", providers, string(cur.Provider), "github (gh) · gitlab (glab) · bitbucket (ainda não suportado)"),
			textField("Host", cur.Host, "github.com, gitlab.empresa.com…", "host da instância; vazio usa o SaaS do provider"),
			textField("Nome", cur.Name, "igual ao host", "identificador único da instância"),
			choiceField("Merge", []string{"merge", "squash", "rebase"}, firstNonEmpty(cur.MergeMethod, "merge"), "método de merge padrão"),
			boolField("Auto-merge", cur.AutoMerge, "habilita merge automático quando o pipeline passar"),
			boolField("Apagar branch", cur.DeleteBranch, "apaga a branch de origem após o merge"),
			boolField("Desativada", cur.Disabled, "mantém a instância mas não consulta"),
		},
	}
	f.submit = func(f *form) error {
		next := config.Instance{
			Name:         f.get("Nome").value(),
			Provider:     config.Provider(f.get("Provider").value()),
			Host:         f.get("Host").value(),
			MergeMethod:  f.get("Merge").value(),
			AutoMerge:    f.get("Auto-merge").on,
			DeleteBranch: f.get("Apagar branch").on,
			Disabled:     f.get("Desativada").on,
		}
		backup := slices.Clone(m.cfg.Instances)
		backupRepos := slices.Clone(m.cfg.Repos)
		if err := m.cfg.UpsertInstance(old, next); err != nil {
			m.cfg.Instances, m.cfg.Repos = backup, backupRepos
			return err
		}
		if err := m.cfg.Save(); err != nil {
			return err
		}
		m.rebuildClients()
		tool := provider.New(next).Tool()
		if next.Provider == config.Bitbucket {
			m.setStatus(stErr, "Bitbucket salvo, mas a integração ainda não existe (veja docs/bitbucket.md)")
		} else if !provider.ToolAvailable(tool) {
			m.setStatus(stErr, tool+" não instalado. Instale: "+provider.InstallHint(tool))
		} else {
			m.setStatus(stOK, "instância salva")
		}
		if old != "" && old != next.Name {
			for i := range m.prs {
				if m.prs[i].Instance == old {
					m.prs[i].Instance = next.Name
				}
			}
		}
		m.afterSubmit = m.refresh()
		return nil
	}
	m.form = f
	m.modal = modalForm
}

func itoa(i int) string { return strconv.Itoa(i) }
