package ui

import (
	"fmt"
	"slices"
	"strings"
	"time"

	tea "charm.land/bubbletea/v2"
	"charm.land/lipgloss/v2"

	"github.com/vitorpacheco/pr-tracker/internal/config"
	"github.com/vitorpacheco/pr-tracker/internal/i18n"
	"github.com/vitorpacheco/pr-tracker/internal/provider"
)

// View renders the whole screen and registers clickable zones.
func (m *Model) View() tea.View {
	m.zones = m.zones[:0]
	v := tea.NewView("")
	v.AltScreen = true
	v.MouseMode = tea.MouseModeCellMotion
	v.WindowTitle = "PR Tracker"
	if m.styles.palette.Name != "" {
		v.BackgroundColor = m.styles.cBlack
		v.ForegroundColor = m.styles.cText
	}
	m.styles.input(&m.filter)
	if m.compose != nil {
		m.styles.textarea(m.compose)
	}
	if m.width == 0 || m.height == 0 {
		return v
	}
	W, H := m.width, m.height

	var lines []string
	lines = append(lines, m.header(W))
	for _, b := range m.banners() {
		m.zones.add("banner", 0, len(lines), W, 1)
		lines = append(lines, fit(m.styles.sBanner.Render(" ⚠ "+b), W))
	}
	if m.screen == screenPRs && (m.filtering || m.filter.Value() != "") {
		f := " " + m.filter.View()
		if !m.filtering {
			f = " " + m.styles.sAccent.Render("/ "+m.filter.Value()) + m.styles.sDim.Render(m.t("  (esc limpa)"))
		}
		lines = append(lines, fit(f, W))
	}
	lines = append(lines, m.styles.sDim.Render(strings.Repeat("─", W)))

	hints := m.hints()
	var probe zones
	footerN := len(m.styles.hintBar(hints, W-2, 0, 0, &probe))
	bodyH := max(H-len(lines)-footerN-2, 3)

	var body []string
	switch {
	case m.screen == screenInstances:
		body = m.instancesBody(W, bodyH, len(lines))
	case m.screen == screenThread && m.thread != nil:
		body = m.threadBody(W, bodyH)
	default:
		body = m.prsBody(W, bodyH, len(lines))
	}
	for len(body) < bodyH {
		body = append(body, "")
	}
	lines = append(lines, body[:bodyH]...)
	lines = append(lines, m.statusLine(W))
	for _, l := range m.styles.hintBar(hints, W-2, 1, len(lines), &m.zones) {
		lines = append(lines, " "+l)
	}
	base := strings.Join(lines, "\n")

	if m.modal != modalNone {
		var mz zones
		box := m.styles.sModal.Render(m.modalContent(W, H, &mz))
		bw, bh := lipgloss.Width(box), lipgloss.Height(box)
		x, y := max((W-bw)/2, 0), max((H-bh)/2, 0)
		m.zones = m.zones[:0] // only the modal is interactive
		for _, z := range mz {
			m.zones.add(z.id, z.x+x+2, z.y+y+1, z.w, z.h)
		}
		base = lipgloss.NewCompositor(
			lipgloss.NewLayer(base),
			lipgloss.NewLayer(box).X(x).Y(y).Z(1),
		).Render()
	}
	v.SetContent(base)
	return v
}

