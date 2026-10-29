//! The text form of the settings command: `/config get|set|add|… <key>
//! [value]`.

use botconf::{
    Change, Schema, SettingsError,
    command::{self, Listing, Outcome, SettingsCommand, ValueEntry},
};

use crate::{
    CommandReply, Panel, PanelError, Viewer,
    text::{Span, Text},
    token::Page,
};

/// A parsed invocation of the settings command.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Action {
    /// Open the settings panel.
    Panel,
    Help,
    Run(SettingsCommand),
}

impl<S: Schema> Panel<S> {
    /// The text form: no arguments open the panel, `help` lists the
    /// subcommands, the others (`list`, `set`, ...) change the settings.
    pub async fn command_text(
        &self,
        args: &str,
        viewer: &Viewer<'_>,
    ) -> Result<CommandReply<S>, PanelError> {
        let usage = usage(&self.command);
        let reply = |text: Text| CommandReply::Text { text, change: None };
        match parse_action(args) {
            Ok(Action::Panel) => Ok(CommandReply::Panel(self.render(&Page::Home, viewer).await)),
            Ok(Action::Help) => Ok(reply(Text::plain(usage))),
            Ok(Action::Run(run)) => {
                match command::execute(&self.store, run, viewer.actor.clone()).await {
                    Ok(outcome) => Ok(CommandReply::Text {
                        text: render(&outcome, &self.command),
                        change: into_change(outcome),
                    }),
                    // Infrastructure failures go to the host's error handler.
                    Err(error @ SettingsError::Storage(_)) => Err(PanelError::Storage(error)),
                    Err(error) => Ok(reply(Text::plain(format!("❌ {error}")))),
                }
            }
            Err(problem) => Ok(reply(Text::plain(format!("❌ {problem}\n\n{usage}")))),
        }
    }
}

fn into_change<S: Schema>(outcome: Outcome<S>) -> Option<Change<S>> {
    match outcome {
        Outcome::Changed { change, .. } | Outcome::Reloaded { change, .. } => Some(change),
        _ => None,
    }
}

