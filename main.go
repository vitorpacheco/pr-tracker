// Command pr-tracker is a terminal UI to follow pull/merge requests across
// GitHub and GitLab instances (SaaS and self-hosted) using gh and glab.
package main

import (
	"context"
	"flag"
	"fmt"
	"os"
	"strings"
	"text/tabwriter"
	"time"

	tea "charm.land/bubbletea/v2"

	"github.com/vitorpacheco/pr-tracker/internal/config"
	"github.com/vitorpacheco/pr-tracker/internal/gitops"
	"github.com/vitorpacheco/pr-tracker/internal/provider"
	"github.com/vitorpacheco/pr-tracker/internal/ui"
)

var version = "dev"

const usage = `pr-tracker — acompanhe pull requests (GitHub) e merge requests (GitLab) no terminal

Uso:
  pr-tracker                          abre a interface
  pr-tracker list                     lista os PRs no stdout
  pr-tracker doctor                   verifica CLIs, autenticação e configuração
  pr-tracker config path              mostra o caminho do arquivo de configuração
  pr-tracker instance list
  pr-tracker instance add --provider github|gitlab [--host H] [--name N] [--merge-method M]
  pr-tracker instance rm NOME
  pr-tracker repo set --instance NOME --repo owner/repo --path ~/code/repo [--remote origin]
  pr-tracker repo list
  pr-tracker version
`

func main() {
	if err := run(os.Args[1:]); err != nil {
		fmt.Fprintln(os.Stderr, "erro:", err)
		os.Exit(1)
	}
}

func run(args []string) error {
	cfg, created, err := config.Load()
	if err != nil {
		return err
	}
	if created {
		seed(cfg)
		if err := cfg.Save(); err != nil {
			return err
		}
	}
	if len(args) == 0 {
		return runUI(cfg)
	}
	switch args[0] {
	case "-h", "--help", "help":
		fmt.Print(usage)
	case "version", "--version":
		fmt.Println(version)
	case "config":
		fmt.Println(cfg.FilePath())
	case "doctor":
		doctor(cfg)
	case "list", "ls":
		return list(cfg)
	case "instance", "instances":
		return instanceCmd(cfg, args[1:])
	case "repo", "repos":
		return repoCmd(cfg, args[1:])
	default:
		fmt.Print(usage)
		return fmt.Errorf("comando desconhecido %q", args[0])
	}
	return nil
}

func runUI(cfg *config.Config) error {
	_, err := tea.NewProgram(ui.New(cfg)).Run()
	return err
}

// seed creates instances for the SaaS hosts the installed CLIs are logged in to.
func seed(cfg *config.Config) {
	ctx, cancel := context.WithTimeout(context.Background(), 15*time.Second)
	defer cancel()
	for _, p := range []config.Provider{config.GitHub, config.GitLab} {
		in := config.Instance{Name: p.DefaultHost(), Provider: p, Host: p.DefaultHost()}
		c := provider.New(in)
		if !provider.ToolAvailable(c.Tool()) {
			continue
		}
		if c.AuthStatus(ctx) == nil {
			_ = cfg.UpsertInstance("", in)
		}
	}
}

func doctor(cfg *config.Config) {
	ok := func(b bool) string {
		if b {
			return "✔"
		}
		return "✘"
	}
	fmt.Println("configuração:", cfg.FilePath())
	wt, _ := cfg.Worktrees()
	fmt.Println("worktrees:   ", wt)
	fmt.Println()
	for _, t := range []string{"git", "gh", "glab", "hunk", "herdr", "tmux"} {
		have := provider.ToolAvailable(t)
		line := fmt.Sprintf("%s %-6s", ok(have), t)
		if !have && t != "git" {
			line += "  " + provider.InstallHint(t)
		}
		fmt.Println(line)
	}
	fmt.Println()
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	for _, in := range cfg.Instances {
		c := provider.New(in)
		err := c.AuthStatus(ctx)
		msg := "ok"
		if err != nil {
			msg = err.Error()
		}
		if in.Disabled {
			msg += " (desativada)"
		}
		fmt.Printf("%s %s [%s %s] %s\n", ok(err == nil), in.Name, in.Provider, in.Host, msg)
	}
	if len(cfg.Instances) == 0 {
		fmt.Println("nenhuma instância configurada: pr-tracker instance add --provider github")
	}
}

func list(cfg *config.Config) error {
	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Minute)
	defer cancel()
	w := tabwriter.NewWriter(os.Stdout, 0, 2, 2, ' ', 0)
	fmt.Fprintln(w, "INSTÂNCIA\tREPO\tPR\tCI\tREVISÃO\tRELAÇÃO\tTÍTULO")
	var errs []string
	for _, in := range cfg.Instances {
		if in.Disabled {
			continue
		}
		prs, err := provider.New(in).List(ctx)
		if err != nil {
			errs = append(errs, in.Name+": "+err.Error())
			continue
		}
		for _, pr := range prs {
			var rel []string
			if pr.Relations&provider.ReviewRequested != 0 {
				rel = append(rel, "revisor")
			}
			if pr.Relations&provider.Authored != 0 {
				rel = append(rel, "autor")
			}
			if pr.Relations&provider.Assigned != 0 {
				rel = append(rel, "atribuído")
			}
			ci := string(pr.CI)
			if ci == "" {
				ci = "-"
			}
			fmt.Fprintf(w, "%s\t%s\t%s\t%s\t%s\t%s\t%s\n", pr.Instance, pr.Repo, pr.Ref(), ci, orDash(pr.Review), strings.Join(rel, ","), pr.Title)
		}
	}
	w.Flush()
	for _, e := range errs {
		fmt.Fprintln(os.Stderr, "✘", e)
	}
	return nil
}

