package provider

import (
	"context"
	"fmt"

	"github.com/vitorpacheco/pr-tracker/internal/config"
)

// bitbucket is a placeholder. Atlassian ships no official Bitbucket CLI
// (acli covers Jira/admin only); see docs/bitbucket.md for the options found.
type bitbucket struct{ in config.Instance }

func (b *bitbucket) Instance() config.Instance { return b.in }
func (b *bitbucket) Tool() string              { return "bkt" }

func (b *bitbucket) err() error {
	return fmt.Errorf("%w: Bitbucket (%s) — veja docs/bitbucket.md", ErrNotSupported, b.in.Host)
}

func (b *bitbucket) List(context.Context) ([]PR, error)             { return nil, b.err() }
func (b *bitbucket) Approve(context.Context, *PR) error             { return b.err() }
func (b *bitbucket) Merge(context.Context, *PR, MergeOptions) error { return b.err() }
func (b *bitbucket) Checkout(context.Context, *PR, string) error    { return b.err() }
func (b *bitbucket) HeadRef(pr *PR) string                          { return "" }
func (b *bitbucket) AuthStatus(context.Context) error               { return b.err() }
