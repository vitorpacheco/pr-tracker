package ui

import (
	"fmt"
	"image/color"
	"time"

	"charm.land/lipgloss/v2"
	"github.com/charmbracelet/x/ansi"

	"github.com/vitorpacheco/pr-tracker/internal/provider"
	"github.com/vitorpacheco/pr-tracker/internal/theme"
)

type styles struct {
	palette    theme.Palette
	cAccent    color.Color
	cAccent2   color.Color
	cGreen     color.Color
	cRed       color.Color
	cYellow    color.Color
	cBlue      color.Color
	cMuted     color.Color
	cDim       color.Color
	cSel       color.Color
	cText      color.Color
	cBlack     color.Color
	sTitle     lipgloss.Style
	sTab       lipgloss.Style
	sTabActive lipgloss.Style
	sKey       lipgloss.Style
	sMuted     lipgloss.Style
	sDim       lipgloss.Style
	sBold      lipgloss.Style
	sGreen     lipgloss.Style
	sRed       lipgloss.Style
	sYellow    lipgloss.Style
	sBlue      lipgloss.Style
	sAccent    lipgloss.Style
	sRowSel    lipgloss.Style
	sLabel     lipgloss.Style
	sModal     lipgloss.Style
	sBanner    lipgloss.Style
	sLabelTag  lipgloss.Style
	sGroup     lipgloss.Style
}

func newStyles(p theme.Palette) *styles {
	cAccent := lipgloss.Color("#8B5CF6")
	cAccent2 := lipgloss.Color("#A78BFA")
	cGreen := lipgloss.Color("#22C55E")
	cRed := lipgloss.Color("#EF4444")
	cYellow := lipgloss.Color("#EAB308")
	cBlue := lipgloss.Color("#38BDF8")
	cMuted := lipgloss.Color("#7C8594")
	cDim := lipgloss.Color("#4B5263")
	cSel := lipgloss.Color("#2E2A47")
	cText := lipgloss.Color("#E5E7EB")
	cBlack := lipgloss.Color("#111111")

	if p.Name != "" {
		cAccent = lipgloss.Color(p.Accent)
		cAccent2 = lipgloss.Color(firstNonEmpty(p.AccentAlt, p.Accent))
		cGreen = lipgloss.Color(p.Green)
		cRed = lipgloss.Color(p.Red)
		cYellow = lipgloss.Color(p.Yellow)
		cBlue = lipgloss.Color(p.Blue)
		cMuted = lipgloss.Color(p.Muted)
		cDim = lipgloss.Color(firstNonEmpty(p.Dim, p.Muted))
		cSel = lipgloss.Color(p.Selection)
		cText = lipgloss.Color(p.Foreground)
		cBlack = lipgloss.Color(p.Background)
	}

	sTitle := lipgloss.NewStyle().Bold(true).Foreground(cBlack).Background(cAccent).Padding(0, 1)
	sTab := lipgloss.NewStyle().Foreground(cMuted).Padding(0, 1)
	sTabActive := lipgloss.NewStyle().Foreground(cText).Background(cSel).Bold(true).Padding(0, 1)
	sKey := lipgloss.NewStyle().Foreground(cAccent2).Bold(true)
	sMuted := lipgloss.NewStyle().Foreground(cMuted)
	sDim := lipgloss.NewStyle().Foreground(cDim)
	sBold := lipgloss.NewStyle().Bold(true)
	sGreen := lipgloss.NewStyle().Foreground(cGreen)
	sRed := lipgloss.NewStyle().Foreground(cRed)
	sYellow := lipgloss.NewStyle().Foreground(cYellow)
	sBlue := lipgloss.NewStyle().Foreground(cBlue)
	sAccent := lipgloss.NewStyle().Foreground(cAccent2)
	sRowSel := lipgloss.NewStyle().Background(cSel)
	sLabel := lipgloss.NewStyle().Foreground(cMuted).Width(14)
	sModal := lipgloss.NewStyle().Border(lipgloss.RoundedBorder()).BorderForeground(cAccent).Padding(0, 1)
	sBanner := lipgloss.NewStyle().Foreground(cYellow)
	sLabelTag := lipgloss.NewStyle().Foreground(cAccent2).Background(lipgloss.Color("#2A2540")).Padding(0, 1)
	sGroup := lipgloss.NewStyle().Foreground(cDim).Bold(true)

	if p.Name != "" {
		sTabActive = sTabActive.Foreground(lipgloss.Color(p.SelectionText))
		sRowSel = sRowSel.Foreground(lipgloss.Color(p.SelectionText))
		sLabelTag = sLabelTag.Background(lipgloss.Color(firstNonEmpty(p.LabelBackground, p.Selection)))
		sTitle = sTitle.Foreground(lipgloss.Color(firstNonEmpty(p.TitleText, p.Background)))
		sModal = sModal.Background(cBlack).Foreground(cText)
	}
	return &styles{palette: p,
		cAccent:    cAccent,
		cAccent2:   cAccent2,
		cGreen:     cGreen,
		cRed:       cRed,
		cYellow:    cYellow,
		cBlue:      cBlue,
		cMuted:     cMuted,
		cDim:       cDim,
		cSel:       cSel,
		cText:      cText,
		cBlack:     cBlack,
		sTitle:     sTitle,
		sTab:       sTab,
		sTabActive: sTabActive,
		sKey:       sKey,
		sMuted:     sMuted,
		sDim:       sDim,
		sBold:      sBold,
		sGreen:     sGreen,
		sRed:       sRed,
		sYellow:    sYellow,
		sBlue:      sBlue,
		sAccent:    sAccent,
		sRowSel:    sRowSel,
		sLabel:     sLabel,
		sModal:     sModal,
		sBanner:    sBanner,
		sLabelTag:  sLabelTag,
		sGroup:     sGroup,
	}
}