func (m *Model) header(W int) string {
	title := m.styles.sTitle.Render("⎇ PR Tracker")
	x := lipgloss.Width(title) + 1
	parts := []string{title, " "}
	if m.screen == screenInstances || m.screen == screenThread {
		name := m.t("Instâncias")
		if m.screen == screenThread {
			name = m.t("Conversa ") + m.thread.item.Ref()
		}
		parts = append(parts, m.styles.sTabActive.Render(name))
		x += lipgloss.Width(parts[len(parts)-1])
		back := m.styles.sTab.Render(m.t("← voltar"))
		m.zones.add("key:esc", x, 0, lipgloss.Width(back), 1)
		parts = append(parts, back)
	} else {
		// Inactive tabs drop their label when the full row doesn't fit.
		compact := W < 160
		for i, t := range tabs {
			if i == 0 || i == firstIssueTab {
				group := "PRs"
				if i == firstIssueTab {
					group = "│ Issues"
				}
				g := m.styles.sGroup.Render(group) + " "
				x += lipgloss.Width(g)
				parts = append(parts, g)
			}
			label := fmt.Sprintf("%s %s %s", m.styles.sKey.Render(itoa(i+1)), m.t(t.label), m.styles.sMuted.Render(itoa(m.count(t))))
			if compact && i != m.tab {
				label = fmt.Sprintf("%s %s", m.styles.sKey.Render(itoa(i+1)), m.styles.sMuted.Render(itoa(m.count(t))))
			}
			st := m.styles.sTab
			if i == m.tab {
				st = m.styles.sTabActive
			}
			s := st.Render(label)
			m.zones.add("tab:"+itoa(i), x, 0, lipgloss.Width(s), 1)
			x += lipgloss.Width(s)
			parts = append(parts, s)
		}
	}
	left := strings.Join(parts, "")

	var right string
	switch {
	case m.loading:
		right = m.styles.sYellow.Render(spinnerFrames[m.frame%len(spinnerFrames)] + m.t(" atualizando"))
	case !m.lastSync.IsZero():
		right = m.styles.sMuted.Render(m.t("atualizado há ")+duration(time.Since(m.lastSync))) +
			m.styles.sDim.Render(m.t(" · próxima em ")+duration(max(time.Until(m.nextSync), 0)))
	}
	right = m.styles.sKey.Render("r") + " " + right + " "
	gap := W - lipgloss.Width(left) - lipgloss.Width(right)
	if gap < 1 {
		return fit(left, W)
	}
	m.zones.add("key:r", W-lipgloss.Width(right), 0, lipgloss.Width(right), 1)
	return left + spaces(gap) + right
}

// banners lists problems that block instances: missing CLIs and fetch errors.
func (m *Model) banners() []string {
	var out []string
	if m.themeError != nil {
		out = append(out, i18n.ErrorText(m.language, m.themeError))
	}
	if m.cacheErr != nil {
		out = append(out, m.t("cache local indisponível: ")+i18n.ErrorText(m.language, m.cacheErr))
	}
	missing := map[string]bool{}
	for _, in := range m.cfg.Instances {
		if in.Disabled || in.Provider == config.Bitbucket {
			continue
		}
		tool := provider.New(in).Tool()
		if !provider.ToolAvailable(tool) && !missing[tool] {
			missing[tool] = true
			out = append(out, fmt.Sprintf(m.t("%s não está instalado e é necessário para %s. Instale: %s"), tool, in.Name, provider.InstallHint(tool)))
		}
	}
	var errs []string
	for _, in := range m.cfg.Instances {
		if err := m.instErr[in.Name]; err != nil && !missing[provider.New(in).Tool()] {
			errs = append(errs, in.Name)
		}
	}
	if len(errs) > 0 {
		out = append(out, fmt.Sprintf(m.t("falha ao consultar %s — clique ou pressione i para detalhes"), strings.Join(errs, ", ")))
	}
	return out
}

