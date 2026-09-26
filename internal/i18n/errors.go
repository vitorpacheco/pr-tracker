package i18n

import (
	"errors"
	"fmt"
)

// messageError keeps arguments separate so presentation can translate an error
// without replacing words inside paths, repository names or external output.
type messageError struct {
	format string
	args   []any
	cause  error
}

func (e *messageError) Error() string { return e.cause.Error() }
func (e *messageError) Unwrap() error { return e.cause }

// Errorf defers translation until the error reaches an interface. Unwrap keeps
// errors.Is/As working, including errors wrapped with %w.
func Errorf(format string, args ...any) error {
	return &messageError{format: format, args: args, cause: fmt.Errorf(format, args...)}
}

// ErrorText translates structured application errors, preserving external ones.
func ErrorText(language string, err error) string {
	if err == nil {
		return ""
	}
	if message, ok := err.(*messageError); ok {
		args := append([]any(nil), message.args...)
		for i, arg := range args {
			if nested, ok := arg.(error); ok {
				args[i] = errors.New(ErrorText(language, nested))
			}
		}
		return fmt.Errorf(Text(language, message.format), args...).Error()
	}
	return err.Error()
}
