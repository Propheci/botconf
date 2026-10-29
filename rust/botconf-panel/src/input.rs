//! Questions the panel asks, and how their answers (typed, or picked with the
//! front end's pickers) are read.

use serde_json::Value;

use botconf::{Kind, Schema, Snapshot, keys::is_entry_name, parse_value};

use crate::{
    Ask, Domain, Input, Panel, Picker, Question, Viewer,
    edit::Edit,
    screens::{btn, list, plain},
    setting::Setting,
    text::{Span, Text},
    token::{Action, AskKind, Page},
};

/// In bytes: entry names are kept short, to read well on buttons.
const MAX_ENTRY_NAME_LEN: usize = 32;

/// How many suggested references a question offers at most.
const MAX_CANDIDATES: usize = 8;

/// The domain used for references of a domain the front end did not
/// register: ids are typed in.
const UNKNOWN_DOMAIN: Domain = Domain::new("", no_id);

fn no_id(_: &str) -> Option<Value> {
    None
}

impl<S: Schema> Panel<S> {
    /// The question asking for `ask` about `setting`.
    pub(crate) async fn question(
        &self,
        snapshot: &Snapshot<S>,
        setting: &Setting<'_>,
        question: Question,
        viewer: &Viewer<'_>,
    ) -> Ask {
        let title = setting.title();
        let current = setting.value(snapshot).map(|value| plain(&value));
        let now = |text: &mut Text| {
            if let Some(value) = &current {
                text.push_plain("\n\nNow (tap to copy): ")
                    .push(Span::code(value));
            }
        };

        let mut text = Text::new();
        let mut picker = None;
        match (question.ask, setting.kind) {
            (ask @ (AskKind::Value | AskKind::Items), Kind::Ref { domain, .. }) => {
                let known = self.domain_of(domain);
                let described = known.copied().unwrap_or(UNKNOWN_DOMAIN);
                let many = ask == AskKind::Items;
                let pick = described.pick.filter(|_| known.is_some() && viewer.pickers);
                let verb = match pick {
                    Some((one, several)) => format!(
                        "Pick {} with the buttons below, or send",
                        if many { several } else { one }
                    ),
                    None => "Send".to_string(),
                };
                if pick.is_some() {
                    picker = Some(Picker {
                        domain: domain.to_string(),
                        many,
                    });
                }
                if many {
                    text.push_plain(format!("{verb} {} to add to ", described.many))
                        .push(Span::bold(&title))
                        .push_plain(".");
                } else {
                    text.push_plain(format!("{verb} {} for ", described.one))
                        .push(Span::bold(&title));
                    if let Some(example) = described.example {
                        text.push_plain(" (e.g. ")
                            .push(Span::code(example))
                            .push_plain(")");
                    }
                    text.push_plain(".");
                    now(&mut text);
                }
            }
            (AskKind::Value, Kind::Number { min, max, unit, .. }) => {
                text.push_plain("Send the new ")
                    .push(Span::bold(&title))
                    .push_plain(format!(", from {min}{unit} to {max}{unit}."));
                now(&mut text);
            }
            (AskKind::Value, Kind::Json) => {
                text.push_plain("Send the new ")
                    .push(Span::bold(&title))
                    .push_plain(", as JSON.");
                now(&mut text);
            }
            (AskKind::Value, _) => {
                text.push_plain("Send the new ")
                    .push(Span::bold(&title))
                    .push_plain(".");
                now(&mut text);
            }
            (AskKind::Items, _) => {
                text.push_plain("Send the items to add to ")
                    .push(Span::bold(&title))
                    .push_plain(".");
            }
            (AskKind::Entry, kind) if entry_kind(kind).empty_value().is_some() => {
                text.push_plain("Send a name for the new ")
                    .push(Span::bold(&title))
                    .push_plain(" entry.");
            }
            (AskKind::Entry, _) => {
                text.push_plain("Send the name of the new ")
                    .push(Span::bold(&title))
                    .push_plain(" entry, then its value.\n")
                    .push(Span::italic(&*setting.setting.description));
            }
        }

        let rows = match (question.ask, setting.kind) {
            (AskKind::Items, Kind::Ref { domain, .. }) => {
                self.candidate_rows(snapshot, setting, &question, domain, viewer)
                    .await
            }
            _ => Vec::new(),
        };

        Ask {
            question,
            title,
            text,
            picker,
            current,
            rows,
        }
    }

    /// Known references worth adding, as buttons.
    async fn candidate_rows(
        &self,
        snapshot: &Snapshot<S>,
        setting: &Setting<'_>,
        question: &Question,
        domain: &str,
        viewer: &Viewer<'_>,
    ) -> Vec<Vec<crate::ButtonView>> {
        let Some(names) = viewer.names else {
            return Vec::new();
        };
        let limit = viewer
            .limits
            .max_rows
            .unwrap_or(MAX_CANDIDATES)
            .clamp(1, MAX_CANDIDATES);
        let listed = list(setting.value(snapshot).as_ref());
        let rows: Vec<_> = names
            .candidates(domain, "", limit + listed.len())
            .await
            .into_iter()
            .filter(|(id, _)| !listed.contains(id))
            .take(limit)
            .map(|(id, name)| {
                vec![btn(
                    format!("➕ {}", name.short),
                    Action::Add(question.target.clone(), id),
                )]
            })
            .collect();
        self.issue_rows(rows, viewer.limits).await
    }

