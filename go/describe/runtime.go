package describe

import (
	"strings"

	"github.com/propheci/botconf/go/settingspb"
)

// RuntimeSettings are the descriptors of the entries marked runtime, with
// their absolute dotted keys, below tables (not lists or maps). A host
// declares them to a botconf store (wuphf's connectors: to the core).
func (d *Description) RuntimeSettings() []*settingspb.SettingDescriptor {
	var settings []*settingspb.SettingDescriptor
	collectRuntime(&d.Table, "", &settings)
	return settings
}

func collectRuntime(table *Table, prefix string, settings *[]*settingspb.SettingDescriptor) {
	for index := range table.Entries {
		entry := &table.Entries[index]
		key := entry.Key
		if prefix != "" {
			key = prefix + "." + key
		}
		switch {
		case entry.Runtime:
			*settings = append(*settings, &settingspb.SettingDescriptor{
				Key:         key,
				Title:       entry.Title,
				Description: summary(entry.Description),
				Kind:        panelKind(entry.Node, entry.Nullable),
			})
		case entry.Node.Table != nil:
			collectRuntime(entry.Node.Table, key, settings)
		}
	}
}

// summary is a description's first paragraph: a panel shows the summary,
// the file and a setup screen the whole.
func summary(description string) string {
	first, _, _ := strings.Cut(description, "\n\n")
	return first
}

// panelKind is the settings panel's editor for a node.
func panelKind(node Node, optional bool) *settingspb.Kind {
	value := node.Value
	if value == nil {
		return &settingspb.Kind{Kind: &settingspb.Kind_Json{Json: &settingspb.JsonKind{}}}
	}
	oneOf := func(choices ...*settingspb.Choice) *settingspb.Kind {
		return &settingspb.Kind{Kind: &settingspb.Kind_OneOf{OneOf: &settingspb.OneOfKind{
			Choices:  choices,
			Optional: optional,
		}}}
	}
	switch value.Kind {
	case Text, Secret, EnvVar:
		return &settingspb.Kind{Kind: &settingspb.Kind_Text{Text: &settingspb.TextKind{
			Optional: optional,
		}}}
	case Bool:
		return oneOf(
			&settingspb.Choice{Value: "true", Label: "✅ Yes"},
			&settingspb.Choice{Value: "false", Label: "❌ No"},
		)
	case Int:
		return &settingspb.Kind{Kind: &settingspb.Kind_Number{Number: &settingspb.NumberKind{
			Min:      value.Min,
			Max:      value.Max,
			Optional: optional,
		}}}
	case OneOf:
		choices := make([]*settingspb.Choice, 0, len(value.Choices))
		for _, choice := range value.Choices {
			choices = append(choices, &settingspb.Choice{Value: choice, Label: choice})
		}
		return oneOf(choices...)
	default:
		return &settingspb.Kind{Kind: &settingspb.Kind_Json{Json: &settingspb.JsonKind{}}}
	}
}
