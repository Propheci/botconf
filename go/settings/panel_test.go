package settings

import "testing"

// Opened leaves the page unset, so the host opens the panel as its command
// does; Home asks for the home screen itself.
func TestOpenedLeavesThePageUnset(t *testing.T) {
	if page := Opened().proto(); page != nil {
		t.Fatalf("Opened().proto() = %v, want nil", page)
	}
	if page := Home().proto(); page.GetHome() == nil {
		t.Fatalf("Home().proto() = %v, want the home page", page)
	}
}

func TestViewCarriesItsContext(t *testing.T) {
	if wire := (View{}).context(); wire != nil {
		t.Fatalf("no context = %v, want nil", wire)
	}
	wire := View{Context: &Context{Chat: "-100", Thread: "7"}}.context()
	if wire.GetChat() != "-100" || wire.GetThread() != "7" {
		t.Fatalf("context = %v", wire)
	}
	if wire := (View{Context: &Context{Chat: "-100"}}).context(); wire.Thread != nil {
		t.Fatalf("no thread = %v, want none", wire.Thread)
	}
}
