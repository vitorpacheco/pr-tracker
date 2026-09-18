package ui

import (
	"strings"

	"charm.land/lipgloss/v2"
)

// zone is a clickable rectangle registered while rendering.
type zone struct {
	id         string
	x, y, w, h int
}

type zones []zone

func (z *zones) add(id string, x, y, w, h int) {
	*z = append(*z, zone{id: id, x: x, y: y, w: w, h: h})
}

// hit returns the topmost zone (last registered) at x,y.
func (z zones) hit(x, y int) (string, bool) {
	for i := len(z) - 1; i >= 0; i-- {
		r := z[i]
		if x >= r.x && x < r.x+r.w && y >= r.y && y < r.y+r.h {
			return r.id, true
		}
	}
	return "", false
}

// hint is a keyboard shortcut shown (and clickable) in the UI.
type hint struct {
	key, label string
}

func renderHint(h hint) string {
	return sKey.Render(h.key) + " " + sMuted.Render(h.label)
}

// hintBar lays hints out left to right, wrapping at width. Each hint becomes
// a "key:<key>" zone relative to (x0, y0).
func hintBar(hints []hint, width, x0, y0 int, z *zones) []string {
	var lines []string
	var cur strings.Builder
	x := 0
	sep := sDim.Render("  ")
	for _, h := range hints {
		s := renderHint(h)
		w := lipgloss.Width(s)
		if x > 0 && x+2+w > width {
			lines = append(lines, cur.String())
			cur.Reset()
			x = 0
		}
		if x > 0 {
			cur.WriteString(sep)
			x += 2
		}
		z.add("key:"+h.key, x0+x, y0+len(lines), w, 1)
		cur.WriteString(s)
		x += w
	}
	if cur.Len() > 0 {
		lines = append(lines, cur.String())
	}
	return lines
}