    /// The edit an answer asks for, and the page to show afterwards. The
    /// error says why the answer makes no sense; the user can try again.
    pub(crate) async fn read(
        &self,
        snapshot: &Snapshot<S>,
        setting: &Setting<'_>,
        question: &Question,
        input: Input,
        viewer: &Viewer<'_>,
    ) -> Result<(Edit, Page), String> {
        let stay = Page::Setting(question.target.clone());
        let answer = match question.ask {
            AskKind::Value => {
                let value = self.value(setting.kind, input, viewer).await?;
                (setting.set(snapshot, Some(value)), stay)
            }
            AskKind::Items => {
                let items = self.items(setting.kind, input, viewer).await?;
                (setting.change_list(snapshot, items, None), stay)
            }
            AskKind::Entry => {
                let Input::Text(text) = input else {
                    return Err("send the name as text".into());
                };
                let text = text.trim();
                let (name, rest) = text.split_once(char::is_whitespace).unwrap_or((text, ""));
                if !is_entry_name(name)
                    || name.contains([':', '#'])
                    || name.len() > MAX_ENTRY_NAME_LEN
                {
                    return Err(format!(
                        "`{name}` is not a valid name: use up to {MAX_ENTRY_NAME_LEN} bytes (a \
                         character or more each), without dots, colons or #"
                    ));
                }
                let kind = entry_kind(setting.kind);
                let value = match (rest.trim(), kind.empty_value()) {
                    // Forms are then filled in field by field.
                    (_, Some(empty)) if matches!(kind, Kind::Form(_)) => empty,
                    ("", Some(empty)) => empty,
                    ("", None) => return Err("send the name, a space, then the value".into()),
                    (rest, _) => typed(kind, rest)?,
                };
                (
                    Edit::Set(format!("{}.{name}", setting.key()), value),
                    Page::Setting(question.target.entry(name)),
                )
            }
        };
        Ok(answer)
    }

    /// A whole value.
    async fn value(&self, kind: &Kind, input: Input, viewer: &Viewer<'_>) -> Result<Value, String> {
        if let Kind::Ref { domain, .. } = kind {
            return match input {
                Input::Picked {
                    domain: picked,
                    ids,
                } => picked_ids(domain, &picked, ids)?
                    .into_iter()
                    .next()
                    .ok_or_else(|| "pick one".to_string()),
                Input::Text(text) => self.reference(domain, text.trim(), viewer).await,
                Input::Cancel => Err("cancelled".into()),
            };
        }

        match input {
            Input::Text(text) if !text.trim().is_empty() => typed(kind, text.trim()),
            _ => Err("send the value as text".into()),
        }
    }

    /// Items to add to a list.
    async fn items(
        &self,
        kind: &Kind,
        input: Input,
        viewer: &Viewer<'_>,
    ) -> Result<Vec<Value>, String> {
        let text = match (input, kind) {
            (
                Input::Picked {
                    domain: picked,
                    ids,
                },
                Kind::Ref { domain, .. },
            ) => {
                return picked_ids(domain, &picked, ids);
            }
            (Input::Text(text), _) => text,
            _ => return Err("send the items as text".into()),
        };

        let words: Vec<_> = text
            .split(|c: char| c.is_whitespace() || c == ',')
            .filter(|word| !word.is_empty())
            .collect();
        if words.is_empty() {
            return Err("send at least one item".into());
        }

        let mut items = Vec::new();
        for word in words {
            let item = match kind {
                Kind::Ref { domain, .. } => self.reference(domain, word, viewer).await?,
                _ => parse_value(word),
            };
            items.push(item);
        }
        Ok(items)
    }

    /// A typed reference of `domain`: an id as the front end's domain reads
    /// it, else what the names resolve it to, else the one suggestion that
    /// matches it. Ids of domains the front end did not register are read as
    /// JSON or as a string.
    async fn reference(
        &self,
        domain: &str,
        word: &str,
        viewer: &Viewer<'_>,
    ) -> Result<Value, String> {
        if word.is_empty() {
            return Err("send the id as text".into());
        }
        let known = self.domain_of(domain);
        if let Some(id) = known.and_then(|known| (known.parse)(word)) {
            return Ok(id);
        }
        if let Some(names) = viewer.names {
            if let Some(id) = names.resolve(domain, word).await {
                return Ok(id);
            }
            let mut matches = names.candidates(domain, word, 2).await;
            if matches.len() == 1 {
                return Ok(matches.remove(0).0);
            }
        }
        match known {
            Some(known) => Err((known.unknown)(word)),
            None => Ok(parse_value(word)),
        }
    }
}

/// Ids picked with the picker of `picked`, for a setting of `domain`.
fn picked_ids(domain: &str, picked: &str, ids: Vec<Value>) -> Result<Vec<Value>, String> {
    if picked != domain {
        return Err("that is not what this setting holds".into());
    }
    if ids.is_empty() {
        return Err("pick at least one".into());
    }
    Ok(ids)
}

/// The kind of the entries of a map.
fn entry_kind(kind: &Kind) -> &Kind {
    match kind {
        Kind::Map { value, .. } => value,
        kind => kind,
    }
}

/// Text as a value of `kind`: taken as typed for text, JSON otherwise.
fn typed(kind: &Kind, text: &str) -> Result<Value, String> {
    match kind {
        Kind::Text { .. } | Kind::OneOf { .. } => Ok(Value::String(text.to_string())),
        Kind::Number { min, max, .. } => {
            let number = text.trim_end_matches('%').trim();
            match number.parse::<i64>() {
                Ok(number) if (*min..=*max).contains(&number) => Ok(Value::from(number)),
                _ => Err(format!("`{text}` is not a number from {min} to {max}")),
            }
        }
        _ => Ok(parse_value(text)),
    }
}
