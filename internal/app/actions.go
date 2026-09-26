package app

import (
	"context"
	"errors"
	"fmt"
	"strings"

	"github.com/vitorpacheco/pr-tracker/internal/gitops"
	"github.com/vitorpacheco/pr-tracker/internal/i18n"
	"github.com/vitorpacheco/pr-tracker/internal/provider"
	"github.com/vitorpacheco/pr-tracker/internal/toolchain"
)

type Action string

const (
	Comment         Action = "comment"
	Approve         Action = "approve"
	Merge           Action = "merge"
	Close           Action = "close"
	Checkout        Action = "checkout"
	UpdateWorktree  Action = "update_worktree"
	PrepareTerminal Action = "prepare_terminal"
	PrepareDiff     Action = "prepare_diff"
	RemoveWorktree  Action = "remove_worktree"
)

// Command describes an already-confirmed operation. Force is accepted only for
// a standalone removal, after a separate confirmation of local data loss.
type Command struct {
	Action      Action
	Item        provider.Item
	Body        string
	RemoveAfter bool
	Force       bool
}

// Outcome can accompany an error: a remote action may succeed even when its
// subsequent local cleanup fails. Adapters must honor Refresh in either case.
type Outcome struct {
	Message      string
	Refresh      bool
	ReloadThread bool
	Directory    string
	Args         []string
	Note         string
}

var (
	ErrCloneRequired = i18n.Errorf("pasta local do repositório não configurada")
	ErrDirtyWorktree = gitops.ErrDirty
)

func (a *Application) itemClient(item *provider.Item) (provider.Client, error) {
	in, ok := a.cfg.Instance(item.Instance)
	if !ok || in.Disabled {
		return nil, fmt.Errorf(a.t("instância indisponível: %s"), item.Instance)
	}
	if item.Provider != in.Provider || item.Host != in.Host {
		return nil, errors.New(a.t("a instância mudou; atualize o item antes de continuar"))
	}
	return a.client(*in, a.cfg.Repos...), nil
}

// Detail loads the conversation without presentation-specific Markdown cleanup.
// Until application state owns item lookup, adapters supply the selected item.
func (a *Application) Detail(ctx context.Context, item provider.Item) (*provider.Thread, error) {
	ctx = toolchain.WithPaths(ctx, a.cfg.ToolPaths)
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	client, err := a.itemClient(&item)
	if err != nil {
		return nil, err
	}
	return client.Thread(ctx, &item)
}

func (a *Application) MergeOptions(item provider.Item) provider.MergeOptions {
	opts := provider.MergeOptions{Method: "merge"}
	if in, ok := a.cfg.Instance(item.Instance); ok {
		if in.MergeMethod != "" {
			opts.Method = in.MergeMethod
		}
		opts.Auto, opts.DeleteBranch = in.AutoMerge, in.DeleteBranch
	}
	if repo, ok := a.cfg.Repo(item.Instance, item.Repo); ok && repo.MergeMethod != "" {
		opts.Method = repo.MergeMethod
	}
	return opts
}

