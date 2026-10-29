// Package settingstest serves a scripted botconf.v1.Settings server in memory,
// for testing code built on the settings client (e.g. panel renderers)
// without a real settings store.
package settingstest

import (
	"context"
	"net"
	"sync"
	"testing"

	"github.com/propheci/botconf/go/settings"
	"github.com/propheci/botconf/go/settingspb"
	"google.golang.org/grpc"
	"google.golang.org/grpc/test/bufconn"
)

// Server answers Render and Act from its functions and records every request.
// Unset functions answer with an empty response.
type Server struct {
	settingspb.UnimplementedSettingsServer

	// RenderFunc answers Render.
	RenderFunc func(*settingspb.RenderRequest) (*settingspb.Screen, error)
	// ActFunc answers Act (presses, answers and commands).
	ActFunc func(*settingspb.ActRequest) (*settingspb.ActResponse, error)

	mu      sync.Mutex
	renders []*settingspb.RenderRequest
	acts    []*settingspb.ActRequest
}

// Render records the request and answers with RenderFunc.
func (s *Server) Render(_ context.Context, request *settingspb.RenderRequest) (*settingspb.RenderResponse, error) {
	s.mu.Lock()
	s.renders = append(s.renders, request)
	s.mu.Unlock()
	if s.RenderFunc == nil {
		return &settingspb.RenderResponse{Screen: &settingspb.Screen{}}, nil
	}
	screen, err := s.RenderFunc(request)
	if err != nil {
		return nil, err
	}
	return &settingspb.RenderResponse{Screen: screen}, nil
}

// Act records the request and answers with ActFunc.
func (s *Server) Act(_ context.Context, request *settingspb.ActRequest) (*settingspb.ActResponse, error) {
	s.mu.Lock()
	s.acts = append(s.acts, request)
	s.mu.Unlock()
	if s.ActFunc == nil {
		return &settingspb.ActResponse{}, nil
	}
	return s.ActFunc(request)
}

// Renders returns the Render requests received so far.
func (s *Server) Renders() []*settingspb.RenderRequest {
	s.mu.Lock()
	defer s.mu.Unlock()
	return append([]*settingspb.RenderRequest(nil), s.renders...)
}

// Acts returns the Act requests received so far.
func (s *Server) Acts() []*settingspb.ActRequest {
	s.mu.Lock()
	defer s.mu.Unlock()
	return append([]*settingspb.ActRequest(nil), s.acts...)
}

// Start serves s over an in-memory connection for the length of the test and
// returns a client dialled to it.
func Start(t testing.TB, s *Server, opts ...settings.Option) *settings.Client {
	t.Helper()
	listener := bufconn.Listen(1 << 20)
	server := grpc.NewServer()
	settingspb.RegisterSettingsServer(server, s)
	go func() { _ = server.Serve(listener) }()
	t.Cleanup(server.Stop)

	dialer := grpc.WithContextDialer(func(ctx context.Context, _ string) (net.Conn, error) {
		return listener.DialContext(ctx)
	})
	opts = append([]settings.Option{settings.WithDialOptions(dialer)}, opts...)
	client, err := settings.Dial("passthrough:///bufnet", opts...)
	if err != nil {
		t.Fatalf("settings.Dial() error = %v", err)
	}
	t.Cleanup(func() { _ = client.Close() })
	return client
}
