package settings

import (
	"context"
	"encoding/json"
	"errors"
	"net"
	"sync"
	"testing"
	"time"

	"github.com/propheci/botconf/go/settingspb"
	"google.golang.org/grpc"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/metadata"
	"google.golang.org/grpc/status"
	"google.golang.org/grpc/test/bufconn"
)

// fakeServer records what the client sends and answers from a script.
type fakeServer struct {
	settingspb.UnimplementedSettingsServer

	mu        sync.Mutex
	metadata  []metadata.MD
	actors    []*settingspb.Actor
	acts      []*settingspb.ActRequest
	getError  error
	watches   int
	streamEnd chan struct{}
}

func (f *fakeServer) record(ctx context.Context, actor *settingspb.Actor) {
	f.mu.Lock()
	defer f.mu.Unlock()
	md, _ := metadata.FromIncomingContext(ctx)
	f.metadata = append(f.metadata, md)
	f.actors = append(f.actors, actor)
}

func (f *fakeServer) Get(ctx context.Context, request *settingspb.GetRequest) (*settingspb.ValueEntry, error) {
	f.record(ctx, request.GetActor())
	if f.getError != nil {
		return nil, f.getError
	}
	value := `5`
	return &settingspb.ValueEntry{Key: request.GetKey(), ValueJson: &value, Source: settingspb.Source_SOURCE_STORED}, nil
}

func (f *fakeServer) Act(ctx context.Context, request *settingspb.ActRequest) (*settingspb.ActResponse, error) {
	f.record(ctx, request.GetActor())
	f.mu.Lock()
	f.acts = append(f.acts, request)
	f.mu.Unlock()
	return &settingspb.ActResponse{}, nil
}

// Watch sends an initial event and one change, then ends the stream (the
// first time) or waits for the client to go away.
func (f *fakeServer) Watch(request *settingspb.WatchRequest, stream settingspb.Settings_WatchServer) error {
	f.record(stream.Context(), request.GetActor())
	f.mu.Lock()
	f.watches++
	first := f.watches == 1
	f.mu.Unlock()
	value := `"x"`
	if err := stream.Send(&settingspb.WatchEvent{Initial: true, Values: []*settingspb.ValueEntry{{Key: "a.mode", ValueJson: &value}}}); err != nil {
		return err
	}
	if !first {
		<-stream.Context().Done()
		return nil
	}
	changed := `"y"`
	if err := stream.Send(&settingspb.WatchEvent{Values: []*settingspb.ValueEntry{{Key: "a.mode", ValueJson: &changed}}}); err != nil {
		return err
	}
	return status.Error(codes.Unavailable, "restarting")
}

// start serves fake over an in-memory connection and dials it.
func start(t *testing.T, fake *fakeServer, opts ...Option) *Client {
	t.Helper()
	listener := bufconn.Listen(1 << 20)
	server := grpc.NewServer()
	settingspb.RegisterSettingsServer(server, fake)
	go func() { _ = server.Serve(listener) }()
	t.Cleanup(server.Stop)

	dialer := grpc.WithContextDialer(func(ctx context.Context, _ string) (net.Conn, error) {
		return listener.DialContext(ctx)
	})
	opts = append([]Option{WithDialOptions(dialer), withBackoff(time.Millisecond, 10*time.Millisecond)}, opts...)
	client, err := Dial("passthrough:///bufnet", opts...)
	if err != nil {
		t.Fatalf("Dial() error = %v", err)
	}
	t.Cleanup(func() { _ = client.Close() })
	return client
}

func contextFor(t *testing.T) context.Context {
	t.Helper()
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	t.Cleanup(cancel)
	return ctx
}

func TestBearerTokenIsSent(t *testing.T) {
	fake := &fakeServer{}
	client := start(t, fake, WithBearer("t"))
	if _, err := client.Get(contextFor(t), nil, "a"); err != nil {
		t.Fatalf("Get() error = %v", err)
	}
	if got := fake.metadata[0].Get("authorization"); len(got) != 1 || got[0] != "Bearer t" {
		t.Fatalf("authorization = %v, want [Bearer t]", got)
	}
}