func (a *Application) Execute(ctx context.Context, command Command) (Outcome, error) {
	ctx = toolchain.WithPaths(ctx, a.cfg.ToolPaths)
	var out Outcome
	if err := ctx.Err(); err != nil {
		return out, err
	}
	switch command.Action {
	case Comment, Approve, Merge, Close, Checkout, UpdateWorktree, PrepareTerminal, PrepareDiff, RemoveWorktree:
	default:
		return out, fmt.Errorf(a.t("ação desconhecida: %q"), command.Action)
	}
	if command.Force && command.Action != RemoveWorktree {
		return out, errors.New(a.t("remoção forçada exige um comando separado"))
	}
	if command.RemoveAfter && command.Action != Approve && command.Action != Merge && command.Action != Close {
		return out, errors.New(a.t("limpeza combinada exige aprovação, merge ou fechamento"))
	}
	item := command.Item
	if item.IsIssue() && command.Action != Comment {
		return out, errors.New(a.t("ação disponível apenas para pull/merge requests"))
	}
	if item.Kind != provider.KindPR && item.Kind != provider.KindIssue {
		return out, errors.New(a.t("tipo de item inválido"))
	}
	if item.Repo == "" || item.Number <= 0 {
		return out, errors.New(a.t("item inválido"))
	}
	client, err := a.itemClient(&item)
	if err != nil {
		return out, err
	}
	switch command.Action {
	case Comment:
		body := strings.TrimSpace(command.Body)
		if body == "" {
			return out, errors.New(a.t("comentário vazio"))
		}
		if err = client.AddComment(ctx, &item, body); err == nil {
			out.Message = a.t("comentário enviado em ") + item.Repo + item.Ref()
			out.Refresh, out.ReloadThread = true, true
		}
		return out, err
	case Approve:
		err = client.Approve(ctx, &item)
		out.Message = a.t("aprovado ") + item.Ref()
	case Merge:
		err = client.Merge(ctx, &item, a.MergeOptions(item))
		out.Message = a.t("merge de ") + item.Ref() + a.t(" feito")
	case Close:
		err = client.Close(ctx, &item)
		out.Message = item.Ref() + a.t(" fechado sem merge")
	case Checkout:
		clone, _, ok := a.resolveClone(ctx, &a.cfg, &item)
		if !ok {
			return out, ErrCloneRequired
		}
		if err = client.Checkout(ctx, &item, clone); err == nil {
			out.Message = "branch " + item.SourceBranch + a.t(" em ") + clone
		}
		return out, err
	case UpdateWorktree, PrepareTerminal, PrepareDiff:
		dir, err := a.ensureWorktree(ctx, client, &item, command.Action == UpdateWorktree)
		if err != nil {
			return out, err
		}
		out.Directory = dir
		if command.Action == UpdateWorktree {
			out.Message = a.t("worktree pronto: ") + dir
		}
		if command.Action == PrepareDiff {
			out.Args, out.Note = a.diffCommand(ctx, dir, &item)
		}
		return out, nil
	case RemoveWorktree:
		err = a.removeWorktree(ctx, &a.cfg, &item, command.Force)
		if err == nil {
			out.Message = a.t("worktree removido")
			if command.Force {
				out.Message += a.t(" (alterações descartadas)")
			}
		}
		return out, err
	}
	if err != nil {
		return Outcome{}, err
	}
	out.Refresh = true
	if command.RemoveAfter {
		if err := a.removeWorktree(ctx, &a.cfg, &item, false); err != nil {
			return out, err
		}
		out.Message += a.t(" · worktree removido")
	}
	return out, nil
}

func (a *Application) ensureWorktree(ctx context.Context, client provider.Client, item *provider.Item, update bool) (string, error) {
	if dir, ok := a.worktreeExists(&a.cfg, item); ok && !update {
		return dir, nil
	}
	clone, remote, ok := a.resolveClone(ctx, &a.cfg, item)
	if !ok {
		return "", ErrCloneRequired
	}
	return a.createWorktree(ctx, &a.cfg, client, item, clone, remote)
}

func (a *Application) diffCommand(ctx context.Context, wt string, pr *provider.Item) ([]string, string) {
	base := "HEAD~1"
	if up, err := gitops.Git(ctx, wt, "rev-parse", "--abbrev-ref", "@{upstream}"); err == nil || pr.TargetBranch != "" {
		remote := "origin"
		if i := strings.Index(up, "/"); err == nil && i > 0 {
			remote = up[:i]
		}
		if mb, err := gitops.Git(ctx, wt, "merge-base", remote+"/"+pr.TargetBranch, "HEAD"); err == nil {
			base = mb
		}
	}
	if a.cfg.DiffTool == "hunk" {
		if path, err := toolchain.Lookup(ctx, "hunk"); err == nil {
			return []string{path, "diff", base}, ""
		}
		return []string{"git", "diff", base}, a.t("hunk não instalado (") + provider.InstallHint("hunk") + a.t("); usando git diff")
	}
	return []string{"git", "diff", base}, ""
}

func (a *Application) t(message string) string {
	return i18n.Text(i18n.Resolve(a.cfg.Language), message)
}
