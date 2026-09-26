package i18n

import (
	"unsafe"

	"golang.org/x/sys/windows"
)

func systemLanguage() string {
	kernel := windows.NewLazySystemDLL("kernel32.dll")
	// The display language can differ from the user's regional format.
	language, _, _ := kernel.NewProc("GetUserDefaultUILanguage").Call()
	var name [85]uint16 // LOCALE_NAME_MAX_LENGTH, including the terminator.
	n, _, _ := kernel.NewProc("LCIDToLocaleName").Call(language, uintptr(unsafe.Pointer(&name[0])), uintptr(len(name)), 0)
	if n == 0 {
		return "en"
	}
	return windows.UTF16ToString(name[:])
}
