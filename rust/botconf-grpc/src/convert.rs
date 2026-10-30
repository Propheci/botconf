//! Conversions between botconf's and botconf-panel's types and the wire.

use botconf::{
    CatalogEntry, Kind, Schema, SettingsError, Snapshot, Source, View,
    command::{Listing, Outcome, ValueEntry},
};
use botconf_panel::{
    Ask, ButtonView, CommandReply, Input, Limits, Page, PanelError, Reaction, Screen, Span, Style,
    Text,
};
use serde_json::Value;
use tonic::Status;

use crate::proto;

/// Telegram's limits, used for the fields a client leaves at zero.
const DEFAULT_TOKEN_BYTES: usize = 64;
const DEFAULT_TEXT_CHARS: usize = 4096;
const DEFAULT_PER_ROW: usize = 8;

/// A setting's descriptor, its choices resolved against `view`.
pub(crate) fn setting(
    position: usize,
    entry: &CatalogEntry,
    view: &dyn View,
) -> proto::SettingDescriptor {
    proto::SettingDescriptor {
        position: u32::try_from(position).unwrap_or(u32::MAX),
        key: entry.key.clone(),
        title: entry.title.clone(),
        description: entry.description.to_string(),
        per_entry: entry.per_entry,
        kind: Some(kind(&entry.kind, view)),
        section: entry.section.as_deref().unwrap_or_default().to_string(),
    }
}

/// A kind, with every list of choices resolved against `view`: a
/// `Choices::Dynamic` function never goes on the wire, only what it lists now.
pub(crate) fn kind(kind: &Kind, view: &dyn View) -> proto::Kind {
    use proto::kind::Kind as Wire;

    let choices = |kind: &Kind| -> Vec<proto::Choice> {
        kind.choices(view)
            .into_iter()
            .map(|choice| proto::Choice {
                value: choice.value,
                label: choice.label,
            })
            .collect()
    };
    let wire = match kind {
        Kind::Json => Wire::Json(proto::JsonKind {}),
        Kind::Text { optional } => Wire::Text(proto::TextKind {
            optional: *optional,
        }),
        Kind::Number {
            min,
            max,
            unit,
            suggestions,
            optional,
        } => Wire::Number(proto::NumberKind {
            min: *min,
            max: *max,
            unit: unit.to_string(),
            suggestions: suggestions.to_vec(),
            optional: *optional,
        }),
        Kind::OneOf {
            custom, optional, ..
        } => Wire::OneOf(proto::OneOfKind {
            choices: choices(kind),
            custom: *custom,
            optional: *optional,
        }),
        Kind::SetOf { inverted, .. } => Wire::SetOf(proto::SetOfKind {
            choices: choices(kind),
            inverted: *inverted,
        }),
        Kind::Ref { domain, many } => Wire::Ref(proto::RefKind {
            domain: domain.to_string(),
            many: *many,
        }),
        Kind::Form(form) => Wire::Form(proto::FormKind {
            fields: form
                .fields
                .iter()
                .map(|field| proto::FormField {
                    key: field.key.to_string(),
                    title: field.title.to_string(),
                    kind: Some(self::kind(&field.kind, view)),
                    group: field.group.as_deref().unwrap_or_default().to_string(),
                    required: field.required,
                    when: field
                        .when
                        .as_ref()
                        .map(|(key, value)| proto::FieldCondition {
                            key: key.to_string(),
                            value: value.to_string(),
                        }),
                })
                .collect(),
            initial_json: form.initial.to_string(),
        }),
        Kind::Map { names, value } => Wire::Map(Box::new(proto::MapKind {
            limited_names: names.is_some(),
            names: choices(kind),
            value: Some(Box::new(self::kind(value, view))),
        })),
    };
    proto::Kind { kind: Some(wire) }
}

/// The wire form of a source: the enum, and the name of an `Other` source.
pub(crate) fn source(source: &Source) -> (proto::Source, String) {
    match source {
        Source::Default => (proto::Source::Default, String::new()),
        Source::File => (proto::Source::File, String::new()),
        Source::Environment => (proto::Source::Environment, String::new()),
        Source::Stored => (proto::Source::Stored, String::new()),
        Source::Other(name) => (proto::Source::Other, name.clone()),
    }
}

/// A value as JSON text.
pub(crate) fn json(value: &Value) -> String {
    value.to_string()
}

