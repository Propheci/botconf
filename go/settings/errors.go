package settings

import (
	"errors"

	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"
)

// Errors a call can fail with, to test with errors.Is. Other failures keep
// their gRPC status (status.FromError works on every error the client
// returns).
var (
	// ErrNotFound: the key is not a runtime setting.
	ErrNotFound = errors.New("no such setting")
	// ErrPermissionDenied: the server refused the credentials or the actor.
	ErrPermissionDenied = errors.New("not allowed")
	// ErrUnavailable: the server cannot be reached.
	ErrUnavailable = errors.New("settings server unavailable")
)

// InvalidValueError is a change the server refused: the value does not fit
// the setting, or the key is not a list or a map entry. Test with errors.As.
type InvalidValueError struct {
	// Reason is the server's explanation, to show to the user.
	Reason string
	status *status.Status
}

func (e *InvalidValueError) Error() string { return e.Reason }

// GRPCStatus returns the status the server answered with.
func (e *InvalidValueError) GRPCStatus() *status.Status { return e.status }

// Error is a failed call whose status maps to one of the Err values.
type Error struct {
	kind   error
	status *status.Status
}

func (e *Error) Error() string { return e.kind.Error() + ": " + e.status.Message() }

// Is matches the Err value the status maps to.
func (e *Error) Is(target error) bool { return target == e.kind }

// GRPCStatus returns the status the server answered with.
func (e *Error) GRPCStatus() *status.Status { return e.status }

// convert turns a call's error into one of the typed errors above.
func convert(err error) error {
	if err == nil {
		return nil
	}
	st, ok := status.FromError(err)
	if !ok {
		return err
	}
	switch st.Code() {
	case codes.NotFound:
		return &Error{kind: ErrNotFound, status: st}
	case codes.InvalidArgument:
		return &InvalidValueError{Reason: st.Message(), status: st}
	case codes.PermissionDenied, codes.Unauthenticated:
		return &Error{kind: ErrPermissionDenied, status: st}
	case codes.Unavailable:
		return &Error{kind: ErrUnavailable, status: st}
	default:
		return err
	}
}
