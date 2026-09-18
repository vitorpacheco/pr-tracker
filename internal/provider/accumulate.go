package provider

import "errors"

// accumulator merges PRs returned by several queries, OR-ing their relations.
type accumulator struct {
	order []string
	byKey map[string]*PR
}

func newAccumulator() *accumulator { return &accumulator{byKey: map[string]*PR{}} }

func (a *accumulator) add(pr PR, rel Relation) {
	k := pr.Key()
	if prev, ok := a.byKey[k]; ok {
		prev.Relations |= rel
		return
	}
	pr.Relations = rel
	a.byKey[k] = &pr
	a.order = append(a.order, k)
}

func (a *accumulator) list() []PR {
	out := make([]PR, 0, len(a.order))
	for _, k := range a.order {
		out = append(out, *a.byKey[k])
	}
	return out
}

func isMissing(err error) bool {
	var m *MissingToolError
	return errors.As(err, &m)
}
