package app

import (
	"context"

	"github.com/vitorpacheco/pr-tracker/internal/i18n"
	"github.com/vitorpacheco/pr-tracker/internal/provider"
	"github.com/vitorpacheco/pr-tracker/internal/toolchain"
)

type ToolStatus struct {
	Name string
	Path string
	Hint string
}
type InstanceStatus struct {
	Name     string
	Disabled bool
	Error    string
}
type Diagnostics struct {
	Tools     []ToolStatus
	Instances []InstanceStatus
}

func (s *Session) Diagnose(ctx context.Context) (Diagnostics, error) {
	s.mu.Lock()
	if s.closed {
		s.mu.Unlock()
		return Diagnostics{}, ErrClosed
	}
	a := s.application
	s.workers.Add(1)
	s.mu.Unlock()
	defer s.workers.Done()
	work, cancel := context.WithCancel(s.ctx)
	defer cancel()
	stop := context.AfterFunc(ctx, cancel)
	defer stop()
	work = toolchain.WithPaths(work, a.cfg.ToolPaths)
	result := Diagnostics{Tools: []ToolStatus{}, Instances: []InstanceStatus{}}
	for _, name := range []string{"git", "gh", "glab", "tea", "hunk"} {
		path, _ := toolchain.Lookup(work, name)
		result.Tools = append(result.Tools, ToolStatus{Name: name, Path: path, Hint: provider.InstallHint(name)})
	}
	for _, in := range a.cfg.Instances {
		status := InstanceStatus{Name: in.Name, Disabled: in.Disabled}
		if !in.Disabled {
			if err := a.client(in, a.cfg.Repos...).AuthStatus(work); err != nil {
				status.Error = i18n.ErrorText(i18n.Resolve(a.cfg.Language), err)
			}
		}
		result.Instances = append(result.Instances, status)
	}
	return result, work.Err()
}