func (m *Model) hints() []hint {
	if m.screen == screenThread {
		return []hint{
			{"↑↓", m.t("rolar")}, {"space", m.t("página")}, {"g", m.t("início")}, {"G", m.t("fim")}, {"n", m.t("comentar")},
			{"o", m.t("navegador")}, {"r", m.t("recarregar")}, {"esc", m.t("voltar")}, {"?", m.t("ajuda")},
		}
	}
	if cur := m.current(); m.screen == screenPRs && cur != nil && cur.IsIssue() {
		return []hint{
			{"enter", m.t("ações")}, {"v", m.t("conversa")}, {"n", m.t("comentar")}, {"o", m.t("navegador")},
			{"/", m.t("filtrar")}, {"r", m.t("atualizar")}, {"i", m.t("instâncias")}, {"s", "config"}, {"?", m.t("ajuda")}, {"q", m.t("sair")},
		}
	}
	if m.screen == screenInstances {
		return []hint{
			{"n", m.t("nova")}, {"e", m.t("editar")}, {"space", m.t("ativar/desativar")}, {"t", m.t("testar auth")},
			{"D", m.t("remover")}, {"s", "config"}, {"esc", m.t("voltar")}, {"?", m.t("ajuda")}, {"q", m.t("sair")},
		}
	}
	return []hint{
		{"enter", m.t("ações")}, {"v", m.t("conversa")}, {"n", m.t("comentar")}, {"w", "worktree"}, {"d", "diff"}, {"t", "terminal"}, {"c", "checkout"},
		{"a", m.t("aprovar")}, {"m", "merge"}, {"X", m.t("fechar")}, {"x", "rm worktree"}, {"o", m.t("navegador")}, {"R", m.t("todos do repo")},
		{"/", m.t("filtrar")}, {"r", m.t("atualizar")}, {"i", m.t("instâncias")}, {"s", "config"}, {"?", m.t("ajuda")}, {"q", m.t("sair")},
	}
}

func (m *Model) statusLine(W int) string {
	if m.status != "" {
		st := m.styles.sMuted
		icon := "•"
		switch m.statusKind {
		case stOK:
			st, icon = m.styles.sGreen, "✔"
		case stErr:
			st, icon = m.styles.sRed, "✘"
		}
		return fit(" "+st.Render(icon+" "+m.status), W)
	}
	if len(m.pending) > 0 {
		var labels []string
		for _, l := range m.pending {
			labels = append(labels, l)
		}
		slices.Sort(labels)
		return fit(" "+m.styles.sYellow.Render(spinnerFrames[m.frame%len(spinnerFrames)]+" "+strings.Join(labels, " · ")), W)
	}
	wt, _ := m.cfg.Worktrees()
	issues := m.count(tabDef{kind: provider.KindIssue})
	return fit(m.styles.sDim.Render(fmt.Sprintf(" %d PRs · %d issues · terminal: %s · worktrees: %s", len(m.prs)-issues, issues, m.mode, wt)), W)
}

// ---------- PR list ----------

func (m *Model) prsBody(W, H, top int) []string {
	if len(m.cfg.Instances) == 0 {
		return centered(W, H, []string{
			m.styles.sBold.Render(m.t("Nenhuma instância configurada")),
			"",
			m.styles.sMuted.Render(m.t("Pressione ")) + m.styles.sKey.Render("i") + m.styles.sMuted.Render(m.t(" e depois ")) + m.styles.sKey.Render("n") + m.styles.sMuted.Render(m.t(" para cadastrar GitHub/GitLab/Gitea (inclusive self-hosted).")),
		})
	}
	listW, detailW := W, 0
	if W >= 110 {
		listW = W * 58 / 100
		detailW = W - listW - 1
	}
	vis := m.visible()
	m.listTop = top + 1
	m.listRows = H - 1
	m.clampOffset()

	var list []string
	list = append(list, m.columns(listW))
	switch {
	case len(vis) == 0 && m.loading && m.lastSync.IsZero():
		list = append(list, centered(listW, H-1, []string{m.styles.sYellow.Render(spinnerFrames[m.frame%len(spinnerFrames)] + m.t(" carregando pull requests…"))})...)
	case len(vis) == 0:
		msg := m.t("Nada por aqui ✨")
		if m.filter.Value() != "" {
			msg = m.t("Nenhum PR corresponde ao filtro")
		}
		list = append(list, centered(listW, H-1, []string{m.styles.sMuted.Render(msg)})...)
	default:
		for i := m.offset; i < len(vis) && i < m.offset+m.listRows; i++ {
			m.zones.add("row:"+itoa(i), 0, m.listTop+i-m.offset, listW, 1)
			list = append(list, m.row(vis[i], i == m.cursor, listW))
		}
	}
	for len(list) < H {
		list = append(list, "")
	}
	if detailW == 0 {
		for i := range list {
			list[i] = fit(list[i], listW)
		}
		return list
	}
	detail := m.detail(m.current(), detailW-2, H)
	out := make([]string, H)
	sep := m.styles.sDim.Render("│")
	for i := range H {
		d := ""
		if i < len(detail) {
			d = detail[i]
		}
		out[i] = fit(list[i], listW) + sep + " " + fit(d, detailW-1)
	}
	if len(vis) > m.listRows {
		// Scroll position indicator on the separator.
		pos := m.listTop - top + (m.cursor * (m.listRows - 1) / max(len(vis)-1, 1))
		if pos < H {
			out[pos] = fit(list[pos], listW) + m.styles.sAccent.Render("┃") + " " + fit(detailAt(detail, pos), detailW-1)
		}
	}
	return out
}

