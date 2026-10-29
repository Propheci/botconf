package settings

import (
	"context"
	"encoding/json"
	"fmt"
	"strings"
	"time"

	"github.com/propheci/botconf/go/settingspb"
)

// Event is what Watch delivers.
type Event struct {
	// Values are the keys whose value or source changed; in an Initial
	// event, every runtime key under the watched prefixes.
	Values []Value
	// Initial is true for the first event after every (re)connect, so
	// Values holds the whole state and nothing in between was missed.
	Initial bool
	// CatalogChanged: sections under the prefixes were registered or
	// removed; Describe again for their settings.
	CatalogChanged bool
}

// Watch follows the keys under prefixes (all keys when none are given) until
// ctx ends. Events arrive in order on the returned channel, which closes when
// ctx ends. When the stream breaks, Watch reconnects with backoff (1s
// doubling to 30s), and each reconnect starts with an Initial event.
func (c *Client) Watch(ctx context.Context, actor *Actor, prefixes ...string) <-chan Event {
	events := make(chan Event)
	request := &settingspb.WatchRequest{Actor: c.actor(actor), Prefixes: prefixes}
	go func() {
		defer close(events)
		delay := c.options.backoff.initial
		for {
			received, err := c.watchOnce(ctx, request, events)
			if ctx.Err() != nil {
				return
			}
			if received {
				delay = c.options.backoff.initial
			}
			if c.options.watchErrors != nil {
				c.options.watchErrors(err)
			}
			select {
			case <-ctx.Done():
				return
			case <-time.After(delay):
			}
			if delay *= 2; delay > c.options.backoff.max {
				delay = c.options.backoff.max
			}
		}
	}()
	return events
}

// watchOnce runs one Watch stream until it ends, forwarding its events. It
// reports whether any event arrived, and why the stream ended.
func (c *Client) watchOnce(ctx context.Context, request *settingspb.WatchRequest, events chan<- Event) (bool, error) {
	ctx, cancel := context.WithCancel(ctx)
	defer cancel()
	stream, err := c.api.Watch(c.call(ctx), request)
	if err != nil {
		return false, convert(err)
	}
	received := false
	for {
		message, err := stream.Recv()
		if err != nil {
			return received, convert(err)
		}
		received = true
		event := Event{Initial: message.GetInitial(), CatalogChanged: message.GetCatalogChanged()}
		for _, entry := range message.GetValues() {
			event.Values = append(event.Values, valueOf(entry))
		}
		select {
		case events <- event:
		case <-ctx.Done():
			return received, ctx.Err()
		}
	}
}

// Decode assembles the values under prefix into one JSON object and
// unmarshals it into out, e.g. a connector's own settings struct: a value at
// `prefix.a.b` becomes field `b` of object `a`. A value at prefix itself is
// the base the others are laid over. Values without a value are left out.
func Decode(values []Value, prefix string, out any) error {
	root := map[string]any{}
	for _, value := range values {
		if value.JSON == nil {
			continue
		}
		var decoded any
		if err := json.Unmarshal(value.JSON, &decoded); err != nil {
			return fmt.Errorf("decode %s: %w", value.Key, err)
		}
		if value.Key == prefix {
			if object, ok := decoded.(map[string]any); ok {
				merge(root, object)
			}
			continue
		}
		path, ok := strings.CutPrefix(value.Key, prefix+".")
		if prefix == "" {
			path, ok = value.Key, true
		}
		if !ok {
			continue
		}
		place(root, strings.Split(path, "."), decoded)
	}
	encoded, err := json.Marshal(root)
	if err != nil {
		return err
	}
	return json.Unmarshal(encoded, out)
}

// place sets path in object to value, creating the objects on the way.
func place(object map[string]any, path []string, value any) {
	for _, segment := range path[:len(path)-1] {
		next, ok := object[segment].(map[string]any)
		if !ok {
			next = map[string]any{}
			object[segment] = next
		}
		object = next
	}
	last := path[len(path)-1]
	if existing, ok := object[last].(map[string]any); ok {
		if incoming, ok := value.(map[string]any); ok {
			merge(existing, incoming)
			return
		}
	}
	object[last] = value
}

// merge lays incoming over object, key by key.
func merge(object, incoming map[string]any) {
	for key, value := range incoming {
		place(object, []string{key}, value)
	}
}
