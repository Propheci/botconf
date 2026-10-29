// Package settings is a client for a program's runtime settings, served by
// botconf-grpc: read and change them, follow changes, and drive the settings
// panel.
//
// The client never decides who may do what. It sends the actor a request acts
// for and the credentials it was given; the server's authorizer decides.
package settings

import (
	"context"
	"encoding/json"
	"fmt"
	"time"

	"github.com/propheci/botconf/go/settingspb"
	"google.golang.org/grpc"
	"google.golang.org/grpc/credentials/insecure"
	"google.golang.org/grpc/metadata"
)

// Client talks to one botconf.v1.Settings server.
type Client struct {
	connection *grpc.ClientConn
	api        settingspb.SettingsClient
	options    options
}

type options struct {
	bearer      string
	dialOptions []grpc.DialOption
	actor       *Actor
	watchErrors func(error)
	backoff     backoff
}

type backoff struct {
	initial, max time.Duration
}

// Option configures a Client.
type Option func(*options)

// WithBearer sends `authorization: Bearer <token>` with every call.
func WithBearer(token string) Option {
	return func(o *options) { o.bearer = token }
}

// WithDialOptions passes options to grpc.NewClient, e.g. TLS credentials,
// which replace the default insecure transport.
func WithDialOptions(dialOptions ...grpc.DialOption) Option {
	return func(o *options) { o.dialOptions = append(o.dialOptions, dialOptions...) }
}

// WithActor sets the actor for calls made with a nil actor.
func WithActor(domain, id string) Option {
	return func(o *options) { o.actor = &Actor{Domain: domain, ID: id} }
}

// WithWatchErrors is called with each error that ends a Watch stream before
// the client reconnects, e.g. to log it.
func WithWatchErrors(handle func(error)) Option {
	return func(o *options) { o.watchErrors = handle }
}

// withBackoff sets Watch's reconnect delays; tests shorten them.
func withBackoff(initial, max time.Duration) Option {
	return func(o *options) { o.backoff = backoff{initial: initial, max: max} }
}

// Dial connects to a settings server. The transport is insecure unless
// WithDialOptions gives credentials, which suits a server bound to a local
// address. Like grpc.NewClient, it does not wait for the connection.
func Dial(address string, opts ...Option) (*Client, error) {
	o := options{backoff: backoff{initial: time.Second, max: 30 * time.Second}}
	for _, opt := range opts {
		opt(&o)
	}
	dialOptions := append([]grpc.DialOption{grpc.WithTransportCredentials(insecure.NewCredentials())}, o.dialOptions...)
	connection, err := grpc.NewClient(address, dialOptions...)
	if err != nil {
		return nil, fmt.Errorf("connect to settings server %s: %w", address, err)
	}
	return &Client{connection: connection, api: settingspb.NewSettingsClient(connection), options: o}, nil
}

// Close closes the connection.
func (c *Client) Close() error {
	return c.connection.Close()
}

// Actor is who a request acts for, as the client claims it, e.g. the
// Telegram user who pressed a button. The server decides whether to trust it.
type Actor struct {
	Domain string
	ID     string
}

// Source is where a value comes from.
type Source = settingspb.Source

// Value is one key's effective value.
type Value struct {
	Key string
	// JSON is the value as JSON; nil when the key has no value at all.
	JSON   json.RawMessage
	Source Source
	// SourceName names the source when Source is SOURCE_OTHER.
	SourceName string
}

func valueOf(entry *settingspb.ValueEntry) Value {
	value := Value{Key: entry.GetKey(), Source: entry.GetSource(), SourceName: entry.GetSourceName()}
	if entry.ValueJson != nil {
		value.JSON = json.RawMessage(entry.GetValueJson())
	}
	return value
}

// call prepares ctx for a call: the bearer token.
func (c *Client) call(ctx context.Context) context.Context {
	if c.options.bearer != "" {
		ctx = metadata.AppendToOutgoingContext(ctx, "authorization", "Bearer "+c.options.bearer)
	}
	return ctx
}

// actor is the actor to send: the explicit one, else the default.
func (c *Client) actor(actor *Actor) *settingspb.Actor {
	if actor == nil {
		actor = c.options.actor
	}
	if actor == nil {
		return nil
	}
	return &settingspb.Actor{Domain: actor.Domain, Id: actor.ID}
}

// Describe returns the runtime settings with their kinds.
func (c *Client) Describe(ctx context.Context, actor *Actor) (*settingspb.DescribeResponse, error) {
	response, err := c.api.Describe(c.call(ctx), &settingspb.DescribeRequest{Actor: c.actor(actor)})
	return response, convert(err)
}

// List returns every runtime setting's value and source.
func (c *Client) List(ctx context.Context, actor *Actor) (*settingspb.ListResponse, error) {
	response, err := c.api.List(c.call(ctx), &settingspb.ListRequest{Actor: c.actor(actor)})
	return response, convert(err)
}

// Get returns one key's value.
func (c *Client) Get(ctx context.Context, actor *Actor, key string) (Value, error) {
	response, err := c.api.Get(c.call(ctx), &settingspb.GetRequest{Actor: c.actor(actor), Key: key})
	if err != nil {
		return Value{}, convert(err)
	}
	return valueOf(response), nil
}

// Set overrides key with value, read like typed input: JSON if it parses,
// text otherwise.
func (c *Client) Set(ctx context.Context, actor *Actor, key, value string) (*settingspb.ChangeResponse, error) {
	response, err := c.api.Set(c.call(ctx), &settingspb.EditRequest{Actor: c.actor(actor), Key: key, Value: value})
	return response, convert(err)
}

// Unset removes the override of key and of its entries.
func (c *Client) Unset(ctx context.Context, actor *Actor, key string) (*settingspb.ChangeResponse, error) {
	response, err := c.api.Unset(c.call(ctx), &settingspb.UnsetRequest{Actor: c.actor(actor), Key: key})
	return response, convert(err)
}

// Add appends item to the list at key, unless it is there already.
func (c *Client) Add(ctx context.Context, actor *Actor, key, item string) (*settingspb.ChangeResponse, error) {
	response, err := c.api.Add(c.call(ctx), &settingspb.EditRequest{Actor: c.actor(actor), Key: key, Value: item})
	return response, convert(err)
}

// Remove removes every occurrence of item from the list at key.
func (c *Client) Remove(ctx context.Context, actor *Actor, key, item string) (*settingspb.ChangeResponse, error) {
	response, err := c.api.Remove(c.call(ctx), &settingspb.EditRequest{Actor: c.actor(actor), Key: key, Value: item})
	return response, convert(err)
}

// Reload asks the server to read its sources again.
func (c *Client) Reload(ctx context.Context, actor *Actor) (*settingspb.ChangeResponse, error) {
	response, err := c.api.Reload(c.call(ctx), &settingspb.ReloadRequest{Actor: c.actor(actor)})
	return response, convert(err)
}
