package ui

import (
	"fmt"
	"slices"
	"strings"
	"time"

	tea "charm.land/bubbletea/v2"
	"charm.land/lipgloss/v2"

	"github.com/vitorpacheco/pr-tracker/internal/config"
	"github.com/vitorpacheco/pr-tracker/internal/provider"
)

// View renders the whole screen and registers clickable zones.
func (m *Model) View() tea.View {
	m.zones = m.zones[:0]
	v := tea.NewView("")
	v.AltScreen = true
	v.MouseMode = tea.MouseModeCellMotion
	v.WindowTitle = "PR Tracker"
	if m.width == 0 || m.height == 0 {
		return v
	}
	W, H := m.width, m.height

	var lines []string
	lines = append(lines, m.header(W))
	for _, b := range m.banners() {
		m.zones.add("banner", 0, len(lines), W, 1)
		lines = append(lines, fit(sBanner.Render(" ⚠ "+b), W))
	}
	if m.screen == screenPRs && (m.filtering || m.filter.Value() != "") {
		f := " " + m.filter.View()
		if !m.filtering {
			f = " " + sAccent.Render("/ "+m.filter.Value()) + sDim.Render("  (esc limpa)")
		}
		lines = append(lines, fit(f, W))
	}
	lines = append(lines, sDim.Render(strings.Repeat("─", W)))

	hints := m.hints()
	var probe zones
	footerN := len(hintBar(hints, W-2, 0, 0, &probe))
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
	for _, l := range hintBar(hints, W-2, 1, len(lines), &m.zones) {
		lines = append(lines, " "+l)
	}
	base := strings.Join(lines, "\n")

	if m.modal != modalNone {
		var mz zones
		box := sModal.Render(m.modalContent(W, H, &mz))
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
	title := sTitle.Render("⎇ PR Tracker")
	x := lipgloss.Width(title) + 1
	parts := []string{title, " "}
	if m.screen == screenInstances || m.screen == screenThread {
		name := "Instâncias"
		if m.screen == screenThread {
			name = "Conversa " + m.thread.item.Ref()
		}
		parts = append(parts, sTabActive.Render(name))
		x += lipgloss.Width(parts[len(parts)-1])
		back := sTab.Render("← voltar")
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
				g := sGroup.Render(group) + " "
				x += lipgloss.Width(g)
				parts = append(parts, g)
			}
			label := fmt.Sprintf("%s %s %s", sKey.Render(itoa(i+1)), t.label, sMuted.Render(itoa(m.count(t))))
			if compact && i != m.tab {
				label = fmt.Sprintf("%s %s", sKey.Render(itoa(i+1)), sMuted.Render(itoa(m.count(t))))
			}
			st := sTab
			if i == m.tab {
				st = sTabActive
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
		right = sYellow.Render(spinnerFrames[m.frame%len(spinnerFrames)] + " atualizando")
	case !m.lastSync.IsZero():
		right = sMuted.Render("atualizado há "+duration(time.Since(m.lastSync))) +
			sDim.Render(" · próxima em "+duration(max(time.Until(m.nextSync), 0)))
	}
	right = sKey.Render("r") + " " + right + " "
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
	missing := map[string]bool{}
	for _, in := range m.cfg.Instances {
		if in.Disabled || in.Provider == config.Bitbucket {
			continue
		}
		tool := provider.New(in).Tool()
		if !provider.ToolAvailable(tool) && !missing[tool] {
			missing[tool] = true
			out = append(out, fmt.Sprintf("%s não está instalado e é necessário para %s. Instale: %s", tool, in.Name, provider.InstallHint(tool)))
		}
	}
	var errs []string
	for _, in := range m.cfg.Instances {
		if err := m.instErr[in.Name]; err != nil && !missing[provider.New(in).Tool()] {
			errs = append(errs, in.Name)
		}
	}
	if len(errs) > 0 {
		out = append(out, fmt.Sprintf("falha ao consultar %s — clique ou pressione i para detalhes", strings.Join(errs, ", ")))
	}
	return out
}

func (m *Model) hints() []hint {
	if m.screen == screenThread {
		return []hint{
			{"↑↓", "rolar"}, {"space", "página"}, {"g", "início"}, {"G", "fim"}, {"n", "comentar"},
			{"o", "navegador"}, {"r", "recarregar"}, {"esc", "voltar"}, {"?", "ajuda"},
		}
	}
	if cur := m.current(); m.screen == screenPRs && cur != nil && cur.IsIssue() {
		return []hint{
			{"enter", "ações"}, {"v", "conversa"}, {"n", "comentar"}, {"o", "navegador"},
			{"/", "filtrar"}, {"r", "atualizar"}, {"i", "instâncias"}, {"s", "config"}, {"?", "ajuda"}, {"q", "sair"},
		}
	}
	if m.screen == screenInstances {
		return []hint{
			{"n", "nova"}, {"e", "editar"}, {"space", "ativar/desativar"}, {"t", "testar auth"},
			{"D", "remover"}, {"s", "config"}, {"esc", "voltar"}, {"?", "ajuda"}, {"q", "sair"},
		}
	}
	return []hint{
		{"enter", "ações"}, {"v", "conversa"}, {"n", "comentar"}, {"w", "worktree"}, {"d", "diff"}, {"t", "terminal"}, {"c", "checkout"},
		{"a", "aprovar"}, {"m", "merge"}, {"X", "fechar"}, {"x", "rm worktree"}, {"o", "navegador"},
		{"/", "filtrar"}, {"r", "atualizar"}, {"i", "instâncias"}, {"s", "config"}, {"?", "ajuda"}, {"q", "sair"},
	}
}

func (m *Model) statusLine(W int) string {
	if m.status != "" {
		st := sMuted
		icon := "•"
		switch m.statusKind {
		case stOK:
			st, icon = sGreen, "✔"
		case stErr:
			st, icon = sRed, "✘"
		}
		return fit(" "+st.Render(icon+" "+m.status), W)
	}
	if len(m.pending) > 0 {
		var labels []string
		for _, l := range m.pending {
			labels = append(labels, l)
		}
		slices.Sort(labels)
		return fit(" "+sYellow.Render(spinnerFrames[m.frame%len(spinnerFrames)]+" "+strings.Join(labels, " · ")), W)
	}
	wt, _ := m.cfg.Worktrees()
	issues := m.count(tabDef{kind: provider.KindIssue})
	return fit(sDim.Render(fmt.Sprintf(" %d PRs · %d issues · terminal: %s · worktrees: %s", len(m.prs)-issues, issues, m.mode, wt)), W)
}

// ---------- PR list ----------

func (m *Model) prsBody(W, H, top int) []string {
	if len(m.cfg.Instances) == 0 {
		return centered(W, H, []string{
			sBold.Render("Nenhuma instância configurada"),
			"",
			sMuted.Render("Pressione ") + sKey.Render("i") + sMuted.Render(" e depois ") + sKey.Render("n") + sMuted.Render(" para cadastrar GitHub/GitLab (inclusive self-hosted)."),
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
		list = append(list, centered(listW, H-1, []string{sYellow.Render(spinnerFrames[m.frame%len(spinnerFrames)] + " carregando pull requests…")})...)
	case len(vis) == 0:
		msg := "Nada por aqui ✨"
		if m.filter.Value() != "" {
			msg = "Nenhum PR corresponde ao filtro"
		}
		list = append(list, centered(listW, H-1, []string{sMuted.Render(msg)})...)
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
	sep := sDim.Render("│")
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
			out[pos] = fit(list[pos], listW) + sAccent.Render("┃") + " " + fit(detailAt(detail, pos), detailW-1)
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
	s := "  " + "CI " + "RV " + fit("REPOSITÓRIO", c.repo) + " " + fit("#", c.ref) + " " + fit("TÍTULO", c.title) + " "
	if c.author > 0 {
		s += fit("AUTOR", c.author)
	}
	s += fit("COM", 4) + fit(" ATU.", 5)
	return sDim.Render(fit(s, W))
}

func (m *Model) row(pr *provider.Item, selected bool, W int) string {
	c := m.widths(W)
	marker := "  "
	titleSt := lipgloss.NewStyle().Foreground(cText)
	if selected {
		marker = sAccent.Render("▌ ")
		titleSt = titleSt.Bold(true).Foreground(lipgloss.Color("#FFFFFF"))
	}
	repo := pr.Repo
	if lipgloss.Width(repo) > c.repo {
		repo = "…" + string([]rune(repo)[len([]rune(repo))-c.repo+1:])
	}
	title := titleSt.Render(pr.Title)
	if pr.Draft {
		title = sMuted.Render("[draft] ") + title
	}
	if len(pr.Labels) > 0 {
		title += " " + sDim.Render(strings.Join(pr.Labels, " · "))
	}
	ci := ciIcon(pr.CI)
	if pr.IsIssue() {
		ci = sGreen.Render("◉")
	}
	comments := ""
	if pr.Comments > 0 {
		comments = itoa(pr.Comments)
	}
	var flags []string
	if _, busy := m.pending[pr.Key()]; busy {
		flags = append(flags, sYellow.Render(spinnerFrames[m.frame%len(spinnerFrames)]))
	}
	if _, ok := m.wts[pr.Key()]; ok {
		flags = append(flags, sBlue.Render("⎇"))
	}
	if pr.Conflicts {
		flags = append(flags, sRed.Render("⚠"))
	}
	s := marker + ci + "  " + reviewIcon(pr) + "  " +
		fit(sMuted.Render(repo), c.repo) + " " +
		fit(sAccent.Render(pr.Ref()), c.ref) + " " +
		fit(title, c.title) + " "
	if c.author > 0 {
		s += fit(sMuted.Render(pr.Author), c.author-1) + " "
	}
	s += fit(sMuted.Render(comments), 4) + fit(sDim.Render(" "+age(pr.UpdatedAt)), 5) + strings.Join(flags, "")
	if selected {
		return sRowSel.Render(fit(s, W))
	}
	return fit(s, W)
}

func (m *Model) detail(pr *provider.Item, W, H int) []string {
	if pr == nil {
		return nil
	}
	var d []string
	title := lipgloss.NewStyle().Bold(true).Foreground(lipgloss.Color("#FFFFFF")).Width(W).Render(pr.Title)
	tl := strings.Split(title, "\n")
	if len(tl) > 3 {
		tl = append(tl[:3], "…")
	}
	d = append(d, tl...)
	d = append(d, sAccent.Render(pr.Repo+" "+pr.Ref())+sDim.Render(" · "+pr.Instance))
	d = append(d, sDim.Render(pr.URL), "")

	kv := func(k, v string) { d = append(d, sLabel.Render(k)+v) }
	kv("Autor", pr.Author)
	if len(pr.Assignees) > 0 {
		kv("Atribuído a", strings.Join(pr.Assignees, ", "))
	}
	if len(pr.Labels) > 0 {
		kv("Labels", labels(pr.Labels))
	}
	kv("Comentários", fmt.Sprintf("%d ", pr.Comments)+sKey.Render("v")+sMuted.Render(" ver · ")+sKey.Render("n")+sMuted.Render(" comentar"))
	if pr.IsIssue() {
		kv("Você é", relations(pr))
		kv("Atualizado", age(pr.UpdatedAt)+sDim.Render(" · criada "+age(pr.CreatedAt)))
		if l, ok := m.pending[pr.Key()]; ok {
			kv("Executando", sYellow.Render(spinnerFrames[m.frame%len(spinnerFrames)]+" "+l))
		}
		return d
	}
	branch := sBlue.Render(pr.SourceBranch) + sDim.Render(" → ") + pr.TargetBranch
	if pr.FromFork {
		branch += sYellow.Render(" (fork)")
	}
	kv("Branch", branch)
	state := sGreen.Render("aberto")
	if pr.Draft {
		state = sMuted.Render("rascunho")
	}
	if pr.Conflicts {
		state += sRed.Render(" · com conflitos")
	}
	kv("Estado", state)
	rev := reviewLabel(pr)
	if pr.ApprovedByMe {
		rev += sGreen.Render(" · você aprovou")
	}
	kv("Revisão", rev)
	if len(pr.ApprovedBy) > 0 {
		kv("Aprovado por", strings.Join(pr.ApprovedBy, ", "))
	}
	kv("Você é", relations(pr))
	if pr.Additions+pr.Deletions+pr.Files > 0 {
		kv("Mudanças", sGreen.Render(fmt.Sprintf("+%d", pr.Additions))+" "+sRed.Render(fmt.Sprintf("-%d", pr.Deletions))+sMuted.Render(fmt.Sprintf(" em %d arquivos", pr.Files)))
	}
	kv("Atualizado", age(pr.UpdatedAt)+sDim.Render(" · criado "+age(pr.CreatedAt)))
	if c := m.clones[pr.Key()]; c != "" {
		kv("Clone", c)
	} else {
		kv("Clone", sYellow.Render("não configurado ")+sKey.Render("p")+sMuted.Render(" define"))
	}
	if wt, ok := m.wts[pr.Key()]; ok {
		kv("Worktree", sBlue.Render(wt))
	} else {
		kv("Worktree", sDim.Render("— ")+sKey.Render("w")+sMuted.Render(" cria"))
	}
	if l, ok := m.pending[pr.Key()]; ok {
		kv("Executando", sYellow.Render(spinnerFrames[m.frame%len(spinnerFrames)]+" "+l))
	}
	d = append(d, "", sLabel.Render("Pipeline")+ciLabel(pr.CI))
	checks := slices.Clone(pr.Checks)
	rank := map[provider.CIState]int{provider.CIFailure: 0, provider.CIPending: 1, provider.CICanceled: 2, provider.CISuccess: 3, provider.CINone: 4}
	slices.SortStableFunc(checks, func(a, b provider.Check) int { return rank[a.State] - rank[b.State] })
	room := H - len(d)
	for i, c := range checks {
		if i >= room-1 && len(checks) > room {
			d = append(d, sDim.Render(fmt.Sprintf("  … mais %d", len(checks)-i)))
			break
		}
		d = append(d, "  "+ciIcon(c.State)+" "+c.Name)
	}
	return d
}

func relations(it *provider.Item) string {
	var rel []string
	if it.Relations&provider.ReviewRequested != 0 {
		rel = append(rel, "revisor")
	}
	if it.Relations&provider.Authored != 0 {
		rel = append(rel, "autor")
	}
	if it.Relations&provider.Assigned != 0 {
		rel = append(rel, "atribuído")
	}
	if it.Relations&provider.Mentioned != 0 {
		rel = append(rel, "mencionado")
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
			sBold.Render("Nenhuma instância"),
			"",
			sMuted.Render("Pressione ") + sKey.Render("n") + sMuted.Render(" para adicionar github.com, gitlab.com ou uma instância self-hosted."),
			sDim.Render("A autenticação usa os próprios gh / glab (gh auth login --hostname …)."),
		})
	}
	out = append(out, sDim.Render(fit("  "+fit("NOME", 22)+fit("PROVIDER", 11)+fit("HOST", 30)+fit("CLI", 7)+"STATUS", W)))
	for i, in := range m.cfg.Instances {
		cl := provider.New(in)
		tool := cl.Tool()
		toolSt := sGreen.Render(tool)
		if !provider.ToolAvailable(tool) {
			toolSt = sRed.Render(tool + "✘")
		}
		var status string
		switch {
		case in.Provider == config.Bitbucket:
			status = sYellow.Render("não suportado ainda")
		case in.Disabled:
			status = sDim.Render("desativada")
		case !provider.ToolAvailable(tool):
			status = sRed.Render("CLI não instalado")
		case m.instErr[in.Name] != nil:
			status = sRed.Render("erro")
		case m.lastSync.IsZero():
			status = sMuted.Render("…")
		default:
			n := 0
			for _, p := range m.prs {
				if p.Instance == in.Name {
					n++
				}
			}
			status = sGreen.Render(fmt.Sprintf("ok · %d PRs", n))
		}
		marker := "  "
		if i == m.instCur {
			marker = sAccent.Render("▌ ")
		}
		row := marker + fit(sBold.Render(in.Name), 22) + fit(sAccent.Render(string(in.Provider)), 11) + fit(in.Host, 30) + fit(toolSt, 7) + status
		m.zones.add("inst:"+itoa(i), 0, top+len(out), W, 1)
		if i == m.instCur {
			row = sRowSel.Render(fit(row, W))
		}
		out = append(out, fit(row, W))
	}
	if m.instCur < len(m.cfg.Instances) {
		in := m.cfg.Instances[m.instCur]
		cl := provider.New(in)
		out = append(out, "", sDim.Render(strings.Repeat("─", W)))
		kv := func(k, v string) { out = append(out, " "+sLabel.Render(k)+v) }
		kv("Merge", firstNonEmpty(in.MergeMethod, "merge")+sDim.Render(fmt.Sprintf(" · auto-merge %v · apagar branch %v", in.AutoMerge, in.DeleteBranch)))
		repos := 0
		for _, r := range m.cfg.Repos {
			if r.Instance == in.Name {
				repos++
			}
		}
		kv("Repos locais", fmt.Sprint(repos))
		if !provider.ToolAvailable(cl.Tool()) {
			kv("CLI", sRed.Render(cl.Tool()+" não instalado — "+provider.InstallHint(cl.Tool())))
		}
		if in.Provider == config.Bitbucket {
			kv("Bitbucket", sYellow.Render("sem CLI oficial; integração adiada (docs/bitbucket.md)"))
		}
		if err := m.instErr[in.Name]; err != nil {
			wrapped := lipgloss.NewStyle().Width(max(W-16, 20)).Render(err.Error())
			for j, l := range strings.Split(wrapped, "\n") {
				if j == 0 {
					kv("Erro", sRed.Render(l))
				} else {
					out = append(out, " "+spaces(12)+sRed.Render(l))
				}
			}
		}
		if in.Provider == config.GitHub {
			kv("Login", sDim.Render("gh auth login --hostname "+in.Host))
		} else if in.Provider == config.GitLab {
			kv("Login", sDim.Render("glab auth login --hostname "+in.Host))
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
		b = append(b, sBold.Foreground(cAccent2).Render(fit(m.menuTitle, min(maxW, 64))), "")
		for i, it := range m.menu {
			label := it.label
			line := " " + sKey.Render(fit(it.key, 2)) + " " + label
			if it.disabled != "" {
				line = " " + sDim.Render(fit(it.key, 2)+" "+label)
			}
			line = fit(line, min(maxW, 64))
			if i == m.menuCur {
				line = sRowSel.Render(line)
			}
			z.add("menu:"+itoa(i), 0, len(b), min(maxW, 64), 1)
			b = append(b, line)
		}
		b = append(b, "", sDim.Render("↑↓ navega · enter/atalho executa · esc fecha"))
		return strings.Join(b, "\n")

	case modalConfirm:
		c := m.confirm
		var b []string
		b = append(b, sBold.Foreground(cAccent2).Render(c.title), "")
		for _, l := range c.body {
			b = append(b, lipgloss.NewStyle().Width(min(maxW, 70)).Render(l))
		}
		b = append(b, "")
		yes := sTabActive.Render(c.yes + " (y/enter)")
		no := sTab.Render("Cancelar (n/esc)")
		z.add("confirm:yes", 0, lipglossHeight(b), lipgloss.Width(yes), 1)
		z.add("confirm:no", lipgloss.Width(yes)+2, lipglossHeight(b), lipgloss.Width(no), 1)
		b = append(b, yes+"  "+no)
		return strings.Join(b, "\n")

	case modalForm:
		return m.form.render(z)

	case modalCompose:
		return m.composeContent(z)

	case modalMessage:
		var b []string
		b = append(b, sRed.Bold(true).Render(m.message[0]), "")
		body := lipgloss.NewStyle().Width(min(maxW, 80)).Render(strings.Join(m.message[1:], "\n"))
		bl := strings.Split(body, "\n")
		if len(bl) > H-8 {
			bl = append(bl[:H-8], "…")
		}
		b = append(b, bl...)
		b = append(b, "")
		ok := sTabActive.Render("OK (enter)")
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
	sec := func(t string) string { return sBold.Foreground(cAccent2).Render(t) }
	row := func(k, d string) string { return "  " + sKey.Render(fit(k, 16)) + d }
	left := []string{
		sec("Navegação"),
		row("↑↓ / j k", "mover seleção"),
		row("1-4 / 5-7", "abas de PRs / issues"),
		row("tab", "próxima aba"),
		row("g / G", "início / fim"),
		row("pgup / pgdn", "página"),
		row("/", "filtrar"),
		row("enter / clique", "menu de ações"),
		row("r", "atualizar agora"),
		row("i", "instâncias"),
		row("s", "configurações"),
		row("q / ctrl+c", "sair"),
	}
	right := []string{
		sec("Ações"),
		row("v", "ver conversa (comentários)"),
		row("n", "comentar"),
		row("w", "checkout em worktree"),
		row("c", "checkout no clone local"),
		row("d", "diff (hunk/git)"),
		row("t", "terminal no worktree"),
		row("a / A", "aprovar / + remover wt"),
		row("m / M", "merge / + remover wt"),
		row("X / C", "fechar sem merge / + remover wt"),
		row("x", "remover worktree"),
		row("o", "abrir no navegador"),
		row("p", "definir pasta local"),
		sDim.Render("  (w…x/p só em PRs)"),
	}
	legend := []string{
		"",
		sec("Legenda"),
		"  " + ciIcon(provider.CISuccess) + " pipeline ok  " + ciIcon(provider.CIFailure) + " falhou  " + ciIcon(provider.CIPending) + " rodando  " + ciIcon(provider.CICanceled) + " cancelado",
		"  " + sGreen.Render("◉") + " issue aberta  " + sGreen.Render("✓") + " você aprovou  " + sGreen.Render("◆") + " aprovado  " + sRed.Render("±") + " alterações pedidas  " + sBlue.Render("⎇") + " worktree  " + sRed.Render("⚠") + " conflito",
		"",
		sDim.Render("  Terminal: " + string(m.mode) + " · config: " + m.cfg.FilePath()),
	}
	cols := lipgloss.JoinHorizontal(lipgloss.Top, strings.Join(left, "\n"), "    ", strings.Join(right, "\n"))
	b := append([]string{cols}, legend...)
	h := lipglossHeight(b)
	closeBtn := sTabActive.Render("Fechar (esc)")
	z.add("close", 0, h+1, lipgloss.Width(closeBtn), 1)
	b = append(b, "", closeBtn)
	return strings.Join(b, "\n")
}
