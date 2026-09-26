// Package config loads and persists the pr-tracker configuration file.
package config

import (
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"runtime"
	"slices"
	"strings"
	"time"

	"github.com/BurntSushi/toml"

	"github.com/vitorpacheco/pr-tracker/internal/i18n"
)

const appName = "pr-tracker"

// Provider identifies a code hosting platform.
type Provider string

const (
	GitHub    Provider = "github"
	GitLab    Provider = "gitlab"
	Gitea     Provider = "gitea"
	Bitbucket Provider = "bitbucket"
)

// Providers lists every provider pr-tracker knows about, in display order.
var Providers = []Provider{GitHub, GitLab, Gitea, Bitbucket}

// DefaultHost returns the SaaS host of a provider.
func (p Provider) DefaultHost() string {
	switch p {
	case GitHub:
		return "github.com"
	case GitLab:
		return "gitlab.com"
	case Gitea:
		return "gitea.com"
	case Bitbucket:
		return "bitbucket.org"
	}
	return ""
}

// Instance is one account on one host (SaaS or self-hosted).
type Instance struct {
	Name     string   `toml:"name"`
	Provider Provider `toml:"provider"`
	Host     string   `toml:"host"`
	// Disabled instances are kept in the file but never queried.
	Disabled bool `toml:"disabled,omitempty"`
	// MergeMethod is merge, squash or rebase.
	MergeMethod string `toml:"merge_method,omitempty"`
	// AutoMerge asks the provider to merge once the pipeline succeeds.
	AutoMerge bool `toml:"auto_merge,omitempty"`
	// DeleteBranch removes the source branch after merging.
	DeleteBranch bool `toml:"delete_branch,omitempty"`
}

// Repo holds per-repository settings, including its optional local clone.
type Repo struct {
	Instance string `toml:"instance"`
	// Name is owner/repo (GitHub, Gitea) or group/subgroup/project (GitLab).
	Name   string `toml:"name"`
	Path   string `toml:"path"`
	Remote string `toml:"remote,omitempty"`
	// TrackAll includes every open pull/merge request from this repository,
	// regardless of its relation to the current user.
	TrackAll bool `toml:"track_all,omitempty"`
	// MergeMethod overrides the instance merge method for this repo.
	MergeMethod string `toml:"merge_method,omitempty"`
}

// Config is the on-disk configuration.
type Config struct {
	// Language is system (default), en or pt; shared by both interfaces.
	Language  string `toml:"language"`
	ThemeFile string `toml:"theme_file,omitempty"`
	// ToolPaths optionally pins external executables for desktop launchers.
	ToolPaths       map[string]string `toml:"tool_paths,omitempty"`
	DesktopTerminal string            `toml:"desktop_terminal,omitempty"`
	// RefreshInterval is a Go duration ("5m", "90s").
	RefreshInterval string `toml:"refresh_interval"`
	// WorktreeDir holds the PR worktrees. Defaults to ~/.pr-tracker/worktrees.
	WorktreeDir string `toml:"worktree_dir,omitempty"`
	// Terminal selects where interactive tools open: auto, herdr, tmux or inline.
	Terminal string `toml:"terminal"`
	// DiffTool is "hunk" or "none".
	DiffTool string `toml:"diff_tool"`
	// CloneRoots are scanned to auto-discover local clones.
	CloneRoots []string `toml:"clone_roots,omitempty"`

	Instances []Instance `toml:"instances"`
	Repos     []Repo     `toml:"repos"`

	path string
}

// Dir returns the configuration directory: $XDG_CONFIG_HOME/pr-tracker on
// Linux (falling back to ~/.config), and ~/.config/pr-tracker on macOS and
// Windows (%USERPROFILE%\.config\pr-tracker).
func Dir() (string, error) {
	if runtime.GOOS != "darwin" && runtime.GOOS != "windows" {
		if x := os.Getenv("XDG_CONFIG_HOME"); x != "" && filepath.IsAbs(x) {
			return filepath.Join(x, appName), nil
		}
	}
	home, err := os.UserHomeDir()
	if err != nil {
		return "", err
	}
	return filepath.Join(home, ".config", appName), nil
}

