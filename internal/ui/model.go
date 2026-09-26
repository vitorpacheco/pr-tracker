// Package ui implements the pr-tracker terminal interface.
package ui

import (
	"context"
	"errors"
	"fmt"
	"slices"
	"strconv"
	"strings"
	"time"

	"charm.land/bubbles/v2/textarea"
	"charm.land/bubbles/v2/textinput"
	tea "charm.land/bubbletea/v2"

	"github.com/vitorpacheco/pr-tracker/internal/app"
	"github.com/vitorpacheco/pr-tracker/internal/cache"
	"github.com/vitorpacheco/pr-tracker/internal/config"
	"github.com/vitorpacheco/pr-tracker/internal/gitops"
	"github.com/vitorpacheco/pr-tracker/internal/i18n"
	"github.com/vitorpacheco/pr-tracker/internal/launch"
	"github.com/vitorpacheco/pr-tracker/internal/provider"
	"github.com/vitorpacheco/pr-tracker/internal/toolchain"
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
	cfg      *config.Config
	language string
	clients  map[string]provider.Client
	session  *app.Session

	prs     []provider.Item
	instErr map[string]error
	clones  map[string]string // PR key -> local clone path
	wts     map[string]string // PR key -> existing worktree path
	loading bool
	// refreshQueued requests one more refresh when configuration changes while
	// a previous refresh is still in flight.
	refreshQueued bool
	gen           int
	lastSync      time.Time
	nextSync      time.Time
	frame         int

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
	fi.Placeholder = i18n.Text(i18n.Resolve(cfg.Language), "filtrar por título, repo, autor…")
	fi.SetWidth(40)
	m := &Model{
		cfg:      cfg,
		language: i18n.Resolve(cfg.Language),
		session:  app.NewSession(cfg, store),
		instErr:  map[string]error{},
		clones:   map[string]string{},
		wts:      map[string]string{},
		pending:  map[string]string{},
		filter:   fi,
		mode:     launch.Resolve(cfg.Terminal),
	}
	m.rebuildClients()
	return m
}

func (m *Model) rebuildClients() {
	m.clients = map[string]provider.Client{}
	for _, in := range m.cfg.Instances {
		m.clients[in.Name] = provider.New(in, m.cfg.Repos...)
	}
}

// Close cancels pending application work before the cache is closed.
func (m *Model) Close() { m.session.Close() }

