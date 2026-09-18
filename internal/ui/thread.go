package ui

import (
	"context"
	"fmt"
	"regexp"
	"strings"
	"time"

	"charm.land/bubbles/v2/textarea"
	tea "charm.land/bubbletea/v2"
	"charm.land/glamour/v2"
	"charm.land/lipgloss/v2"

	"github.com/vitorpacheco/pr-tracker/internal/provider"
)

// threadView is the conversation screen of one PR or issue.
type threadView struct {
	item    provider.Item
	thread  *provider.Thread
	err     error
	loading bool
	offset  int

	// Rendered lines, cached for width.
	lines []string
	width int
}

type threadMsg struct {
	key    string
	thread *provider.Thread
	err    error
}

func (m *Model) openThread(it *provider.Item) tea.Cmd {
	m.thread = &threadView{item: *it}
	m.screen = screenThread
	return m.loadThread()
}

func (m *Model) loadThread() tea.Cmd {
	t := m.thread
	client := m.clients[t.item.Instance]
	if client == nil {
		return nil
	}
	t.loading = true
	it := t.item
	return func() tea.Msg {
		ctx, cancel := context.WithTimeout(context.Background(), time.Minute)
		defer cancel()
		th, err := client.Thread(ctx, &it)
		return threadMsg{key: it.Key(), thread: th, err: err}
	}
}

func (m *Model) applyThread(msg threadMsg) {
	t := m.thread
	if t == nil || t.item.Key() != msg.key {
		return
	}
	t.loading = false
	t.thread, t.err = msg.thread, msg.err
	t.lines = nil
}

// threadKey handles keys on the conversation screen; ok=false lets global
// shortcuts (help, settings…) run.
func (m *Model) threadKey(k string) (tea.Cmd, bool) {
	t := m.thread
	page := max(m.listRows-2, 1)
	switch k {
	case "esc", "q", "backspace", "left", "h":
		m.screen = screenPRs
		m.thread = nil
	case "up", "k":
		t.offset--
	case "down", "j":
		t.offset++
	case "pgup", "ctrl+u", "b":
		t.offset -= page
	case "pgdown", "ctrl+d", "space", "f":
		t.offset += page
	case "home", "g":
		t.offset = 0
	case "end", "G":
		t.offset = len(t.lines)
	case "r":
		return m.loadThread(), true
	case "n":
		return m.openCompose(&t.item), true
	case "o":
		m.openBrowser(t.item.URL)
	default:
		return nil, false
	}
	return nil, true
}

var (
	reHTMLComment = regexp.MustCompile(`(?s)<!--.*?-->`)
	reSummary     = regexp.MustCompile(`(?is)<summary>(.*?)</summary>`)
	reHTMLTags    = regexp.MustCompile(`(?i)</?(details|summary|sub|sup|br|p|div|span|b|i|em|strong|img|a|picture|source|table|thead|tbody|tr|td|th|blockquote|kbd|code)\b[^>]*>`)
	reBlankLines  = regexp.MustCompile(`\n{3,}`)
)

// cleanMarkdown drops HTML that terminals cannot render (bot comments are
// full of it) while keeping the text.
func cleanMarkdown(s string) string {
	s = strings.ReplaceAll(s, "\r\n", "\n")
	s = reHTMLComment.ReplaceAllString(s, "")
	s = reSummary.ReplaceAllString(s, "\n**▸ $1**\n")
	s = reHTMLTags.ReplaceAllString(s, "")
	s = reBlankLines.ReplaceAllString(s, "\n\n")
	return strings.TrimSpace(s)
}

func renderMarkdown(r *glamour.TermRenderer, s string, width int) []string {
	s = cleanMarkdown(s)
	if s == "" {
		return nil
	}
	out, err := r.Render(s)
	if err != nil {
		out = lipgloss.NewStyle().Width(width).Render(s)
	}
	lines := strings.Split(strings.Trim(out, "\n"), "\n")
	// glamour pads lines with trailing blanks; trim them so fit() works.
	for i, l := range lines {
		lines[i] = strings.TrimRight(l, " ")
	}
	return lines
}