func TestDefaultActorIsUsed(t *testing.T) {
	fake := &fakeServer{}
	client := start(t, fake, WithActor("telegram.user", "1"))
	ctx := contextFor(t)
	if _, err := client.Get(ctx, nil, "a"); err != nil {
		t.Fatalf("Get() error = %v", err)
	}
	if _, err := client.Get(ctx, &Actor{Domain: "telegram.user", ID: "2"}, "a"); err != nil {
		t.Fatalf("Get() error = %v", err)
	}
	if got := fake.actors[0].GetId(); got != "1" {
		t.Errorf("default actor = %q, want 1", got)
	}
	if got := fake.actors[1].GetId(); got != "2" {
		t.Errorf("explicit actor = %q, want 2", got)
	}

	// Without a default, a nil actor sends none.
	bare := &fakeServer{}
	if _, err := start(t, bare).Get(ctx, nil, "a"); err != nil {
		t.Fatalf("Get() error = %v", err)
	}
	if bare.actors[0] != nil {
		t.Errorf("actor = %v, want none", bare.actors[0])
	}
}

func TestStatusesBecomeTypedErrors(t *testing.T) {
	for _, test := range []struct {
		status error
		want   error
	}{
		{status.Error(codes.NotFound, "`x` cannot be changed at runtime"), ErrNotFound},
		{status.Error(codes.PermissionDenied, "owners only"), ErrPermissionDenied},
		{status.Error(codes.Unauthenticated, "token required"), ErrPermissionDenied},
		{status.Error(codes.Unavailable, "down"), ErrUnavailable},
	} {
		client := start(t, &fakeServer{getError: test.status})
		_, err := client.Get(contextFor(t), nil, "a")
		if !errors.Is(err, test.want) {
			t.Errorf("error %v is not %v", err, test.want)
		}
		if st, ok := status.FromError(err); !ok || st.Code() != status.Code(test.status) {
			t.Errorf("status of %v lost", err)
		}
	}

	client := start(t, &fakeServer{getError: status.Error(codes.InvalidArgument, "invalid value for `a`: not a number")})
	_, err := client.Get(contextFor(t), nil, "a")
	var invalid *InvalidValueError
	if !errors.As(err, &invalid) || invalid.Reason != "invalid value for `a`: not a number" {
		t.Fatalf("error = %#v, want the server's reason", err)
	}

	other := status.Error(codes.Internal, "boom")
	client = start(t, &fakeServer{getError: other})
	if _, err := client.Get(contextFor(t), nil, "a"); status.Code(err) != codes.Internal {
		t.Fatalf("error = %v, want the Internal status kept", err)
	}
}

func TestWatchReconnectsWithAnInitialEvent(t *testing.T) {
	fake := &fakeServer{}
	client := start(t, fake)
	events := client.Watch(contextFor(t), nil, "a")

	want := []struct {
		initial bool
		value   string
	}{{true, `"x"`}, {false, `"y"`}, {true, `"x"`}}
	for i, expected := range want {
		event, ok := <-events
		if !ok {
			t.Fatalf("event %d: channel closed", i)
		}
		if event.Initial != expected.initial || string(event.Values[0].JSON) != expected.value {
			t.Fatalf("event %d = %+v, want initial=%v value %s", i, event, expected.initial, expected.value)
		}
	}
}

func TestWatchStopsWithContext(t *testing.T) {
	fake := &fakeServer{}
	fake.watches = 1 // every stream stays open after its initial event
	client := start(t, fake)
	ctx, cancel := context.WithCancel(contextFor(t))
	events := client.Watch(ctx, nil)
	if event := <-events; !event.Initial {
		t.Fatalf("first event = %+v, want initial", event)
	}
	cancel()
	select {
	case _, ok := <-events:
		if ok {
			t.Fatal("event after cancel")
		}
	case <-time.After(2 * time.Second):
		t.Fatal("channel not closed after cancel")
	}
}

