//go:build darwin && (gui || bindings)

package main

// Wails uses UTType in native file dialogs. Its CLI adds this framework, but
// direct go builds (including CI) must also link it.

// #cgo LDFLAGS: -framework UniformTypeIdentifiers
import "C"