func detailAt(d []string, i int) string {
	if i < len(d) {
		return d[i]
	}
	return ""
}

type colWidths struct{ repo, ref, author, title int }

func (m *Model) widths(W int) colWidths {
	c := colWidths{repo: min(26, max(12, W/5)), ref: 6}
	if W >= 80 {
		c.author = 12
	}
	// marker(2) ci(3) review(3) repo ref title author comments(4) age(5) flags(4)
	c.title = max(W-2-3-3-c.repo-1-c.ref-1-c.author-1-4-5-4, 8)
	return c
}

func (m *Model) columns(W int) string {
	c := m.widths(W)
	s := "  " + "CI " + "RV " + fit(m.t("REPOSITÓRIO"), c.repo) + " " + fit("#", c.ref) + " " + fit(m.t("TÍTULO"), c.title) + " "
	if c.author > 0 {
		s += fit(m.t("AUTOR"), c.author)
	}
	s += fit("COM", 4) + fit(m.t(" ATU."), 5)
	return m.styles.sDim.Render(fit(s, W))
}

func (m *Model) row(pr *provider.Item, selected bool, W int) string {
	c := m.widths(W)
	marker := "  "
	titleSt := lipgloss.NewStyle().Foreground(m.styles.cText)
	if selected {
		marker = m.styles.sAccent.Render("▌ ")
		titleSt = titleSt.Bold(true).Foreground(m.styles.cText)
	}
	repo := pr.Repo
	if lipgloss.Width(repo) > c.repo {
		repo = "…" + string([]rune(repo)[len([]rune(repo))-c.repo+1:])
	}
	title := titleSt.Render(pr.Title)
	if pr.Draft {
		title = m.styles.sMuted.Render("[draft] ") + title
	}
	if len(pr.Labels) > 0 {
		title += " " + m.styles.sDim.Render(strings.Join(pr.Labels, " · "))
	}
	ci := m.styles.ciIcon(pr.CI)
	if pr.IsIssue() {
		ci = m.styles.sGreen.Render("◉")
	}
	comments := ""
	if pr.Comments > 0 {
		comments = itoa(pr.Comments)
	}
	var flags []string
	if _, busy := m.pending[pr.Key()]; busy {
		flags = append(flags, m.styles.sYellow.Render(spinnerFrames[m.frame%len(spinnerFrames)]))
	}
	if _, ok := m.wts[pr.Key()]; ok {
		flags = append(flags, m.styles.sBlue.Render("⎇"))
	}
	if pr.Conflicts {
		flags = append(flags, m.styles.sRed.Render("⚠"))
	}
	s := marker + ci + "  " + m.styles.reviewIcon(pr) + "  " +
		fit(m.styles.sMuted.Render(repo), c.repo) + " " +
		fit(m.styles.sAccent.Render(pr.Ref()), c.ref) + " " +
		fit(title, c.title) + " "
	if c.author > 0 {
		s += fit(m.styles.sMuted.Render(pr.Author), c.author-1) + " "
	}
	s += fit(m.styles.sMuted.Render(comments), 4) + fit(m.styles.sDim.Render(" "+age(pr.UpdatedAt, m.t)), 5) + strings.Join(flags, "")
	if selected {
		return m.styles.sRowSel.Render(fit(s, W))
	}
	return fit(s, W)
}