pub(crate) fn value_entry(entry: &ValueEntry) -> proto::ValueEntry {
    let (source, source_name) = source(&entry.source);
    proto::ValueEntry {
        key: entry.key.clone(),
        value_json: entry.value.as_ref().map(json),
        source: source.into(),
        source_name,
        description: entry.description.as_deref().unwrap_or_default().to_string(),
    }
}

/// `key`'s value and source in `snapshot`.
pub(crate) fn current<S: Schema>(snapshot: &Snapshot<S>, key: &str) -> proto::ValueEntry {
    let (source, source_name) = source(&snapshot.source(key));
    proto::ValueEntry {
        key: key.to_string(),
        value_json: snapshot.value(key).as_ref().map(json),
        source: source.into(),
        source_name,
        description: String::new(),
    }
}

pub(crate) fn listing(listing: &Listing) -> proto::ListResponse {
    proto::ListResponse {
        settings: listing.settings.iter().map(value_entry).collect(),
        entries: listing.entries.iter().map(value_entry).collect(),
        ignored: listing
            .ignored
            .iter()
            .map(|(key, reason)| proto::Ignored {
                key: key.clone(),
                reason: reason.clone(),
            })
            .collect(),
        parked: listing
            .parked
            .iter()
            .map(|(key, value)| proto::ValueEntry {
                key: key.clone(),
                value_json: Some(json(value)),
                source: proto::Source::Stored.into(),
                ..Default::default()
            })
            .collect(),
    }
}

/// The response to a change; `key` is the key the request named, for an
/// unset of nothing.
pub(crate) fn change_response<S: Schema>(
    outcome: &Outcome<S>,
    snapshot: &Snapshot<S>,
    key: Option<&str>,
) -> proto::ChangeResponse {
    match outcome {
        Outcome::Changed {
            previous,
            current,
            lints,
            ..
        } => proto::ChangeResponse {
            changed: true,
            previous_json: previous.as_ref().map(json),
            current: Some(value_entry(current)),
            lints: lints.clone(),
            ..Default::default()
        },
        Outcome::Reloaded {
            overrides, ignored, ..
        } => proto::ChangeResponse {
            changed: true,
            overrides: u32::try_from(*overrides).unwrap_or(u32::MAX),
            ignored: u32::try_from(*ignored).unwrap_or(u32::MAX),
            ..Default::default()
        },
        Outcome::NotOverridden(_) | Outcome::Listing(_) | Outcome::Value(_) => {
            proto::ChangeResponse {
                changed: false,
                current: key.map(|key| self::current(snapshot, key)),
                ..Default::default()
            }
        }
    }
}

/// The status a failed store operation is answered with.
pub(crate) fn settings_status(error: &SettingsError) -> Status {
    match error {
        SettingsError::UnknownKey(_) => Status::not_found(error.to_string()),
        SettingsError::InvalidValue { .. }
        | SettingsError::NotAList(_)
        | SettingsError::NotAnEntry(_) => Status::invalid_argument(error.to_string()),
        SettingsError::InvalidBase(_) => Status::failed_precondition(error.to_string()),
        SettingsError::Storage(_) => {
            tracing::error!(error = %Report(error), "the settings storage failed");
            Status::internal("the settings storage failed")
        }
    }
}

/// An error with its sources, for logs.
struct Report<'a>(&'a dyn std::error::Error);

impl std::fmt::Display for Report<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)?;
        let mut source = self.0.source();
        while let Some(error) = source {
            write!(f, ": {error}")?;
            source = error.source();
        }
        Ok(())
    }
}

/// The client's limits; fields left at zero take Telegram's.
pub(crate) fn limits(limits: Option<&proto::Limits>) -> Limits {
    let limits = limits.cloned().unwrap_or_default();
    let or = |value: u32, default: usize| match value {
        0 => default,
        value => usize::try_from(value).unwrap_or(usize::MAX),
    };
    Limits {
        token_bytes: or(limits.token_bytes, DEFAULT_TOKEN_BYTES),
        text_chars: or(limits.text_chars, DEFAULT_TEXT_CHARS),
        max_rows: (limits.max_rows != 0).then(|| or(limits.max_rows, 0)),
        max_per_row: or(limits.max_per_row, DEFAULT_PER_ROW),
    }
}