func TestDecodeBuildsNestedObjects(t *testing.T) {
	values := []Value{
		{Key: "a.limit", JSON: json.RawMessage(`5`)},
		{Key: "a.mode", JSON: json.RawMessage(`"x"`)},
		{Key: "a.nested.k", JSON: json.RawMessage(`true`)},
		{Key: "a.gone"},
		{Key: "b.other", JSON: json.RawMessage(`1`)},
	}
	var out struct {
		Limit  int    `json:"limit"`
		Mode   string `json:"mode"`
		Nested struct {
			K bool `json:"k"`
		} `json:"nested"`
	}
	if err := Decode(values, "a", &out); err != nil {
		t.Fatalf("Decode() error = %v", err)
	}
	if out.Limit != 5 || out.Mode != "x" || !out.Nested.K {
		t.Fatalf("decoded %+v", out)
	}

	// A value at the prefix itself is the base the others override.
	base := []Value{
		{Key: "a", JSON: json.RawMessage(`{"limit": 1, "mode": "base"}`)},
		{Key: "a.limit", JSON: json.RawMessage(`7`)},
	}
	if err := Decode(base, "a", &out); err != nil || out.Limit != 7 || out.Mode != "base" {
		t.Fatalf("decoded %+v (error %v), want limit 7 over the base", out, err)
	}
}

func TestQuestionsExpire(t *testing.T) {
	now := time.Unix(1000, 0)
	questions := NewQuestions(10 * time.Minute)
	questions.now = func() time.Time { return now }

	questions.Ask("chat", "user", &settingspb.Ask{Question: "q1"}, "42")
	if question, panel, ok := questions.Pending("chat", "user"); !ok || question != "q1" || panel != "42" {
		t.Fatalf("Pending() = %q, %q, %v", question, panel, ok)
	}
	if _, _, ok := questions.Pending("chat", "someone else"); ok {
		t.Fatal("another user's question is pending")
	}

	now = now.Add(10 * time.Minute)
	if _, _, ok := questions.Pending("chat", "user"); ok {
		t.Fatal("expired question still pending")
	}

	questions.Ask("chat", "user", &settingspb.Ask{Question: "q2"}, "43")
	questions.Finish("chat", "user")
	if _, _, ok := questions.Pending("chat", "user"); ok {
		t.Fatal("finished question still pending")
	}
}

func TestActVariants(t *testing.T) {
	fake := &fakeServer{}
	client := start(t, fake)
	ctx := contextFor(t)
	view := View{Limits: &settingspb.Limits{TokenBytes: 64}, Pickers: true}

	calls := []func() error{
		func() error { _, err := client.Press(ctx, nil, view, "tok"); return err },
		func() error { _, err := client.Answer(ctx, nil, view, "q", TextInput("hello")); return err },
		func() error {
			_, err := client.Answer(ctx, nil, view, "q", PickedInput("telegram.user", json.RawMessage(`7`)))
			return err
		},
		func() error { _, err := client.Answer(ctx, nil, view, "q", CancelInput()); return err },
		func() error { _, err := client.Command(ctx, nil, view, "set a 1"); return err },
	}
	for i, call := range calls {
		if err := call(); err != nil {
			t.Fatalf("call %d: %v", i, err)
		}
	}

	acts := fake.acts
	if got := acts[0].GetPress(); got != "tok" {
		t.Errorf("press = %q", got)
	}
	if got := acts[1].GetAnswer(); got.GetQuestion() != "q" || got.GetText() != "hello" {
		t.Errorf("text answer = %v", got)
	}
	if got := acts[2].GetAnswer().GetPicked(); got.GetDomain() != "telegram.user" || len(got.GetIdsJson()) != 1 || got.GetIdsJson()[0] != "7" {
		t.Errorf("picked answer = %v", got)
	}
	if acts[3].GetAnswer().GetCancel() == nil {
		t.Errorf("cancel answer = %v", acts[3].GetAnswer())
	}
	if got := acts[4].GetCommand(); got != "set a 1" {
		t.Errorf("command = %q", got)
	}
	for i, act := range acts {
		if act.GetLimits().GetTokenBytes() != 64 || !act.GetPickers() {
			t.Errorf("act %d lost the view: %v", i, act)
		}
	}
}