func (m *Model) detail(pr *provider.Item, W, H int) []string {
	if pr == nil {
		return nil
	}
	var d []string
	title := lipgloss.NewStyle().Bold(true).Foreground(m.styles.cText).Width(W).Render(pr.Title)
	tl := strings.Split(title, "\n")
	if len(tl) > 3 {
		tl = append(tl[:3], "…")
	}
	d = append(d, tl...)
	d = append(d, m.styles.sAccent.Render(pr.Repo+" "+pr.Ref())+m.styles.sDim.Render(" · "+pr.Instance))
	d = append(d, m.styles.sDim.Render(pr.URL), "")

	kv := func(k, v string) { d = append(d, m.styles.sLabel.Render(k)+v) }
	kv(m.t("Autor"), pr.Author)
	if len(pr.Assignees) > 0 {
		kv(m.t("Atribuído a"), strings.Join(pr.Assignees, ", "))
	}
	if len(pr.Labels) > 0 {
		kv("Labels", m.styles.labels(pr.Labels))
	}
	kv(m.t("Comentários"), fmt.Sprintf("%d ", pr.Comments)+m.styles.sKey.Render("v")+m.styles.sMuted.Render(m.t(" ver · "))+m.styles.sKey.Render("n")+m.styles.sMuted.Render(m.t(" comentar")))
	if pr.IsIssue() {
		kv(m.t("Você é"), m.styles.relations(pr, m.t))
		kv(m.t("Atualizado"), age(pr.UpdatedAt, m.t)+m.styles.sDim.Render(m.t(" · criada ")+age(pr.CreatedAt, m.t)))
		if l, ok := m.pending[pr.Key()]; ok {
			kv(m.t("Executando"), m.styles.sYellow.Render(spinnerFrames[m.frame%len(spinnerFrames)]+" "+l))
		}
		return d
	}
	branch := m.styles.sBlue.Render(pr.SourceBranch) + m.styles.sDim.Render(" → ") + pr.TargetBranch
	if pr.FromFork {
		branch += m.styles.sYellow.Render(" (fork)")
	}
	kv("Branch", branch)
	state := m.styles.sGreen.Render(m.t("aberto"))
	if pr.Draft {
		state = m.styles.sMuted.Render(m.t("rascunho"))
	}
	if pr.Conflicts {
		state += m.styles.sRed.Render(m.t(" · com conflitos"))
	}
	kv(m.t("Estado"), state)
	rev := m.styles.reviewLabel(pr, m.t)
	if pr.ApprovedByMe {
		rev += m.styles.sGreen.Render(m.t(" · você aprovou"))
	}
	kv(m.t("Revisão"), rev)
	if len(pr.ApprovedBy) > 0 {
		kv(m.t("Aprovado por"), strings.Join(pr.ApprovedBy, ", "))
	}
	kv(m.t("Você é"), m.styles.relations(pr, m.t))
	if pr.Additions+pr.Deletions+pr.Files > 0 {
		kv(m.t("Mudanças"), m.styles.sGreen.Render(fmt.Sprintf("+%d", pr.Additions))+" "+m.styles.sRed.Render(fmt.Sprintf("-%d", pr.Deletions))+m.styles.sMuted.Render(fmt.Sprintf(m.t(" em %d arquivos"), pr.Files)))
	}
	kv(m.t("Atualizado"), age(pr.UpdatedAt, m.t)+m.styles.sDim.Render(m.t(" · criado ")+age(pr.CreatedAt, m.t)))
	if c := m.clones[pr.Key()]; c != "" {
		kv("Clone", c)
	} else {
		kv("Clone", m.styles.sYellow.Render(m.t("não configurado "))+m.styles.sKey.Render("p")+m.styles.sMuted.Render(m.t(" define")))
	}
	if wt, ok := m.wts[pr.Key()]; ok {
		kv("Worktree", m.styles.sBlue.Render(wt))
	} else {
		kv("Worktree", m.styles.sDim.Render("— ")+m.styles.sKey.Render("w")+m.styles.sMuted.Render(m.t(" cria")))
	}
	if l, ok := m.pending[pr.Key()]; ok {
		kv(m.t("Executando"), m.styles.sYellow.Render(spinnerFrames[m.frame%len(spinnerFrames)]+" "+l))
	}
	d = append(d, "", m.styles.sLabel.Render("Pipeline")+m.styles.ciLabel(pr.CI, m.t))
	checks := slices.Clone(pr.Checks)
	rank := map[provider.CIState]int{provider.CIFailure: 0, provider.CIPending: 1, provider.CICanceled: 2, provider.CISuccess: 3, provider.CINone: 4}
	slices.SortStableFunc(checks, func(a, b provider.Check) int { return rank[a.State] - rank[b.State] })
	room := H - len(d)
	for i, c := range checks {
		if i >= room-1 && len(checks) > room {
			d = append(d, m.styles.sDim.Render(fmt.Sprintf(m.t("  … mais %d"), len(checks)-i)))
			break
		}
		d = append(d, "  "+m.styles.ciIcon(c.State)+" "+c.Name)
	}
	return d
}

