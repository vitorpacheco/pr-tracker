package ui

import (
	"fmt"
	"time"

	"charm.land/lipgloss/v2"
	"github.com/charmbracelet/x/ansi"

	"github.com/vitorpacheco/pr-tracker/internal/provider"
)

var (
	cAccent  = lipgloss.Color("#8B5CF6")
	cAccent2 = lipgloss.Color("#A78BFA")
	cGreen   = lipgloss.Color("#22C55E")
	cRed     = lipgloss.Color("#EF4444")
	cYellow  = lipgloss.Color("#EAB308")
	cBlue    = lipgloss.Color("#38BDF8")
	cMuted   = lipgloss.Color("#7C8594")
	cDim     = lipgloss.Color("#4B5263")
	cSel     = lipgloss.Color("#2E2A47")
	cText    = lipgloss.Color("#E5E7EB")
	cBlack   = lipgloss.Color("#111111")
)

var (
	sTitle     = lipgloss.NewStyle().Bold(true).Foreground(cBlack).Background(cAccent).Padding(0, 1)
	sTab       = lipgloss.NewStyle().Foreground(cMuted).Padding(0, 1)
	sTabActive = lipgloss.NewStyle().Foreground(cText).Background(cSel).Bold(true).Padding(0, 1)
	sKey       = lipgloss.NewStyle().Foreground(cAccent2).Bold(true)
	sMuted     = lipgloss.NewStyle().Foreground(cMuted)
	sDim       = lipgloss.NewStyle().Foreground(cDim)
	sBold      = lipgloss.NewStyle().Bold(true)
	sGreen     = lipgloss.NewStyle().Foreground(cGreen)
	sRed       = lipgloss.NewStyle().Foreground(cRed)
	sYellow    = lipgloss.NewStyle().Foreground(cYellow)
	sBlue      = lipgloss.NewStyle().Foreground(cBlue)
	sAccent    = lipgloss.NewStyle().Foreground(cAccent2)
	sRowSel    = lipgloss.NewStyle().Background(cSel)
	sLabel     = lipgloss.NewStyle().Foreground(cMuted).Width(14)
	sModal     = lipgloss.NewStyle().Border(lipgloss.RoundedBorder()).BorderForeground(cAccent).Padding(0, 1)
	sBanner    = lipgloss.NewStyle().Foreground(cYellow)
	sLabelTag  = lipgloss.NewStyle().Foreground(cAccent2).Background(lipgloss.Color("#2A2540")).Padding(0, 1)
	sGroup     = lipgloss.NewStyle().Foreground(cDim).Bold(true)
)

var spinnerFrames = []string{"⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"}

func ciIcon(s provider.CIState) string {
	switch s {
	case provider.CISuccess:
		return sGreen.Render("✔")
	case provider.CIFailure:
		return sRed.Render("✘")
	case provider.CIPending:
		return sYellow.Render("●")
	case provider.CICanceled:
		return sMuted.Render("⊘")
	}
	return sDim.Render("·")
}

func ciLabel(s provider.CIState) string {
	switch s {
	case provider.CISuccess:
		return sGreen.Render("✔ sucesso")
	case provider.CIFailure:
		return sRed.Render("✘ falhou")
	case provider.CIPending:
		return sYellow.Render("● em andamento")
	case provider.CICanceled:
		return sMuted.Render("⊘ cancelado")
	}
	return sDim.Render("sem pipeline")
}

func reviewIcon(pr *provider.Item) string {
	switch {
	case pr.IsIssue():
		return " "
	case pr.ApprovedByMe:
		return sGreen.Render("✓")
	case pr.Review == provider.ReviewApproved:
		return sGreen.Render("◆")
	case pr.Review == provider.ReviewChangesRequested:
		return sRed.Render("±")
	}
	return sDim.Render("◇")
}

func reviewLabel(pr *provider.Item) string {
	switch pr.Review {
	case provider.ReviewApproved:
		return sGreen.Render("aprovado")
	case provider.ReviewChangesRequested:
		return sRed.Render("alterações solicitadas")
	case provider.ReviewRequired:
		return sYellow.Render("aguardando revisão")
	}
	return sDim.Render("—")
}

func age(t time.Time) string {
	if t.IsZero() {
		return ""
	}
	d := time.Since(t)
	switch {
	case d < time.Minute:
		return "agora"
	case d < time.Hour:
		return fmt.Sprintf("%dm", int(d.Minutes()))
	case d < 48*time.Hour:
		return fmt.Sprintf("%dh", int(d.Hours()))
	case d < 60*24*time.Hour:
		return fmt.Sprintf("%dd", int(d.Hours()/24))
	case d < 365*24*time.Hour:
		return fmt.Sprintf("%dmo", int(d.Hours()/24/30))
	}
	return fmt.Sprintf("%da", int(d.Hours()/24/365))
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