/// The page a client asks for; none is the home screen.
pub(crate) fn page(page: Option<&proto::Page>) -> Page {
    match page.and_then(|page| page.page.as_ref()) {
        None | Some(proto::page::Page::Home(_)) => Page::Home,
        Some(proto::page::Page::Section(id)) => Page::Section(id.clone()),
    }
}

/// An answer's input. Picked ids must be JSON.
pub(crate) fn input(input: Option<&proto::answer::Input>) -> Result<Input, Status> {
    match input {
        None => Err(Status::invalid_argument("the answer is empty")),
        Some(proto::answer::Input::Text(text)) => Ok(Input::Text(text.clone())),
        Some(proto::answer::Input::Cancel(_)) => Ok(Input::Cancel),
        Some(proto::answer::Input::Picked(picked)) => {
            let ids = picked
                .ids_json
                .iter()
                .map(|id| {
                    serde_json::from_str(id).map_err(|error| {
                        Status::invalid_argument(format!("picked id {id:?} is not JSON: {error}"))
                    })
                })
                .collect::<Result<_, _>>()?;
            Ok(Input::Picked {
                domain: picked.domain.clone(),
                ids,
            })
        }
    }
}

fn style(style: Style) -> proto::Style {
    match style {
        Style::Plain => proto::Style::Plain,
        Style::Bold => proto::Style::Bold,
        Style::Italic => proto::Style::Italic,
        Style::Code => proto::Style::Code,
        Style::CodeBlock => proto::Style::CodeBlock,
    }
}

pub(crate) fn text(text: &Text) -> proto::Text {
    proto::Text {
        spans: text
            .0
            .iter()
            .map(|span: &Span| proto::Span {
                style: style(span.style).into(),
                text: span.text.clone(),
            })
            .collect(),
    }
}

fn rows(rows: &[Vec<ButtonView>]) -> Vec<proto::Row> {
    rows.iter()
        .map(|row| proto::Row {
            buttons: row
                .iter()
                .map(|button| proto::Button {
                    label: button.label.clone(),
                    token: button.token.clone(),
                })
                .collect(),
        })
        .collect()
}

pub(crate) fn screen(screen: &Screen) -> proto::Screen {
    proto::Screen {
        text: Some(text(&screen.text)),
        rows: rows(&screen.rows),
    }
}

fn ask(ask: &Ask) -> proto::Ask {
    proto::Ask {
        question: ask.question.encode(),
        text: Some(text(&ask.text)),
        picker: ask.picker.as_ref().map(|picker| proto::Picker {
            domain: picker.domain.clone(),
            many: picker.many,
        }),
        current: ask.current.clone(),
        title: ask.title.clone(),
        rows: rows(&ask.rows),
        back: ask.back.as_ref().map(|back| proto::Button {
            label: back.label.clone(),
            token: back.token.clone(),
        }),
    }
}

/// A reaction's response, and the change it made for the host's hook.
pub(crate) fn reaction<S: Schema>(
    reaction: Reaction<S>,
) -> (proto::ActResponse, Option<botconf::Change<S>>) {
    let response = proto::ActResponse {
        screen: reaction.screen.as_ref().map(screen),
        post: reaction.post.as_ref().map(screen),
        notice: reaction.notice,
        ask: reaction.ask.as_ref().map(ask),
        closed: reaction.closed,
        ..Default::default()
    };
    (response, reaction.change)
}

/// A text command's response, and the change it made.
pub(crate) fn command_reply<S: Schema>(
    reply: CommandReply<S>,
) -> (proto::ActResponse, Option<botconf::Change<S>>) {
    match reply {
        CommandReply::Panel(panel) => (
            proto::ActResponse {
                post: Some(screen(&panel)),
                ..Default::default()
            },
            None,
        ),
        CommandReply::Text {
            text: reply,
            change,
        } => (
            proto::ActResponse {
                reply: Some(text(&reply)),
                ..Default::default()
            },
            change,
        ),
    }
}