func (st *styles) relations(it *provider.Item, tr func(string) string) string {
	var rel []string
	if it.Relations&provider.ReviewRequested != 0 {
		rel = append(rel, tr("revisor"))
	}
	if it.Relations&provider.Authored != 0 {
		rel = append(rel, tr("autor"))
	}
	if it.Relations&provider.Assigned != 0 {
		rel = append(rel, tr("atribuído"))
	}
	if it.Relations&provider.Mentioned != 0 {
		rel = append(rel, tr("mencionado"))
	}
	return strings.Join(rel, ", ")
}

func centered(W, H int, content []string) []string {
	out := make([]string, 0, H)
	top := max((H-len(content))/2, 0)
	for range top {
		out = append(out, "")
	}
	for _, c := range content {
		out = append(out, lipgloss.PlaceHorizontal(W, lipgloss.Center, c))
	}
	return out
}

// ---------- instances ----------

func (m *Model) instancesBody(W, H, top int) []string {
	var out []string
	if len(m.cfg.Instances) == 0 {
		return centered(W, H, []string{
			m.styles.sBold.Render(m.t("Nenhuma instância")),
			"",
			m.styles.sMuted.Render(m.t("Pressione ")) + m.styles.sKey.Render("n") + m.styles.sMuted.Render(m.t(" para adicionar github.com, gitlab.com ou uma instância self-hosted.")),
			m.styles.sDim.Render(m.t("A autenticação usa os próprios gh / glab / tea (gh auth login --hostname …).")),
		})
	}
	out = append(out, m.styles.sDim.Render(fit("  "+fit(m.t("NOME"), 22)+fit("PROVIDER", 11)+fit("HOST", 30)+fit("CLI", 7)+"STATUS", W)))
	for i, in := range m.cfg.Instances {
		cl := provider.New(in)
		tool := cl.Tool()
		toolSt := m.styles.sGreen.Render(tool)
		if !provider.ToolAvailable(tool) {
			toolSt = m.styles.sRed.Render(tool + "✘")
		}
		var status string
		switch {
		case in.Provider == config.Bitbucket:
			status = m.styles.sYellow.Render(m.t("não suportado ainda"))
		case in.Disabled:
			status = m.styles.sDim.Render(m.t("desativada"))
		case !provider.ToolAvailable(tool):
			status = m.styles.sRed.Render(m.t("CLI não instalado"))
		case m.instErr[in.Name] != nil:
			status = m.styles.sRed.Render(m.t("erro"))
		case m.lastSync.IsZero():
			status = m.styles.sMuted.Render("…")
		default:
			n := 0
			for _, p := range m.prs {
				if p.Instance == in.Name {
					n++
				}
			}
			status = m.styles.sGreen.Render(fmt.Sprintf("ok · %d PRs", n))
		}
		marker := "  "
		if i == m.instCur {
			marker = m.styles.sAccent.Render("▌ ")
		}
		row := marker + fit(m.styles.sBold.Render(in.Name), 22) + fit(m.styles.sAccent.Render(string(in.Provider)), 11) + fit(in.Host, 30) + fit(toolSt, 7) + status
		m.zones.add("inst:"+itoa(i), 0, top+len(out), W, 1)
		if i == m.instCur {
			row = m.styles.sRowSel.Render(fit(row, W))
		}
		out = append(out, fit(row, W))
	}
	if m.instCur < len(m.cfg.Instances) {
		in := m.cfg.Instances[m.instCur]
		cl := provider.New(in)
		out = append(out, "", m.styles.sDim.Render(strings.Repeat("─", W)))
		kv := func(k, v string) { out = append(out, " "+m.styles.sLabel.Render(k)+v) }
		kv("Merge", firstNonEmpty(in.MergeMethod, "merge")+m.styles.sDim.Render(fmt.Sprintf(m.t(" · auto-merge %v · apagar branch %v"), in.AutoMerge, in.DeleteBranch)))
		repos := 0
		for _, r := range m.cfg.Repos {
			if r.Instance == in.Name {
				repos++
			}
		}
		kv(m.t("Repos locais"), fmt.Sprint(repos))
		if !provider.ToolAvailable(cl.Tool()) {
			kv("CLI", m.styles.sRed.Render(cl.Tool()+m.t(" não instalado — ")+provider.InstallHint(cl.Tool())))
		}
		if in.Provider == config.Bitbucket {
			kv("Bitbucket", m.styles.sYellow.Render(m.t("sem CLI oficial; integração adiada (docs/bitbucket.md)")))
		}
		if err := m.instErr[in.Name]; err != nil {
			wrapped := lipgloss.NewStyle().Width(max(W-16, 20)).Render(i18n.ErrorText(m.language, err))
			for j, l := range strings.Split(wrapped, "\n") {
				if j == 0 {
					kv(m.t("Erro"), m.styles.sRed.Render(l))
				} else {
					out = append(out, " "+spaces(12)+m.styles.sRed.Render(l))
				}
			}
		}
		switch in.Provider {
		case config.GitHub:
			kv("Login", m.styles.sDim.Render("gh auth login --hostname "+in.Host))
		case config.GitLab:
			kv("Login", m.styles.sDim.Render("glab auth login --hostname "+in.Host))
		case config.Gitea:
			kv("Login", m.styles.sDim.Render("tea login add --url https://"+in.Host))
		}
	}
	return out
}