func (t *threadView) render(width int) []string {
	if t.lines != nil && t.width == width {
		return t.lines
	}
	t.width = width
	it := &t.item
	var out []string
	title := lipgloss.NewStyle().Bold(true).Foreground(lipgloss.Color("#FFFFFF")).Width(width - 2).Render(it.Title)
	for _, l := range strings.Split(title, "\n") {
		out = append(out, " "+l)
	}
	kind := "PR"
	if it.IsIssue() {
		kind = "issue"
	}
	out = append(out, " "+sAccent.Render(it.Repo+" "+it.Ref())+sDim.Render(" · "+kind+" · "+it.Instance+" · "+it.URL))
	if len(it.Labels) > 0 {
		out = append(out, " "+labels(it.Labels))
	}
	switch {
	case t.thread == nil && t.err != nil:
		return append(out, "", sRed.Render(" ✘ "+oneLine(t.err.Error())))
	case t.thread == nil:
		return out // still loading; not cached
	}
	r, err := glamour.NewTermRenderer(glamour.WithStandardStyle("dark"), glamour.WithWordWrap(max(width-6, 20)))
	if err != nil {
		return append(out, sRed.Render(" ✘ "+err.Error()))
	}
	sep := func(head string) string {
		return " " + head + " " + sDim.Render(strings.Repeat("─", max(width-lipgloss.Width(head)-3, 0)))
	}
	out = append(out, "", sep(sBold.Render(it.Author)+sDim.Render(" · descrição · "+age(it.CreatedAt))))
	out = append(out, renderMarkdown(r, t.thread.Body, width)...)
	for _, c := range t.thread.Comments {
		body := renderMarkdown(r, c.Body, width)
		if body == nil && (c.Review == "" || c.Review == "commented") {
			continue // bot markers and empty review wrappers
		}
		head := sBold.Render(firstNonEmpty(c.Author, "ghost")) + sDim.Render(" · "+age(c.CreatedAt))
		switch c.Review {
		case "approved":
			head += " " + sGreen.Render("✔ aprovou")
		case "changes_requested":
			head += " " + sRed.Render("± pediu alterações")
		case "commented":
			head += " " + sMuted.Render("◇ review")
		case "dismissed":
			head += " " + sDim.Render("review descartado")
		}
		if c.Path != "" {
			loc := c.Path
			if c.Line > 0 {
				loc += fmt.Sprintf(":%d", c.Line)
			}
			head += " " + sBlue.Render(loc)
		}
		out = append(out, "", sep(head))
		out = append(out, body...)
	}
	if len(t.thread.Comments) == 0 {
		out = append(out, "", sDim.Render(" nenhum comentário ainda — ")+sKey.Render("n")+sDim.Render(" para comentar"))
	}
	out = append(out, "")
	t.lines = out
	return out
}

func (m *Model) threadBody(W, H int) []string {
	t := m.thread
	m.listRows = H
	lines := t.render(W)
	if t.loading && t.thread == nil {
		lines = append(lines, "", " "+sYellow.Render(spinnerFrames[m.frame%len(spinnerFrames)]+" carregando conversa…"))
	}
	t.offset = max(0, min(t.offset, len(lines)-H))
	end := min(t.offset+H, len(lines))
	out := make([]string, 0, H)
	for _, l := range lines[t.offset:end] {
		out = append(out, fit(l, W-1))
	}
	// Scrollbar in the last column.
	if len(lines) > H {
		pos := t.offset * (H - 1) / max(len(lines)-H, 1)
		for i := range out {
			bar := sDim.Render("│")
			if i == pos {
				bar = sAccent.Render("┃")
			}
			out[i] = fit(out[i], W-1) + bar
		}
	}
	return out
}

func labels(ls []string) string {
	var parts []string
	for _, l := range ls {
		parts = append(parts, sLabelTag.Render(l))
	}
	return strings.Join(parts, " ")
}

// ---------- compose ----------

func (m *Model) openCompose(it *provider.Item) tea.Cmd {
	ta := textarea.New()
	ta.Placeholder = "Escreva o comentário (markdown)…"
	ta.ShowLineNumbers = false
	ta.CharLimit = 65536
	ta.SetWidth(min(max(m.width-12, 30), 90))
	ta.SetHeight(min(max(m.height-14, 4), 12))
	m.compose = &ta
	m.composeFor = *it
	m.modal = modalCompose
	return ta.Focus()
}

func (m *Model) composeKey(msg tea.KeyPressMsg) tea.Cmd {
	switch msg.String() {
	case "esc":
		m.modal, m.compose = modalNone, nil
		return nil
	case "ctrl+s":
		return m.sendComment()
	}
	var cmd tea.Cmd
	*m.compose, cmd = m.compose.Update(msg)
	return cmd
}

func (m *Model) sendComment() tea.Cmd {
	body := strings.TrimSpace(m.compose.Value())
	if body == "" {
		m.setStatus(stErr, "comentário vazio")
		return nil
	}
	it := m.composeFor
	client := m.clients[it.Instance]
	m.modal, m.compose = modalNone, nil
	if client == nil {
		return nil
	}
	key := it.Key()
	m.pending[key] = "enviando comentário"
	return func() tea.Msg {
		ctx, cancel := context.WithTimeout(context.Background(), time.Minute)
		defer cancel()
		if err := client.AddComment(ctx, &it, body); err != nil {
			return actionDoneMsg{key: key, err: err}
		}
		return actionDoneMsg{key: key, ok: "comentário enviado em " + it.Repo + it.Ref(), refresh: true, reloadThread: true}
	}
}

func (m *Model) composeContent(z *zones) string {
	it := &m.composeFor
	var b []string
	b = append(b, sBold.Foreground(cAccent2).Render("Comentar em "+it.Repo+" "+it.Ref()), sDim.Render(fit(it.Title, m.compose.Width())), "")
	b = append(b, m.compose.View(), "")
	send := sTabActive.Render("Enviar ctrl+s")
	cancel := sTab.Render("Cancelar esc")
	y := lipglossHeight(b)
	z.add("compose:send", 0, y, lipgloss.Width(send), 1)
	z.add("compose:cancel", lipgloss.Width(send)+2, y, lipgloss.Width(cancel), 1)
	b = append(b, send+"  "+cancel)
	return strings.Join(b, "\n")
}