func orDash(s string) string {
	if s == "" {
		return "-"
	}
	return s
}

func instanceCmd(cfg *config.Config, args []string) error {
	if len(args) == 0 || args[0] == "list" || args[0] == "ls" {
		w := tabwriter.NewWriter(os.Stdout, 0, 2, 2, ' ', 0)
		fmt.Fprintln(w, "NOME\tPROVIDER\tHOST\tMERGE\tSTATUS")
		for _, in := range cfg.Instances {
			st := "ativa"
			if in.Disabled {
				st = "desativada"
			}
			fmt.Fprintf(w, "%s\t%s\t%s\t%s\t%s\n", in.Name, in.Provider, in.Host, orDash(in.MergeMethod), st)
		}
		return w.Flush()
	}
	switch args[0] {
	case "add":
		fs := flag.NewFlagSet("instance add", flag.ContinueOnError)
		prov := fs.String("provider", "", "github, gitlab ou bitbucket")
		host := fs.String("host", "", "host (padrão: SaaS do provider)")
		name := fs.String("name", "", "nome único (padrão: host)")
		merge := fs.String("merge-method", "", "merge, squash ou rebase")
		auto := fs.Bool("auto-merge", false, "merge automático quando o pipeline passar")
		del := fs.Bool("delete-branch", false, "apagar a branch de origem após o merge")
		if err := fs.Parse(args[1:]); err != nil {
			return err
		}
		in := config.Instance{Name: *name, Provider: config.Provider(*prov), Host: *host, MergeMethod: *merge, AutoMerge: *auto, DeleteBranch: *del}
		if err := cfg.UpsertInstance("", in); err != nil {
			return err
		}
		if err := cfg.Save(); err != nil {
			return err
		}
		added := cfg.Instances[len(cfg.Instances)-1]
		fmt.Printf("instância %q adicionada (%s %s)\n", added.Name, added.Provider, added.Host)
		c := provider.New(added)
		switch {
		case added.Provider == config.Bitbucket:
			fmt.Println("aviso: a integração com Bitbucket ainda não existe — veja docs/bitbucket.md")
		case !provider.ToolAvailable(c.Tool()):
			fmt.Printf("aviso: %s não está instalado. Instale: %s\n", c.Tool(), provider.InstallHint(c.Tool()))
		default:
			ctx, cancel := context.WithTimeout(context.Background(), 20*time.Second)
			defer cancel()
			if err := c.AuthStatus(ctx); err != nil {
				fmt.Println("aviso:", err)
			}
		}
	case "rm", "remove", "delete":
		if len(args) < 2 {
			return fmt.Errorf("uso: pr-tracker instance rm NOME")
		}
		if !cfg.RemoveInstance(args[1]) {
			return fmt.Errorf("instância %q não encontrada", args[1])
		}
		return cfg.Save()
	default:
		return fmt.Errorf("subcomando desconhecido %q", args[0])
	}
	return nil
}

func repoCmd(cfg *config.Config, args []string) error {
	if len(args) == 0 || args[0] == "list" || args[0] == "ls" {
		w := tabwriter.NewWriter(os.Stdout, 0, 2, 2, ' ', 0)
		fmt.Fprintln(w, "INSTÂNCIA\tREPO\tPASTA\tREMOTE")
		for _, r := range cfg.Repos {
			fmt.Fprintf(w, "%s\t%s\t%s\t%s\n", r.Instance, r.Name, r.Path, orDash(r.Remote))
		}
		return w.Flush()
	}
	if args[0] != "set" {
		return fmt.Errorf("subcomando desconhecido %q", args[0])
	}
	fs := flag.NewFlagSet("repo set", flag.ContinueOnError)
	inst := fs.String("instance", "", "nome da instância")
	repo := fs.String("repo", "", "owner/repo ou group/subgroup/project")
	path := fs.String("path", "", "pasta do clone local (vazio remove)")
	remote := fs.String("remote", "", "remote (padrão: detectado)")
	merge := fs.String("merge-method", "", "sobrescreve o método de merge da instância")
	if err := fs.Parse(args[1:]); err != nil {
		return err
	}
	if _, ok := cfg.Instance(*inst); !ok {
		return fmt.Errorf("instância %q não encontrada", *inst)
	}
	if *repo == "" {
		return fmt.Errorf("--repo é obrigatório")
	}
	if *path != "" {
		if err := gitops.ValidateClone(context.Background(), *path); err != nil {
			return err
		}
	}
	cfg.SetRepoPath(*inst, *repo, *path)
	if r, ok := cfg.Repo(*inst, *repo); ok {
		r.Remote, r.MergeMethod = *remote, *merge
	}
	return cfg.Save()
}