// ---------- modals ----------

func (m *Model) modalContent(W, H int, z *zones) string {
	maxW := min(W-6, 90)
	switch m.modal {
	case modalMenu:
		var b []string
		b = append(b, m.styles.sBold.Foreground(m.styles.cAccent2).Render(fit(m.menuTitle, min(maxW, 64))), "")
		for i, it := range m.menu {
			label := it.label
			line := " " + m.styles.sKey.Render(fit(it.key, 2)) + " " + label
			if it.disabled != "" {
				line = " " + m.styles.sDim.Render(fit(it.key, 2)+" "+label)
			}
			line = fit(line, min(maxW, 64))
			if i == m.menuCur {
				line = m.styles.sRowSel.Render(line)
			}
			z.add("menu:"+itoa(i), 0, len(b), min(maxW, 64), 1)
			b = append(b, line)
		}
		b = append(b, "", m.styles.sDim.Render(m.t("↑↓ navega · enter/atalho executa · esc fecha")))
		return strings.Join(b, "\n")

	case modalConfirm:
		c := m.confirm
		var b []string
		b = append(b, m.styles.sBold.Foreground(m.styles.cAccent2).Render(c.title), "")
		for _, l := range c.body {
			b = append(b, lipgloss.NewStyle().Width(min(maxW, 70)).Render(l))
		}
		b = append(b, "")
		yes := m.styles.sTabActive.Render(c.yes + " (y/enter)")
		no := m.styles.sTab.Render(m.t("Cancelar (n/esc)"))
		z.add("confirm:yes", 0, lipglossHeight(b), lipgloss.Width(yes), 1)
		z.add("confirm:no", lipgloss.Width(yes)+2, lipglossHeight(b), lipgloss.Width(no), 1)
		b = append(b, yes+"  "+no)
		return strings.Join(b, "\n")

	case modalForm:
		return m.form.render(z, m.t, m.styles)

	case modalCompose:
		return m.composeContent(z)

	case modalMessage:
		var b []string
		b = append(b, m.styles.sRed.Bold(true).Render(m.message[0]), "")
		body := lipgloss.NewStyle().Width(min(maxW, 80)).Render(strings.Join(m.message[1:], "\n"))
		bl := strings.Split(body, "\n")
		if len(bl) > H-8 {
			bl = append(bl[:H-8], "…")
		}
		b = append(b, bl...)
		b = append(b, "")
		ok := m.styles.sTabActive.Render("OK (enter)")
		z.add("close", 0, len(b), lipgloss.Width(ok), 1)
		b = append(b, ok)
		return strings.Join(b, "\n")

	case modalHelp:
		return m.helpContent(z)
	}
	return ""
}