/// A refused or stale press or answer comes back in the response, for the
/// client to show; infrastructure failures are errors.
pub(crate) fn panel_error(error: PanelError) -> Result<proto::ActResponse, Status> {
    match error {
        PanelError::Rejected(reason) => Ok(proto::ActResponse {
            rejected: Some(reason),
            ..Default::default()
        }),
        PanelError::Stale { notice, screen } => Ok(proto::ActResponse {
            stale: true,
            notice: Some(notice),
            screen: Some(self::screen(&screen)),
            ..Default::default()
        }),
        PanelError::Storage(error) => Err(settings_status(&error)),
        PanelError::Tokens(error) => {
            tracing::error!(error = %Report(error.as_ref()), "the panel's token store failed");
            Err(Status::internal("the panel's token store failed"))
        }
    }
}

#[cfg(test)]
mod tests {
    use botconf::{
        Choice, Choices, DynamicChoices, Field, FixedChoice, Form, RuntimeSetting, UnknownKey,
    };
    use botconf_panel::{AskKind, Picker, Question, Target};
    use serde_json::json;
    use tonic::Code;

    use super::*;

    /// A view that knows one map, for `KeysOf`.
    struct Lamps;

    impl View for Lamps {
        fn value(&self, key: &str) -> Option<Value> {
            (key == "lamps").then(|| json!({ "desk": {}, "bed": {} }))
        }

        fn section_any(&self, _id: &str) -> Option<&dyn std::any::Any> {
            None
        }

        fn derived_any(&self) -> &dyn std::any::Any {
            &()
        }

        fn schema_any(&self) -> &dyn std::any::Any {
            &()
        }
    }

    fn flavours(_: &dyn View) -> Vec<Choice> {
        vec![Choice::new("mint", "🌿 mint")]
    }

