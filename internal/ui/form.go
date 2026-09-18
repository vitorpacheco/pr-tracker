package ui

import (
	"slices"
	"strings"

	"charm.land/bubbles/v2/textinput"
	tea "charm.land/bubbletea/v2"
	"charm.land/lipgloss/v2"
)

type fieldKind int

const (
	fieldText fieldKind = iota
	fieldChoice
	fieldBool
)

type field struct {
	label   string
	kind    fieldKind
	help    string
	input   textinput.Model
	options []string
	choice  int
	on      bool
}

func textField(label, value, placeholder, help string) *field {
	ti := textinput.New()
	ti.Prompt = ""
	ti.Placeholder = placeholder
	ti.SetValue(value)
	ti.SetWidth(44)
	ti.CharLimit = 512
	return &field{label: label, kind: fieldText, input: ti, help: help}
}

func choiceField(label string, options []string, value, help string) *field {
	i := max(slices.Index(options, value), 0)
	return &field{label: label, kind: fieldChoice, options: options, choice: i, help: help}
}

func boolField(label string, on bool, help string) *field {
	return &field{label: label, kind: fieldBool, on: on, help: help}
}

func (f *field) value() string {
	switch f.kind {
	case fieldChoice:
		return f.options[f.choice]
	case fieldBool:
		if f.on {
			return "true"
		}
		return "false"
	}
	return strings.TrimSpace(f.input.Value())
}

// form is a modal with labelled fields and Save/Cancel buttons.
type form struct {
	title  string
	fields []*field
	focus  int
	err    string
	submit func(f *form) error
}

func (f *form) get(label string) *field {
	for _, fl := range f.fields {
		if fl.label == label {
			return fl
		}
	}
	return nil
}

func (f *form) setFocus(i int) tea.Cmd {
	n := len(f.fields)
	f.focus = (i%n + n) % n
	var cmd tea.Cmd
	for j, fl := range f.fields {
		if fl.kind != fieldText {
			continue
		}
		if j == f.focus {
			cmd = fl.input.Focus()
		} else {
			fl.input.Blur()
		}
	}
	return cmd
}

// formResult tells the model what happened after a key.
type formResult int

const (
	formContinue formResult = iota
	formCancel
	formSubmit
)

func (f *form) update(msg tea.KeyPressMsg) (formResult, tea.Cmd) {
	fl := f.fields[f.focus]
	switch msg.String() {
	case "esc":
		return formCancel, nil
	case "ctrl+s":
		return formSubmit, nil
	case "tab", "down":
		return formContinue, f.setFocus(f.focus + 1)
	case "shift+tab", "up":
		return formContinue, f.setFocus(f.focus - 1)
	case "enter":
		if f.focus == len(f.fields)-1 {
			return formSubmit, nil
		}
		return formContinue, f.setFocus(f.focus + 1)
	}
	switch fl.kind {
	case fieldChoice:
		switch msg.String() {
		case "left", "h":
			fl.choice = (fl.choice - 1 + len(fl.options)) % len(fl.options)
		case "right", "l", "space":
			fl.choice = (fl.choice + 1) % len(fl.options)
		}
	case fieldBool:
		switch msg.String() {
		case "space", "left", "right", "h", "l", "x":
			fl.on = !fl.on
		}
	case fieldText:
		var cmd tea.Cmd
		fl.input, cmd = fl.input.Update(msg)
		return formContinue, cmd
	}
	return formContinue, nil
}

// click handles a zone id belonging to the form.
func (f *form) click(id string, idx int) (formResult, tea.Cmd) {
	switch id {
	case "form:save":
		return formSubmit, nil
	case "form:cancel":
		return formCancel, nil
	case "form:field":
		cmd := f.setFocus(idx)
		fl := f.fields[idx]
		switch fl.kind {
		case fieldChoice:
			fl.choice = (fl.choice + 1) % len(fl.options)
		case fieldBool:
			fl.on = !fl.on
		}
		return formContinue, cmd
	}
	return formContinue, nil
}

// render returns the modal body; zones are relative to the box's top-left
// content origin and are offset by the caller.
func (f *form) render(z *zones) string {
	const labelW = 18
	var b []string
	b = append(b, sBold.Foreground(cAccent2).Render(f.title), "")
	for i, fl := range f.fields {
		focused := i == f.focus
		label := sLabel.Width(labelW).Render(fl.label)
		if focused {
			label = sKey.Width(labelW).Render("› " + fl.label)
		}
		var val string
		switch fl.kind {
		case fieldText:
			val = fl.input.View()
		case fieldChoice:
			var parts []string
			for j, o := range fl.options {
				if j == fl.choice {
					parts = append(parts, sTabActive.Render(o))
				} else {
					parts = append(parts, sTab.Render(o))
				}
			}
			val = strings.Join(parts, "")
			if focused {
				val = sDim.Render("◂ ") + val + sDim.Render(" ▸")
			}
		case fieldBool:
			if fl.on {
				val = sGreen.Render("[x] sim")
			} else {
				val = sMuted.Render("[ ] não")
			}
		}
		line := label + val
		z.add("form:field:"+itoa(i), 0, len(b), 60, 1)
		b = append(b, line)
		if focused && fl.help != "" {
			b = append(b, spaces(labelW)+sDim.Render(fl.help))
		}
	}
	b = append(b, "")
	if f.err != "" {
		b = append(b, sRed.Render("✘ "+f.err), "")
	}
	save := sTabActive.Render("Salvar ctrl+s")
	cancel := sTab.Render("Cancelar esc")
	z.add("form:save", 0, len(b), lipgloss.Width(save), 1)
	z.add("form:cancel", lipgloss.Width(save)+2, len(b), lipgloss.Width(cancel), 1)
	b = append(b, save+"  "+cancel)
	b = append(b, sDim.Render("tab/↑↓ navega · ←/→/espaço altera opções"))
	return strings.Join(b, "\n")
}