var spinnerFrames = []string{"⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"}

func (st *styles) ciIcon(s provider.CIState) string {
	switch s {
	case provider.CISuccess:
		return st.sGreen.Render("✔")
	case provider.CIFailure:
		return st.sRed.Render("✘")
	case provider.CIPending:
		return st.sYellow.Render("●")
	case provider.CICanceled:
		return st.sMuted.Render("⊘")
	}
	return st.sDim.Render("·")
}

func (st *styles) ciLabel(s provider.CIState, tr func(string) string) string {
	switch s {
	case provider.CISuccess:
		return st.sGreen.Render(tr("✔ sucesso"))
	case provider.CIFailure:
		return st.sRed.Render(tr("✘ falhou"))
	case provider.CIPending:
		return st.sYellow.Render(tr("● em andamento"))
	case provider.CICanceled:
		return st.sMuted.Render(tr("⊘ cancelado"))
	}
	return st.sDim.Render(tr("sem pipeline"))
}

func (st *styles) reviewIcon(pr *provider.Item) string {
	switch {
	case pr.IsIssue():
		return " "
	case pr.ApprovedByMe:
		return st.sGreen.Render("✓")
	case pr.Review == provider.ReviewApproved:
		return st.sGreen.Render("◆")
	case pr.Review == provider.ReviewChangesRequested:
		return st.sRed.Render("±")
	}
	return st.sDim.Render("◇")
}

func (st *styles) reviewLabel(pr *provider.Item, tr func(string) string) string {
	switch pr.Review {
	case provider.ReviewApproved:
		return st.sGreen.Render(tr("aprovado"))
	case provider.ReviewChangesRequested:
		return st.sRed.Render(tr("alterações solicitadas"))
	case provider.ReviewRequired:
		return st.sYellow.Render(tr("aguardando revisão"))
	}
	return st.sDim.Render("—")
}

func age(t time.Time, tr func(string) string) string {
	if t.IsZero() {
		return ""
	}
	d := time.Since(t)
	switch {
	case d < time.Minute:
		return tr("agora")
	case d < time.Hour:
		return fmt.Sprintf("%dm", int(d.Minutes()))
	case d < 48*time.Hour:
		return fmt.Sprintf("%dh", int(d.Hours()))
	case d < 60*24*time.Hour:
		return fmt.Sprintf("%dd", int(d.Hours()/24))
	case d < 365*24*time.Hour:
		return fmt.Sprintf("%dmo", int(d.Hours()/24/30))
	}
	return fmt.Sprintf(tr("%da"), int(d.Hours()/24/365))
}

func duration(d time.Duration) string {
	switch {
	case d < time.Minute:
		return fmt.Sprintf("%ds", int(d.Seconds()))
	case d < time.Hour:
		return fmt.Sprintf("%dm", int(d.Minutes()))
	}
	return fmt.Sprintf("%dh%02dm", int(d.Hours()), int(d.Minutes())%60)
}

// fit truncates s to w cells and pads it to exactly w cells.
func fit(s string, w int) string {
	if w <= 0 {
		return ""
	}
	s = ansi.Truncate(s, w, "…")
	if pad := w - lipgloss.Width(s); pad > 0 {
		s += spaces(pad)
	}
	return s
}

func spaces(n int) string {
	if n <= 0 {
		return ""
	}
	b := make([]byte, n)
	for i := range b {
		b[i] = ' '
	}
	return string(b)
}