// Messages.
type (
	stateMsg struct {
		state   app.State
		err     error
		gen     int
		refresh bool
	}
	clockMsg       struct{}
	autoRefreshMsg struct{ gen int }
	actionDoneMsg  struct {
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
	return func() tea.Msg {
		state, err := m.session.Load(context.Background())
		return stateMsg{state: state, err: err}
	}
}

func (m *Model) refresh() tea.Cmd {
	if m.loading {
		return nil
	}
	m.loading = true
	m.gen++
	gen := m.gen
	m.session.Configure(m.cfg)
	return func() tea.Msg {
		state, err := m.session.Refresh(context.Background())
		return stateMsg{state: state, err: err, gen: gen, refresh: true}
	}
}

func (m *Model) scheduleRefresh() tea.Cmd {
	gen := m.gen
	d := m.cfg.Interval()
	m.nextSync = time.Now().Add(d)
	return tea.Tick(d, func(time.Time) tea.Msg { return autoRefreshMsg{gen: gen} })
}

func (m *Model) requestRefresh() tea.Cmd {
	if m.loading {
		m.refreshQueued = true
		return nil
	}
	return m.refresh()
}

func (m *Model) scanWorktrees() {
	m.wts = m.session.Worktrees()
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

	case stateMsg:
		if msg.refresh && msg.gen != m.gen {
			return m, nil
		}
		if msg.refresh {
			m.loading = false
		}
		if msg.err != nil {
			m.setStatus(stErr, i18n.ErrorText(m.language, msg.err))
		} else {
			m.prs, m.instErr, m.clones, m.wts = msg.state.Items, msg.state.Errors, msg.state.Clones, msg.state.Worktrees
			m.cacheErr, m.lastSync = msg.state.CacheError, msg.state.SyncedAt
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
		if msg.refresh {
			if m.refreshQueued {
				m.refreshQueued = false
				return m, m.refresh()
			}
			return m, m.scheduleRefresh()
		}
		return m, nil

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
			m.showMessage(m.t("Erro"), i18n.ErrorText(m.language, msg.err))
			m.setStatus(stErr, i18n.ErrorText(m.language, msg.err))
		} else if msg.ok != "" {
			m.setStatus(stOK, msg.ok)
		}
		if msg.dirty != nil {
			m.confirmForceRemove(*msg.dirty)
		}
		if msg.refresh {
			cmds = append(cmds, m.requestRefresh())
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
			m.showMessage(m.t("Erro ao abrir"), i18n.ErrorText(m.language, err))
			return m, nil
		}
		if cmd != nil {
			return m, tea.ExecProcess(cmd, func(err error) tea.Msg { return execDoneMsg{err} })
		}
		m.setStatus(stOK, m.t("aberto em nova aba (")+string(m.mode)+"): "+msg.label)
		return m, nil

	case execDoneMsg:
		m.scanWorktrees()
		if msg.err != nil {
			m.setStatus(stErr, i18n.ErrorText(m.language, msg.err))
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
		m.setStatus(stInfo, m.t("atualizando…"))
		return m.requestRefresh()
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
			f.err = oneLine(i18n.ErrorText(m.language, err))
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
		{key: "v", label: fmt.Sprintf(m.t("Ver conversa (%d comentários)"), pr.Comments)},
		{key: "n", label: m.t("Comentar")},
	}
	if pr.IsIssue() {
		m.menu = append(conversation, menuItem{key: "o", label: m.t("Abrir no navegador")})
		return
	}
	_, hasWT := m.wts[pr.Key()]
	noWT := ""
	if !hasWT {
		noWT = m.t("não há worktree para este PR")
	}
	client := m.clients[pr.Instance]
	noTool := ""
	if client != nil && !m.toolAvailable(client.Tool()) {
		noTool = client.Tool() + m.t(" não instalado")
	}
	wtLabel := m.t("Checkout em worktree")
	if hasWT {
		wtLabel = m.t("Atualizar worktree")
	}
	tracked := false
	if repo, ok := m.cfg.Repo(pr.Instance, pr.Repo); ok {
		tracked = repo.TrackAll
	}
	kind := "PRs"
	if pr.Provider == config.GitLab {
		kind = "MRs"
	}
	trackLabel := m.t("Acompanhar todos os ") + kind + m.t(" deste repositório")
	if tracked {
		trackLabel = m.t("Parar de acompanhar todos os ") + kind + m.t(" deste repositório")
	}
	m.menu = append(conversation, []menuItem{
		{key: "w", label: wtLabel},
		{key: "c", label: m.t("Checkout no clone local"), disabled: noTool},
		{key: "d", label: m.t("Ver diff (") + m.diffToolName() + ")"},
		{key: "t", label: m.t("Abrir terminal no worktree (") + string(m.mode) + ")"},
		{key: "a", label: m.t("Aprovar"), disabled: noTool},
		{key: "A", label: m.t("Aprovar e remover worktree"), disabled: firstNonEmpty(noTool, noWT)},
		{key: "m", label: "Merge", disabled: noTool},
		{key: "M", label: m.t("Merge e remover worktree"), disabled: firstNonEmpty(noTool, noWT)},
		{key: "X", label: m.t("Fechar sem merge"), disabled: noTool},
		{key: "C", label: m.t("Fechar sem merge e remover worktree"), disabled: firstNonEmpty(noTool, noWT)},
		{key: "x", label: m.t("Remover worktree (sem aprovar)"), disabled: noWT},
		{key: "o", label: m.t("Abrir no navegador")},
		{key: "R", label: trackLabel},
		{key: "p", label: m.t("Definir pasta local do repositório")},
	}...)
}

func (m *Model) openBrowser(url string) {
	if err := launch.Browser(url); err != nil {
		m.setStatus(stErr, i18n.ErrorText(m.language, err))
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
	if m.cfg.DiffTool == "hunk" && m.toolAvailable("hunk") {
		return "hunk"
	}
	return "git diff"
}

func (m *Model) prAction(pr *provider.Item, k string) tea.Cmd {
	if _, busy := m.pending[pr.Key()]; busy && k != "o" && k != "p" {
		m.setStatus(stInfo, m.t("aguarde: ")+m.pending[pr.Key()])
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
		if strings.Contains("wcdtaAmMXCxp", k) {
			m.setStatus(stInfo, m.t("ação disponível apenas para pull/merge requests"))
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
	switch k {
	case "R":
		enabled := false
		if repo, ok := m.cfg.Repo(p.Instance, p.Repo); ok {
			enabled = repo.TrackAll
		}
		m.cfg.SetRepoTrackAll(p.Instance, p.Repo, !enabled)
		if err := m.saveConfiguration(); err != nil {
			m.cfg.SetRepoTrackAll(p.Instance, p.Repo, enabled)
			m.setStatus(stErr, i18n.ErrorText(m.language, err))
			return nil
		}
		m.rebuildClients()
		if enabled {
			m.setStatus(stOK, m.t("acompanhamento de todos os PRs/MRs desativado para ")+p.Repo)
		} else {
			m.setStatus(stOK, m.t("acompanhando todos os PRs/MRs de ")+p.Repo)
		}
		return m.requestRefresh()
	case "p":
		m.openRepoPathForm(p, "")
		return m.form.setFocus(0)
	case "w", "t", "d":
		action := app.UpdateWorktree
		label := m.t("criando worktree")
		if k == "t" {
			action, label = app.PrepareTerminal, m.t("preparando worktree")
		}
		if k == "d" {
			action, label = app.PrepareDiff, m.t("preparando worktree")
		}
		return start(label, func(ctx context.Context) tea.Msg {
			out, err := m.session.Execute(ctx, p.Key(), app.Command{Action: action, Item: p})
			if err != nil || k == "w" {
				return actionResult(p, k, out, err)
			}
			name := filepathBase(p.Repo) + "-" + strconv.Itoa(p.Number)
			if k == "d" {
				name = "diff " + name
			}
			return launchMsg{key: key, dir: out.Directory, label: name, argv: out.Args, note: out.Note}
		})
	case "c":
		m.ask(m.t("Checkout no clone local?"), "Checkout", func() tea.Cmd {
			return start("checkout", func(ctx context.Context) tea.Msg {
				out, err := m.session.Execute(ctx, p.Key(), app.Command{Action: app.Checkout, Item: p})
				return actionResult(p, k, out, err)
			})
		}, m.t("Troca a branch atual de ")+firstNonEmpty(m.clones[key], m.t("(clone não configurado)")), m.t("para a branch do PR usando ")+client.Tool()+".")
	case "a", "A":
		if k == "A" && !hasWT {
			return nil
		}
		title := m.t("Aprovar ") + p.Ref() + "?"
		if k == "A" {
			title = m.t("Aprovar ") + p.Ref() + m.t(" e remover o worktree?")
		}
		m.ask(title, m.t("Aprovar"), func() tea.Cmd {
			return start(m.t("aprovando"), func(ctx context.Context) tea.Msg {
				out, err := m.session.Execute(ctx, p.Key(), app.Command{Action: app.Approve, Item: p, RemoveAfter: k == "A"})
				return actionResult(p, k, out, err)
			})
		}, p.Repo, p.Title)
	case "m", "M":
		if k == "M" && !hasWT {
			return nil
		}
		opts, err := m.session.MergeOptions(p.Key())
		if err != nil {
			m.setStatus(stErr, i18n.ErrorText(m.language, err))
			return nil
		}
		title := m.t("Fazer merge de ") + p.Ref() + "?"
		if k == "M" {
			title = m.t("Fazer merge de ") + p.Ref() + m.t(" e remover o worktree?")
		}
		body := []string{p.Repo, p.Title, "", m.t("método: ") + opts.Method}
		if opts.Auto {
			body = append(body, m.t("auto-merge: sim (aguarda pipeline)"))
		}
		if opts.DeleteBranch {
			body = append(body, m.t("apagar branch de origem: sim"))
		}
		if p.CI == provider.CIFailure {
			body = append(body, sRed.Render(m.t("atenção: pipeline falhou")))
		}
		m.ask(title, "Merge", func() tea.Cmd {
			return start(m.t("fazendo merge"), func(ctx context.Context) tea.Msg {
				out, err := m.session.Execute(ctx, p.Key(), app.Command{Action: app.Merge, Item: p, RemoveAfter: k == "M"})
				return actionResult(p, k, out, err)
			})
		}, body...)
	case "X", "C":
		if k == "C" && !hasWT {
			return nil
		}
		title := m.t("Fechar ") + p.Ref() + m.t(" sem merge?")
		if k == "C" {
			title = m.t("Fechar ") + p.Ref() + m.t(" sem merge e remover o worktree?")
		}
		m.ask(title, m.t("Fechar"), func() tea.Cmd {
			return start(m.t("fechando"), func(ctx context.Context) tea.Msg {
				out, err := m.session.Execute(ctx, p.Key(), app.Command{Action: app.Close, Item: p, RemoveAfter: k == "C"})
				return actionResult(p, k, out, err)
			})
		}, p.Repo, p.Title, "", m.t("A branch ")+p.SourceBranch+m.t(" é mantida no servidor."))
	case "x":
		if !hasWT {
			return nil
		}
		m.ask(m.t("Remover o worktree de ")+p.Ref()+"?", m.t("Remover"), func() tea.Cmd {
			return start(m.t("removendo worktree"), func(ctx context.Context) tea.Msg {
				out, err := m.session.Execute(ctx, p.Key(), app.Command{Action: app.RemoveWorktree, Item: p})
				return actionResult(p, k, out, err)
			})
		}, m.wts[key])
	}
	return nil
}

func (m *Model) confirmForceRemove(pr provider.Item) {
	key := pr.Key()
	m.ask(m.t("O worktree tem alterações locais"), m.t("Remover mesmo assim"), func() tea.Cmd {
		m.pending[key] = m.t("removendo worktree")
		return func() tea.Msg {
			ctx, cancel := context.WithTimeout(context.Background(), time.Minute)
			defer cancel()
			out, err := m.session.Execute(ctx, pr.Key(), app.Command{Action: app.RemoveWorktree, Item: pr, Force: true})
			return actionResult(pr, "x", out, err)
		}
	}, m.wts[key], "", sRed.Render(m.t("As alterações não commitadas serão perdidas.")))
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
		title: m.t("Pasta local de ") + pr.Repo + " (" + pr.Instance + ")",
		fields: []*field{
			textField("Pasta do clone", cur, "~/code/"+filepathBase(pr.Repo), m.t("caminho de um clone existente; vazio remove o mapeamento")),
			textField("Remote", remote, m.t("detectar automaticamente"), m.t("remote que aponta para o repositório (ex.: origin, upstream)")),
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
		if err := m.saveConfiguration(); err != nil {
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
		m.setStatus(stOK, m.t("pasta local salva para ")+pr.Repo)
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
		title: m.t("Configurações — ") + c.FilePath(),
		fields: []*field{
			choiceField("Idioma", []string{"system", "en", "pt"}, firstNonEmpty(c.Language, "system"), m.t("Seguir o idioma do computador ou escolher um idioma")),
			textField("Atualização", c.RefreshInterval, "5m", m.t("intervalo de atualização automática (ex.: 90s, 5m, 1h)")),
			choiceField("Terminal", []string{"auto", "herdr", "tmux", "inline"}, c.Terminal, m.t("onde abrir terminal/diff; auto = herdr > tmux > inline")),
			choiceField("Diff", []string{"hunk", "git"}, c.DiffTool, m.t("ferramenta de revisão do diff")),
			textField("Worktrees", firstNonEmpty(c.WorktreeDir, wtDir), "~/.pr-tracker/worktrees", m.t("pasta onde os worktrees são criados")),
			textField("Clone roots", strings.Join(c.CloneRoots, ", "), "~/code, ~/work", m.t("pastas onde procurar clones automaticamente (separadas por vírgula)")),
		},
	}
	f.get("Idioma").optionLabels = map[string]string{"system": "Sistema", "en": "English", "pt": "Português"}
	f.submit = func(f *form) error {
		next := *c
		next.Language = f.get("Idioma").value()
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
		if err := m.saveConfiguration(); err != nil {
			return err
		}
		m.language = i18n.Resolve(c.Language)
		m.filter.Placeholder = m.t("filtrar por título, repo, autor…")
		if m.thread != nil {
			m.thread.lines = nil
		}
		m.mode = launch.Resolve(c.Terminal)
		m.setStatus(stOK, m.t("configurações salvas"))
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
			if err := m.saveConfiguration(); err != nil {
				m.setStatus(stErr, i18n.ErrorText(m.language, err))
			}
			m.rebuildClients()
			return m.refresh()
		}
	case "D", "delete":
		if m.instCur < n {
			name := m.cfg.Instances[m.instCur].Name
			m.ask(m.t("Remover a instância ")+name+"?", m.t("Remover"), func() tea.Cmd {
				m.cfg.RemoveInstance(name)
				if err := m.saveConfiguration(); err != nil {
					m.setStatus(stErr, i18n.ErrorText(m.language, err))
				}
				m.instCur = max(0, min(m.instCur, len(m.cfg.Instances)-1))
				m.rebuildClients()
				m.prs = slices.DeleteFunc(m.prs, func(p provider.Item) bool { return p.Instance == name })
				return m.refresh()
			}, m.t("Os mapeamentos de pastas locais desta instância também serão removidos."))
		}
	case "t":
		if m.instCur < n {
			c := provider.New(m.cfg.Instances[m.instCur])
			m.setStatus(stInfo, m.t("verificando autenticação de ")+c.Instance().Name+"…")
			authenticated := m.t(" autenticado em ")
			return func() tea.Msg {
				ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
				defer cancel()
				if err := c.AuthStatus(ctx); err != nil {
					return actionDoneMsg{err: err}
				}
				return actionDoneMsg{ok: c.Tool() + authenticated + c.Instance().Host}
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
	providers := []string{string(config.GitHub), string(config.GitLab), string(config.Gitea), string(config.Bitbucket)}
	title := m.t("Nova instância")
	if old != "" {
		title = m.t("Editar instância ") + old
	}
	f := &form{
		title: title,
		fields: []*field{
			choiceField("Provider", providers, string(cur.Provider), m.t("github (gh) · gitlab (glab) · gitea (tea) · bitbucket (ainda não suportado)")),
			textField("Host", cur.Host, m.t("github.com, gitlab.empresa.com…"), m.t("host da instância; vazio usa o SaaS do provider")),
			textField("Nome", cur.Name, m.t("igual ao host"), m.t("identificador único da instância")),
			choiceField("Merge", []string{"merge", "squash", "rebase"}, firstNonEmpty(cur.MergeMethod, "merge"), m.t("método de merge padrão")),
			boolField("Auto-merge", cur.AutoMerge, m.t("habilita merge automático quando o pipeline passar")),
			boolField("Apagar branch", cur.DeleteBranch, m.t("apaga a branch de origem após o merge")),
			boolField("Desativada", cur.Disabled, m.t("mantém a instância mas não consulta")),
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
		if err := m.saveConfiguration(); err != nil {
			return err
		}
		m.rebuildClients()
		tool := provider.New(next).Tool()
		if next.Provider == config.Bitbucket {
			m.setStatus(stErr, m.t("Bitbucket salvo, mas a integração ainda não existe (veja docs/bitbucket.md)"))
		} else if !provider.ToolAvailable(tool) {
			m.setStatus(stErr, tool+m.t(" não instalado. Instale: ")+provider.InstallHint(tool))
		} else {
			m.setStatus(stOK, m.t("instância salva"))
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

func actionResult(item provider.Item, action string, out app.Outcome, err error) tea.Msg {
	if errors.Is(err, app.ErrCloneRequired) {
		return needPathMsg{pr: item, action: action}
	}
	msg := actionDoneMsg{key: item.Key(), ok: out.Message, err: err, refresh: out.Refresh, reloadThread: out.ReloadThread}
	if errors.Is(err, app.ErrDirtyWorktree) {
		msg.err, msg.dirty = nil, &item
	}
	return msg
}

// saveConfiguration restores the last committed settings when persistence fails.
func (m *Model) saveConfiguration() error {
	err := m.session.SaveSettings(*m.cfg)
	*m.cfg = m.session.Configuration()
	return err
}

func (m *Model) toolAvailable(name string) bool {
	_, err := toolchain.Lookup(toolchain.WithPaths(context.Background(), m.cfg.ToolPaths), name)
	return err == nil
}

func (m *Model) t(message string) string { return i18n.Text(m.language, message) }