    const LEVELS: &[FixedChoice] = &[FixedChoice::new("info", "Info")];
    const FIELDS: &[Field] = &[
        Field::new("on", "On", Kind::text(false)),
        Field::new(
            "flavour",
            "Flavour",
            Kind::one_of(Choices::Dynamic(DynamicChoices(flavours)), false, true),
        )
        .group("look")
        .required()
        .when("on", "yes"),
    ];
    const FORM: &Form = &Form::new(FIELDS, r#"{"on": "yes"}"#);
    const FORM_KIND: &Kind = &Kind::form(FORM);

    fn wire(kind: &Kind) -> proto::kind::Kind {
        super::kind(kind, &Lamps).kind.unwrap()
    }

    #[test]
    fn kinds_carry_resolved_choices() {
        use proto::kind::Kind as Wire;

        assert_eq!(wire(&Kind::Json), Wire::Json(proto::JsonKind {}));
        assert_eq!(
            wire(&Kind::text(true)),
            Wire::Text(proto::TextKind { optional: true })
        );
        assert_eq!(
            wire(&Kind::number(1, 9, "%", &[1, 5], false)),
            Wire::Number(proto::NumberKind {
                min: 1,
                max: 9,
                unit: "%".into(),
                suggestions: vec![1, 5],
                optional: false,
            })
        );
        let Wire::OneOf(one_of) = wire(&Kind::one_of(
            Choices::Dynamic(DynamicChoices(flavours)),
            true,
            false,
        )) else {
            panic!("not one-of");
        };
        assert_eq!(
            one_of.choices,
            [proto::Choice {
                value: "mint".into(),
                label: "🌿 mint".into()
            }]
        );
        assert!(one_of.custom);

        let Wire::SetOf(set_of) = wire(&Kind::set_of(Choices::keys_of("lamps"), true)) else {
            panic!("not set-of");
        };
        let values: Vec<_> = set_of.choices.iter().map(|choice| &choice.value).collect();
        assert_eq!(values, ["bed", "desk"]);
        assert!(set_of.inverted);

        assert_eq!(
            wire(&Kind::reference("telegram.user", true)),
            Wire::Ref(proto::RefKind {
                domain: "telegram.user".into(),
                many: true
            })
        );
    }

    #[test]
    fn form_fields_round_trip_required_and_when() {
        use proto::kind::Kind as Wire;

        let Wire::Form(form) = wire(FORM_KIND) else {
            panic!("not a form");
        };
        assert!(!form.fields[0].required);
        assert_eq!(form.fields[0].when, None);
        assert!(form.fields[1].required);
        assert_eq!(
            form.fields[1].when,
            Some(proto::FieldCondition {
                key: "on".into(),
                value: "yes".into(),
            })
        );
        // What goes on the wire is what decodes from it.
        let decoded = <proto::FormField as prost::Message>::decode(
            prost::Message::encode_to_vec(&form.fields[1]).as_slice(),
        )
        .unwrap();
        assert_eq!(decoded, form.fields[1]);
    }

    #[test]
    fn forms_and_maps_resolve_nested_choices() {
        use proto::kind::Kind as Wire;

        let Wire::Form(form) = wire(FORM_KIND) else {
            panic!("not a form");
        };
        assert_eq!(form.initial_json, r#"{"on": "yes"}"#);
        assert_eq!(form.fields[1].group, "look");
        assert_eq!(form.fields[0].group, "");
        let Some(Wire::OneOf(flavour)) = form.fields[1].kind.as_ref().unwrap().kind.as_ref() else {
            panic!("not one-of");
        };
        assert_eq!(flavour.choices[0].value, "mint");

        let limited = Kind::map(Some(Choices::fixed(LEVELS)), FORM_KIND);
        let Wire::Map(map) = wire(&limited) else {
            panic!("not a map");
        };
        assert!(map.limited_names);
        assert_eq!(map.names[0].label, "Info");
        assert!(matches!(map.value.unwrap().kind, Some(Wire::Form(_))));

        let Wire::Map(open) = wire(&Kind::map(None, &Kind::Json)) else {
            panic!("not a map");
        };
        assert!(!open.limited_names);
        assert!(open.names.is_empty());
    }

    #[test]
    fn settings_carry_their_position_and_section() {
        let entry = CatalogEntry {
            key: "features.lamp.level".into(),
            title: "Level".into(),
            description: "How much".into(),
            per_entry: false,
            kind: Kind::one_of(Choices::fixed(LEVELS), false, false),
            section: Some("lamp".into()),
        };
        let wire = setting(7, &entry, &Lamps);
        assert_eq!(wire.position, 7);
        assert_eq!(wire.key, "features.lamp.level");
        assert_eq!(wire.section, "lamp");
        assert_eq!(wire.description, "How much");

        let top = CatalogEntry {
            section: None,
            ..entry
        };
        assert_eq!(setting(0, &top, &Lamps).section, "");
        // Declared with `RuntimeSetting` in hosts; only its catalog entry
        // reaches the wire.
        let _ = RuntimeSetting::new("x", "");
    }

    #[test]
    fn sources_name_other_origins() {
        assert_eq!(
            source(&Source::Default),
            (proto::Source::Default, String::new())
        );
        assert_eq!(source(&Source::File), (proto::Source::File, String::new()));
        assert_eq!(
            source(&Source::Environment),
            (proto::Source::Environment, String::new())
        );
        assert_eq!(
            source(&Source::Stored),
            (proto::Source::Stored, String::new())
        );
        assert_eq!(
            source(&Source::Other("connector a".into())),
            (proto::Source::Other, "connector a".into())
        );
    }

    #[test]
    fn values_are_json_and_absent_values_are_unset() {
        let entry = ValueEntry {
            key: "greeting".into(),
            value: Some(json!("hi")),
            source: Source::Stored,
            description: Some("What to say".into()),
        };
        let wire = value_entry(&entry);
        assert_eq!(wire.value_json.as_deref(), Some(r#""hi""#));
        assert_eq!(wire.source(), proto::Source::Stored);
        assert_eq!(wire.description, "What to say");

        let absent = value_entry(&ValueEntry {
            value: None,
            description: None,
            ..entry
        });
        assert_eq!(absent.value_json, None);
        assert_eq!(absent.description, "");
    }

    #[test]
    fn store_errors_get_fitting_codes() {
        let code = |error: SettingsError| settings_status(&error).code();
        assert_eq!(code(UnknownKey("x".into()).into()), Code::NotFound);
        for error in [
            SettingsError::InvalidValue {
                key: "x".into(),
                reason: "too big".into(),
            },
            SettingsError::NotAList("x".into()),
            SettingsError::NotAnEntry("x".into()),
        ] {
            let status = settings_status(&error);
            assert_eq!(status.code(), Code::InvalidArgument);
            assert_eq!(status.message(), error.to_string());
        }
        assert_eq!(
            code(SettingsError::InvalidBase(botconf::BuildError::Invalid(
                "x".into()
            ))),
            Code::FailedPrecondition
        );
        let storage = settings_status(&SettingsError::Storage("disk full".into()));
        assert_eq!(storage.code(), Code::Internal);
        assert!(
            !storage.message().contains("disk full"),
            "details are logged, not sent"
        );
    }

    #[test]
    fn missing_limits_take_telegrams() {
        assert_eq!(
            limits(None),
            Limits {
                token_bytes: 64,
                text_chars: 4096,
                max_rows: None,
                max_per_row: 8,
            }
        );
        let discord = proto::Limits {
            token_bytes: 100,
            text_chars: 2000,
            max_rows: 5,
            max_per_row: 5,
        };
        assert_eq!(
            limits(Some(&discord)),
            Limits {
                token_bytes: 100,
                text_chars: 2000,
                max_rows: Some(5),
                max_per_row: 5,
            }
        );
    }

    #[test]
    fn pages_default_to_home() {
        assert_eq!(page(None), Page::Home);
        let section = proto::Page {
            page: Some(proto::page::Page::Section("lamp".into())),
        };
        assert_eq!(page(Some(&section)), Page::Section("lamp".into()));
    }

    #[test]
    fn picked_ids_must_be_json() {
        let picked = |ids: &[&str]| {
            proto::answer::Input::Picked(proto::Picked {
                domain: "telegram.user".into(),
                ids_json: ids.iter().map(|id| (*id).to_string()).collect(),
            })
        };
        assert_eq!(
            input(Some(&picked(&["7", "\"@ann\""]))).unwrap(),
            Input::Picked {
                domain: "telegram.user".into(),
                ids: vec![json!(7), json!("@ann")],
            }
        );
        assert_eq!(
            input(Some(&picked(&["not json"]))).unwrap_err().code(),
            Code::InvalidArgument
        );
        assert_eq!(
            input(Some(&proto::answer::Input::Text("x".into()))).unwrap(),
            Input::Text("x".into())
        );
        assert_eq!(
            input(Some(&proto::answer::Input::Cancel(proto::Cancel {}))).unwrap(),
            Input::Cancel
        );
        assert_eq!(input(None).unwrap_err().code(), Code::InvalidArgument);
    }

    #[test]
    fn screens_and_questions_keep_their_parts() {
        let screen = Screen {
            text: Text(vec![Span::bold("Settings"), Span::plain("\nhi")]),
            rows: vec![vec![ButtonView {
                label: "✏️ Change".into(),
                token: "a:1:v".into(),
            }]],
        };
        let wire = super::screen(&screen);
        let spans = wire.text.unwrap().spans;
        assert_eq!(spans[0].style(), proto::Style::Bold);
        assert_eq!(spans[1].text, "\nhi");
        assert_eq!(wire.rows[0].buttons[0].token, "a:1:v");

        let question = Question::new(Target::setting(1), AskKind::Value);
        let asked = ask(&Ask {
            question: question.clone(),
            title: "Greeting".into(),
            text: Text::plain("Send the greeting"),
            picker: Some(Picker {
                domain: "telegram.user".into(),
                many: true,
            }),
            current: Some("hi".into()),
            rows: screen.rows.clone(),
            back: Some(ButtonView {
                label: "‹ Back".into(),
                token: "o:s:1".into(),
            }),
        });
        assert_eq!(Question::parse(&asked.question), Some(question));
        assert_eq!(asked.title, "Greeting");
        assert_eq!(asked.picker.unwrap().domain, "telegram.user");
        assert_eq!(asked.rows.len(), 1);
        assert_eq!(asked.back.unwrap().token, "o:s:1");
    }

    #[test]
    fn panel_refusals_are_responses_and_failures_are_errors() {
        let rejected = panel_error(PanelError::Rejected("no".into())).unwrap();
        assert_eq!(rejected.rejected.as_deref(), Some("no"));

        let stale = panel_error(PanelError::Stale {
            notice: "out of date".into(),
            screen: Screen::default(),
        })
        .unwrap();
        assert!(stale.stale);
        assert_eq!(stale.notice.as_deref(), Some("out of date"));
        assert!(stale.screen.is_some());

        let storage = PanelError::Storage(SettingsError::Storage("disk".into()));
        assert_eq!(panel_error(storage).unwrap_err().code(), Code::Internal);
        let tokens = PanelError::Tokens("disk".into());
        assert_eq!(panel_error(tokens).unwrap_err().code(), Code::Internal);
    }
}
