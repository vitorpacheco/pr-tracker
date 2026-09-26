//go:build gui || bindings

package main

import (
	"context"
	"embed"
	"fmt"
	"os"

	"github.com/wailsapp/wails/v2"
	"github.com/wailsapp/wails/v2/pkg/options"
	"github.com/wailsapp/wails/v2/pkg/options/assetserver"
	"github.com/wailsapp/wails/v2/pkg/options/linux"
	"github.com/wailsapp/wails/v2/pkg/runtime"

	"github.com/vitorpacheco/pr-tracker/internal/app"
	"github.com/vitorpacheco/pr-tracker/internal/cache"
	"github.com/vitorpacheco/pr-tracker/internal/config"
	"github.com/vitorpacheco/pr-tracker/internal/i18n"
	"github.com/vitorpacheco/pr-tracker/internal/launch"
	"github.com/vitorpacheco/pr-tracker/internal/toolchain"
)

//go:embed all:frontend/dist
var assets embed.FS

func main() {
	ready := make(chan struct{})
	bridge := &Bridge{ready: ready}
	var session *app.Session
	var store *cache.Store
	err := wails.Run(&options.App{
		Title: "pr-tracker", Width: 1280, Height: 820, MinWidth: 400, MinHeight: 480,
		AssetServer: &assetserver.Options{Assets: assets},
		// Avoid WebKitGTK's accelerated surface path, which can disconnect from
		// Wayland with "Missing acquire timeline" on Hyprland/NVIDIA.
		Linux: &linux.Options{ProgramName: "io.github.vitorpacheco.pr_tracker", WebviewGpuPolicy: linux.WebviewGpuPolicyNever},
		OnStartup: func(ctx context.Context) {
			defer close(ready)
			cfg, _, err := config.Load()
			if err != nil {
				bridge.startupError = err.Error()
				cfg = config.Default()
				bridge.settingsError = err
			}
			store, err = cache.OpenDefault()
			if err != nil {
				bridge.startupError = err.Error()
			}
			session = app.NewSession(cfg, store)
			bridge.session = session
			bridge.folder = launch.Folder
			bridge.pickFolder = func() (string, error) {
				return runtime.OpenDirectoryDialog(ctx, runtime.OpenDialogOptions{Title: i18n.Text(i18n.Resolve(session.Configuration().Language), "Selecionar clone local")})
			}
			bridge.pickTheme = func() (string, error) {
				return runtime.OpenFileDialog(ctx, runtime.OpenDialogOptions{Title: bridge.t("Arquivo de cores"), Filters: []runtime.FileFilter{{DisplayName: "TOML", Pattern: "*.toml"}}})
			}
			bridge.saveTheme = func() (string, error) {
				return runtime.SaveFileDialog(ctx, runtime.SaveDialogOptions{Title: bridge.t("Exportar esquema de cores"), DefaultFilename: "colors.toml", CanCreateDirectories: true, Filters: []runtime.FileFilter{{DisplayName: "TOML", Pattern: "*.toml"}}})
			}
			bridge.terminal = func(dir string, argv []string) error {
				settings := session.Configuration()
				work := toolchain.WithPaths(ctx, settings.ToolPaths)
				if len(argv) > 0 {
					path, err := toolchain.Lookup(work, argv[0])
					if err != nil {
						return err
					}
					argv[0] = path
				}
				return launch.Desktop(work, settings.DesktopTerminal, dir, argv)
			}
			bridge.openURL = func(url string) { runtime.BrowserOpenURL(ctx, url) }
		},
		OnShutdown: func(context.Context) {
			if session != nil {
				session.Close()
			}
			if store != nil {
				store.Close()
			}
		},
		Bind: []interface{}{bridge},
	})
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}