// DataDir returns ~/.pr-tracker, where worktrees are stored.
func DataDir() (string, error) {
	home, err := os.UserHomeDir()
	if err != nil {
		return "", err
	}
	return filepath.Join(home, "."+appName), nil
}

// Path returns the configuration file path. PR_TRACKER_CONFIG overrides it.
func Path() (string, error) {
	if p := os.Getenv("PR_TRACKER_CONFIG"); p != "" {
		return p, nil
	}
	dir, err := Dir()
	if err != nil {
		return "", err
	}
	return filepath.Join(dir, "config.toml"), nil
}

// Default returns a configuration with defaults and no instances.
func Default() *Config {
	return &Config{Language: "system", RefreshInterval: "5m", Terminal: "auto", DiffTool: "hunk"}
}

// Load reads the configuration file. A missing file yields defaults and
// created=true so the caller can seed and save it.
func Load() (cfg *Config, created bool, err error) {
	path, err := Path()
	if err != nil {
		return nil, false, err
	}
	cfg = Default()
	cfg.path = path
	if _, err := toml.DecodeFile(path, cfg); err != nil {
		if errors.Is(err, os.ErrNotExist) {
			return cfg, true, nil
		}
		return nil, false, fmt.Errorf(cfg.t("lendo %s: %w"), path, err)
	}
	cfg.path = path
	return cfg, false, cfg.Validate()
}

// Save writes the configuration file, creating its directory.
func (c *Config) Save() error {
	if c.path == "" {
		p, err := Path()
		if err != nil {
			return err
		}
		c.path = p
	}
	if err := os.MkdirAll(filepath.Dir(c.path), 0o755); err != nil {
		return err
	}
	tmp := c.path + ".tmp"
	f, err := os.Create(tmp)
	if err != nil {
		return err
	}
	if err := toml.NewEncoder(f).Encode(c); err != nil {
		f.Close()
		return err
	}
	if err := f.Close(); err != nil {
		return err
	}
	return os.Rename(tmp, c.path)
}

// FilePath is where the configuration was loaded from.
func (c *Config) FilePath() string { return c.path }

// Validate checks field values.
func (c *Config) Validate() error {
	if !i18n.Supported(c.Language) {
		return fmt.Errorf(c.t("idioma inválido %q (system, en, pt)"), c.Language)
	}
	if _, err := time.ParseDuration(c.RefreshInterval); err != nil {
		return fmt.Errorf(c.t("refresh_interval inválido %q: %w"), c.RefreshInterval, err)
	}
	seen := map[string]bool{}
	for _, in := range c.Instances {
		if in.Name == "" {
			return errors.New(c.t("instância sem nome"))
		}
		if seen[in.Name] {
			return fmt.Errorf(c.t("instância duplicada %q"), in.Name)
		}
		seen[in.Name] = true
		if !slices.Contains(Providers, in.Provider) {
			return fmt.Errorf(c.t("instância %q: provider desconhecido %q"), in.Name, in.Provider)
		}
		switch in.MergeMethod {
		case "", "merge", "squash", "rebase":
		default:
			return fmt.Errorf(c.t("instância %q: merge_method inválido %q"), in.Name, in.MergeMethod)
		}
	}
	switch c.Terminal {
	case "auto", "herdr", "tmux", "inline":
	default:
		return fmt.Errorf(c.t("terminal inválido %q (auto, herdr, tmux, inline)"), c.Terminal)
	}
	return nil
}

// Interval returns the refresh interval, with a floor of 15 seconds.
func (c *Config) Interval() time.Duration {
	d, err := time.ParseDuration(c.RefreshInterval)
	if err != nil || d <= 0 {
		return 5 * time.Minute
	}
	return max(d, 15*time.Second)
}

// Worktrees returns the resolved worktree directory.
func (c *Config) Worktrees() (string, error) {
	if c.WorktreeDir != "" {
		return ExpandHome(c.WorktreeDir), nil
	}
	d, err := DataDir()
	if err != nil {
		return "", err
	}
	return filepath.Join(d, "worktrees"), nil
}

// Instance returns the instance with the given name.
func (c *Config) Instance(name string) (*Instance, bool) {
	for i := range c.Instances {
		if c.Instances[i].Name == name {
			return &c.Instances[i], true
		}
	}
	return nil, false
}