/// The help of the settings command, `/<command>`.
pub(crate) fn usage(command: &str) -> String {
    format!(
        "/{command} — open the settings panel
/{command} list — show every runtime setting
/{command} get <key>
/{command} set <key> <value>
/{command} unset <key> — go back to the config file's value
/{command} add <key> <item> — append to a list
/{command} remove <key> <item> — remove from a list
/{command} reload — re-read the config file and the stored settings

Values are JSON (42, [1, 2], \"text\", {{}}) or plain text.
Map settings also take per-entry keys, e.g. <map>.<entry>."
    )
}

pub(crate) fn parse_action(args: &str) -> Result<Action, String> {
    let (verb, rest) = split_word(args);
    let (key, value) = split_word(rest);
    let key = key.to_string();
    let value = value.to_string();

    let require_nothing = |command: SettingsCommand| {
        if rest.is_empty() {
            Ok(Action::Run(command))
        } else {
            Err(format!("`{verb}` takes no arguments"))
        }
    };
    let require_key = |command: fn(String) -> SettingsCommand| {
        if key.is_empty() {
            Err(format!("`{verb}` needs a key"))
        } else if !value.is_empty() {
            Err(format!("`{verb}` takes only a key"))
        } else {
            Ok(Action::Run(command(key.clone())))
        }
    };
    let require_value = |command: fn(String, String) -> SettingsCommand| {
        if key.is_empty() || value.is_empty() {
            Err(format!("`{verb}` needs a key and a value"))
        } else {
            Ok(Action::Run(command(key.clone(), value.clone())))
        }
    };

    match verb {
        "" => Ok(Action::Panel),
        "list" => require_nothing(SettingsCommand::List),
        "reload" => require_nothing(SettingsCommand::Reload),
        "help" => Ok(Action::Help),
        "get" => require_key(SettingsCommand::Get),
        "unset" => require_key(SettingsCommand::Unset),
        "set" => require_value(SettingsCommand::Set),
        "add" => require_value(SettingsCommand::Add),
        "remove" => require_value(SettingsCommand::Remove),
        _ => Err(format!("unknown subcommand `{}`", args.trim())),
    }
}

/// Splits off the first whitespace-separated word; the rest is trimmed.
fn split_word(text: &str) -> (&str, &str) {
    let text = text.trim();
    match text.split_once(char::is_whitespace) {
        Some((word, rest)) => (word, rest.trim()),
        None => (text, ""),
    }
}

/// Renders an outcome.
pub(crate) fn render<S: Schema>(outcome: &Outcome<S>, command: &str) -> Text {
    let mut text = Text::new();
    match outcome {
        Outcome::Listing(listing) => return render_listing(listing, command),
        Outcome::Value(entry) => return render_entry(entry),
        Outcome::Changed {
            key,
            previous,
            current,
            lints,
            ..
        } => {
            text.push_plain("✅ Updated ")
                .push(Span::code(key))
                .push_plain("\nwas: ")
                .push(Span::code(command::render_value(previous.as_ref())))
                .push_plain("\nnow: ")
                .append(value_line(current));
            for lint in lints {
                text.push_plain(format!("\n⚠️ {lint}"));
            }
        }
        Outcome::NotOverridden(key) => {
            text.push_plain("ℹ️ ")
                .push(Span::code(key))
                .push_plain(" is not overridden");
        }
        Outcome::Reloaded {
            overrides, ignored, ..
        } => {
            text.push_plain(format!(
                "✅ Reloaded the configuration: {overrides} stored setting(s) applied, {ignored} \
                 ignored"
            ));
        }
    }
    text
}

fn render_listing(listing: &Listing, command: &str) -> Text {
    let mut text = Text(vec![Span::bold("Runtime settings")]);
    for entry in &listing.settings {
        text.push_plain("\n\n").append(render_entry(entry));
    }

    if !listing.entries.is_empty() {
        text.push_plain("\n\n")
            .push(Span::bold("Per-entry overrides"));
        for entry in &listing.entries {
            text.push_plain("\n").append(value_line(entry));
        }
    }

    if !listing.ignored.is_empty() {
        text.push_plain("\n\n").push(Span::bold(format!(
            "Ignored stored values (/{command} unset them)"
        )));
        for (key, reason) in &listing.ignored {
            text.push_plain("\n")
                .push(Span::code(key))
                .push_plain(format!(": {reason}"));
        }
    }

    if !listing.parked.is_empty() {
        text.push_plain("\n\n")
            .push(Span::bold("Waiting for their section"));
        for (key, value) in &listing.parked {
            text.push_plain("\n")
                .push(Span::code(key))
                .push_plain(" = ")
                .push(Span::code(value.to_string()));
        }
    }

    text.push_plain(format!("\n\nSend /{command} help for the syntax."));
    text
}

fn render_entry(entry: &ValueEntry) -> Text {
    let mut text = value_line(entry);
    if let Some(description) = &entry.description {
        text.push_plain("\n").push(Span::italic(&**description));
    }
    text
}

/// `<key> = <value> (<source>)`
fn value_line(entry: &ValueEntry) -> Text {
    let mut text = Text(vec![Span::code(&entry.key)]);
    text.push_plain(" = ")
        .push(Span::code(entry.rendered_value()))
        .push_plain(format!(" ({})", entry.source));
    text
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::testing::panel;

    #[test]
    fn parses_actions() {
        let run = Action::Run;
        let cases = [
            ("", Action::Panel),
            ("  list ", run(SettingsCommand::List)),
            ("reload", run(SettingsCommand::Reload)),
            ("help", Action::Help),
            (
                "get logging.filter",
                run(SettingsCommand::Get("logging.filter".into())),
            ),
            ("unset a.b", run(SettingsCommand::Unset("a.b".into()))),
            (
                "set telegram.sudo_users_id [1, 2]",
                run(SettingsCommand::Set(
                    "telegram.sudo_users_id".into(),
                    "[1, 2]".into(),
                )),
            ),
            (
                "add k  7",
                run(SettingsCommand::Add("k".into(), "7".into())),
            ),
            (
                "remove k 7",
                run(SettingsCommand::Remove("k".into(), "7".into())),
            ),
        ];
        for (args, expected) in cases {
            assert_eq!(parse_action(args), Ok(expected), "{args:?}");
        }
    }

    #[test]
    fn rejects_malformed_actions() {
        for args in [
            "get",
            "get a b",
            "set a",
            "add",
            "list extra",
            "reload now",
            "frobnicate",
        ] {
            assert!(parse_action(args).is_err(), "{args:?}");
        }
    }

    #[tokio::test]
    async fn renders_changes_and_listings() {
        let panel = panel().await;
        let store = panel.store();

        let outcome = command::execute(
            store,
            SettingsCommand::Set("admins".into(), "[5]".into()),
            Some("1".into()),
        )
        .await
        .unwrap();
        let text = render(&outcome, "config").to_plain();
        assert!(text.starts_with("✅"), "{text}");
        assert!(text.contains("[5]") && text.contains("(stored)"), "{text}");

        store.set("access.lamp", json!([3]), None).await.unwrap();
        let outcome = command::execute(store, SettingsCommand::List, None)
            .await
            .unwrap();
        let text = render(&outcome, "config").to_plain();
        assert!(text.contains("features.lamp.presets"), "{text}");
        assert!(text.contains("Per-entry overrides"), "{text}");
        assert!(text.contains("access.lamp"), "{text}");
        assert!(text.contains("Send /config help"), "{text}");
        assert!(text.contains("(default)"), "{text}");
    }
}
