package provider

import (
	"errors"
	"slices"
)

func sortComments(c []Comment) {
	slices.SortStableFunc(c, func(a, b Comment) int { return a.CreatedAt.Compare(b.CreatedAt) })
}

// accumulator merges PRs returned by several queries, OR-ing their relations.
type accumulator struct {
	order []string
	byKey map[string]*Item
}

func newAccumulator() *accumulator { return &accumulator{byKey: map[string]*Item{}} }

func (a *accumulator) add(pr Item, rel Relation) {
	k := pr.Key()
	if prev, ok := a.byKey[k]; ok {
		prev.Relations |= rel
		return
	}
	pr.Relations = rel
	a.byKey[k] = &pr
	a.order = append(a.order, k)
}

func (a *accumulator) list() []Item {
	out := make([]Item, 0, len(a.order))
	for _, k := range a.order {
		out = append(out, *a.byKey[k])
	}
	return out
}

func isMissing(err error) bool {
	var m *MissingToolError
	return errors.As(err, &m)
}