func lipglossHeight(lines []string) int {
	return lipgloss.Height(strings.Join(lines, "\n"))
}

func (m *Model) helpContent(z *zones) string {
	sec := func(t string) string { return m.styles.sBold.Foreground(m.styles.cAccent2).Render(t) }
	row := func(k, d string) string { return "  " + m.styles.sKey.Render(fit(k, 16)) + d }
	left := []string{
		sec(m.t("Navegação")),
		row("↑↓ / j k", m.t("mover seleção")),
		row("1-4 / 5-7", m.t("abas de PRs / issues")),
		row("tab", m.t("próxima aba")),
		row("g / G", m.t("início / fim")),
		row("pgup / pgdn", m.t("página")),
		row("/", m.t("filtrar")),
		row(m.t("enter / clique"), m.t("menu de ações")),
		row("r", m.t("atualizar agora")),
		row("i", m.t("instâncias")),
		row("s", m.t("configurações")),
		row("q / ctrl+c", m.t("sair")),
	}
	right := []string{
		sec(m.t("Ações")),
		row("v", m.t("ver conversa (comentários)")),
		row("n", m.t("comentar")),
		row("w", m.t("checkout em worktree")),
		row("c", m.t("checkout no clone local")),
		row("d", "diff (hunk/git)"),
		row("t", m.t("terminal no worktree")),
		row("a / A", m.t("aprovar / + remover wt")),
		row("m / M", m.t("merge / + remover wt")),
		row("X / C", m.t("fechar sem merge / + remover wt")),
		row("x", m.t("remover worktree")),
		row("o", m.t("abrir no navegador")),
		row("R", m.t("todos os PRs/MRs do repo")),
		row("p", m.t("definir pasta local")),
		m.styles.sDim.Render(m.t("  (w…x/R/p só em PRs)")),
	}
	legend := []string{
		"",
		sec(m.t("Legenda")),
		"  " + m.styles.ciIcon(provider.CISuccess) + " pipeline ok  " + m.styles.ciIcon(provider.CIFailure) + m.t(" falhou  ") + m.styles.ciIcon(provider.CIPending) + m.t(" rodando  ") + m.styles.ciIcon(provider.CICanceled) + m.t(" cancelado"),
		"  " + m.styles.sGreen.Render("◉") + m.t(" issue aberta  ") + m.styles.sGreen.Render("✓") + m.t(" você aprovou  ") + m.styles.sGreen.Render("◆") + m.t(" aprovado  ") + m.styles.sRed.Render("±") + m.t(" alterações pedidas  ") + m.styles.sBlue.Render("⎇") + " worktree  " + m.styles.sRed.Render("⚠") + m.t(" conflito"),
		"",
		m.styles.sDim.Render("  Terminal: " + string(m.mode) + " · config: " + m.cfg.FilePath()),
	}
	cols := lipgloss.JoinHorizontal(lipgloss.Top, strings.Join(left, "\n"), "    ", strings.Join(right, "\n"))
	b := append([]string{cols}, legend...)
	h := lipglossHeight(b)
	closeBtn := m.styles.sTabActive.Render(m.t("Fechar (esc)"))
	z.add("close", 0, h+1, lipgloss.Width(closeBtn), 1)
	b = append(b, "", closeBtn)
	return strings.Join(b, "\n")
}