// UpsertInstance adds or replaces an instance keyed by oldName (empty to add).
func (c *Config) UpsertInstance(oldName string, in Instance) error {
	if in.Host == "" {
		in.Host = in.Provider.DefaultHost()
	}
	in.Host = NormalizeHost(in.Host)
	if in.Name == "" {
		in.Name = in.Host
	}
	for i := range c.Instances {
		if c.Instances[i].Name == in.Name && in.Name != oldName {
			return fmt.Errorf(c.t("já existe uma instância chamada %q"), in.Name)
		}
	}
	for i := range c.Instances {
		if oldName != "" && c.Instances[i].Name == oldName {
			c.Instances[i] = in
			if oldName != in.Name {
				for j := range c.Repos {
					if c.Repos[j].Instance == oldName {
						c.Repos[j].Instance = in.Name
					}
				}
			}
			return c.Validate()
		}
	}
	c.Instances = append(c.Instances, in)
	return c.Validate()
}

// RemoveInstance deletes an instance and its repo mappings.
func (c *Config) RemoveInstance(name string) bool {
	n := len(c.Instances)
	c.Instances = slices.DeleteFunc(c.Instances, func(in Instance) bool { return in.Name == name })
	c.Repos = slices.DeleteFunc(c.Repos, func(r Repo) bool { return r.Instance == name })
	return len(c.Instances) != n
}

// Repo returns the local mapping of a repository.
func (c *Config) Repo(instance, name string) (*Repo, bool) {
	for i := range c.Repos {
		if c.Repos[i].Instance == instance && strings.EqualFold(c.Repos[i].Name, name) {
			return &c.Repos[i], true
		}
	}
	return nil, false
}

// SetRepoTrackAll enables or disables tracking every open pull/merge request
// from a repository. Enabling it creates a repository entry even when there is
// no local clone mapping; disabling it removes an otherwise empty entry.
func (c *Config) SetRepoTrackAll(instance, name string, enabled bool) {
	r, ok := c.Repo(instance, name)
	if !ok {
		if !enabled {
			return
		}
		c.Repos = append(c.Repos, Repo{Instance: instance, Name: name, TrackAll: true})
		return
	}
	r.TrackAll = enabled
	if !enabled && r.Path == "" && r.Remote == "" && r.MergeMethod == "" {
		c.Repos = slices.DeleteFunc(c.Repos, func(candidate Repo) bool {
			return candidate.Instance == instance && strings.EqualFold(candidate.Name, name)
		})
	}
}

// SetRepoPath stores (or clears, with an empty path) the local clone path.
func (c *Config) SetRepoPath(instance, name, path string) {
	if path == "" {
		if r, ok := c.Repo(instance, name); ok {
			r.Path = ""
			if !r.TrackAll {
				c.Repos = slices.DeleteFunc(c.Repos, func(candidate Repo) bool {
					return candidate.Instance == instance && strings.EqualFold(candidate.Name, name)
				})
			}
		}
		return
	}
	if r, ok := c.Repo(instance, name); ok {
		r.Path = path
		return
	}
	c.Repos = append(c.Repos, Repo{Instance: instance, Name: name, Path: path})
}

// ExpandHome replaces a leading ~ with the user's home directory.
func ExpandHome(p string) string {
	if p == "~" || strings.HasPrefix(p, "~/") || strings.HasPrefix(p, `~\`) {
		if home, err := os.UserHomeDir(); err == nil {
			return filepath.Join(home, p[1:])
		}
	}
	return p
}

// NormalizeHost strips scheme and trailing slashes from a host.
func NormalizeHost(h string) string {
	h = strings.TrimSpace(h)
	h = strings.TrimPrefix(h, "https://")
	h = strings.TrimPrefix(h, "http://")
	return strings.TrimRight(h, "/")
}

func (c *Config) t(message string) string { return i18n.Text(i18n.Resolve(c.Language), message) }

// ThemePath resolves relative palette files beside the configuration file.
func (c *Config) ThemePath() string {
	if c.ThemeFile == "" {
		return ""
	}
	path := ExpandHome(c.ThemeFile)
	if filepath.IsAbs(path) {
		return path
	}
	base := c.path
	if base == "" {
		base, _ = Path()
	}
	return filepath.Join(filepath.Dir(base), path)
}
