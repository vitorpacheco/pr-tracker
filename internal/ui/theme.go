package ui

import (
	"charm.land/bubbles/v2/textarea"
	"charm.land/bubbles/v2/textinput"
	"charm.land/glamour/v2"
	"charm.land/glamour/v2/ansi"
	glamourstyles "charm.land/glamour/v2/styles"
	"charm.land/lipgloss/v2"
)

func (st *styles) input(input *textinput.Model) {
	s := textinput.DefaultStyles(st.palette.Mode != "light")
	if st.palette.Name != "" {
		for _, state := range []*textinput.StyleState{&s.Focused, &s.Blurred} {
			state.Text = lipgloss.NewStyle().Foreground(st.cText)
			state.Placeholder = st.sMuted
			state.Suggestion = st.sMuted
			state.Prompt = st.sAccent
		}
		s.Cursor.Color = st.cAccent
	}
	input.SetStyles(s)
}

func (st *styles) textarea(input *textarea.Model) {
	s := textarea.DefaultStyles(st.palette.Mode != "light")
	if st.palette.Name != "" {
		for _, state := range []*textarea.StyleState{&s.Focused, &s.Blurred} {
			state.Base = lipgloss.NewStyle().Foreground(st.cText).Background(st.cBlack)
			state.Text = lipgloss.NewStyle().Foreground(st.cText)
			state.Placeholder = st.sMuted
			state.Prompt = st.sAccent
			state.LineNumber = st.sMuted
			state.CursorLineNumber = st.sAccent
			state.CursorLine = lipgloss.NewStyle().Background(st.cSel)
			state.EndOfBuffer = st.sDim
			state.Selection = st.sRowSel
		}
		s.Cursor.Color = st.cAccent
	}
	input.SetStyles(s)
}

func (st *styles) markdownStyle() glamour.TermRendererOption {
	p := st.palette
	if p.Name == "" {
		return glamour.WithStandardStyle("dark")
	}
	s := glamourstyles.DarkStyleConfig
	if p.Mode == "light" {
		s = glamourstyles.LightStyleConfig
	}
	for _, block := range []*ansi.StyleBlock{&s.Document, &s.BlockQuote, &s.Paragraph, &s.List.StyleBlock, &s.Heading, &s.H1, &s.H2, &s.H3, &s.H4, &s.H5, &s.H6, &s.Code, &s.CodeBlock.StyleBlock, &s.Table.StyleBlock, &s.DefinitionList, &s.HTMLBlock, &s.HTMLSpan} {
		block.Color = &p.Foreground
		block.BackgroundColor = nil
	}
	for _, primitive := range []*ansi.StylePrimitive{&s.Text, &s.Strikethrough, &s.Emph, &s.Strong, &s.HorizontalRule, &s.Item, &s.Enumeration, &s.Task.StylePrimitive, &s.Link, &s.LinkText, &s.Image, &s.ImageText, &s.DefinitionTerm, &s.DefinitionDescription} {
		primitive.Color = &p.Foreground
		primitive.BackgroundColor = nil
	}
	for _, heading := range []*ansi.StyleBlock{&s.Heading, &s.H1, &s.H2, &s.H3, &s.H4, &s.H5, &s.H6} {
		color := firstNonEmpty(p.Heading, p.Accent)
		heading.Color = &color
	}
	s.Link.Color = &p.Blue
	s.LinkText.Color = &p.Accent
	s.Code.Color = &p.Accent
	s.Code.BackgroundColor = &p.Raised
	s.HorizontalRule.Color = &p.Border
	s.CodeBlock.Theme = ""
	s.CodeBlock.Chroma = &ansi.Chroma{
		Text:          ansi.StylePrimitive{Color: &p.Foreground},
		Background:    ansi.StylePrimitive{BackgroundColor: &p.Raised},
		Comment:       ansi.StylePrimitive{Color: &p.Muted},
		Keyword:       ansi.StylePrimitive{Color: &p.Accent},
		LiteralString: ansi.StylePrimitive{Color: &p.Green},
		LiteralNumber: ansi.StylePrimitive{Color: &p.Yellow},
		NameFunction:  ansi.StylePrimitive{Color: &p.Blue},
		Error:         ansi.StylePrimitive{Color: &p.Red},
	}
	return glamour.WithStyles(s)
}
