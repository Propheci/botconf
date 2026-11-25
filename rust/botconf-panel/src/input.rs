//! Questions the panel asks, and how their answers (typed, or picked with the
//! front end's pickers) are read.

use serde_json::Value;

use botconf::{Kind, Schema, Snapshot, keys::is_entry_name, parse_value};

use crate::{
    Ask, Domain, Input, Panel, Picker, Question, SettingQuestion, Viewer,
    edit::Edit,
    screens::{btn, in_draft, list, plain},
    setting::Setting,
    text::{Span, Text},
    token::{Action, AskKind, Browse, Page},
};

/// In bytes: entry names are kept short, to read well on buttons.
const MAX_ENTRY_NAME_LEN: usize = 32;

/// How many suggested references a question offers at most.
const MAX_CANDIDATES: usize = 8;

/// The domain used for references of a domain the front end did not
/// register: ids are typed in.
const UNKNOWN_DOMAIN: Domain = Domain::new("", no_id);

/// The page `question` was asked from, where ‹ Back and cancelling lead: the
/// draft it was asked on, else the setting it is about.
pub(crate) fn asked_from(question: &SettingQuestion) -> Page {
    match (&question.draft, question.target.parent()) {
        (Some(draft), Some(form)) => Page::Draft(form, draft.clone()),
        _ => Page::Setting(question.target.clone()),
    }
}

fn no_id(_: &str) -> Option<Value> {
    None
}

impl<S: Schema> Panel<S> {
    /// The question asking for `ask` about `setting`, showing the page of
    /// its candidates `browse` names (for a reference).
    pub(crate) async fn question(
        &self,
        snapshot: &Snapshot<S>,
        setting: &Setting<'_>,
        question: SettingQuestion,
        browse: &Browse,
        viewer: &Viewer<'_>,
    ) -> Ask {
        let title = setting.title();
        let current = match &question.draft {
            Some(draft) => setting.value_in(draft),
            None => setting.value(snapshot),
        }
        .map(|value| plain(&value));
        let now = |text: &mut Text| {
            if let Some(value) = &current {
                text.push_plain("\n\nNow (tap to copy): ")
                    .push(Span::code(value));
            }
        };

        let rows = match (question.ask, setting.kind) {
            (AskKind::Value | AskKind::Items, Kind::Ref { domain, .. }) => {
                self.candidate_rows(snapshot, setting, &question, domain, browse, viewer)
                    .await
            }
            _ => Vec::new(),
        };

        let mut text = Text::new();
        let mut picker = None;
        match (question.ask, setting.kind) {
            (ask @ (AskKind::Value | AskKind::Items), Kind::Ref { domain, .. }) => {
                let known = self.domain_of(domain);
                let described = known.copied().unwrap_or(UNKNOWN_DOMAIN);
                let many = ask == AskKind::Items;
                if !browse.query.is_empty() {
                    text.push_plain(format!("{} matching ", capitalized(described.noun.1)))
                        .push(Span::bold(format!("“{}”", browse.query)))
                        .push_plain(":\n\n");
                }
                let hint = |text: &mut Text| {
                    if !rows.is_empty() {
                        text.push_plain("\nTap one below, or send part of a name to search.");
                    }
                };
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
                    hint(&mut text);
                } else {
                    text.push_plain(format!("{verb} {} for ", described.one))
                        .push(Span::bold(&title));
                    if let Some(example) = described.example {
                        text.push_plain(" (e.g. ")
                            .push(Span::code(example))
                            .push_plain(")");
                    }
                    text.push_plain(".");
                    hint(&mut text);
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

        let back = self
            .issue_rows(
                vec![vec![btn("‹ Back", Action::open(asked_from(&question)))]],
                viewer.limits,
            )
            .await
            .into_iter()
            .flatten()
            .next();

        Ask {
            question: Question::Setting(question),
            title,
            text,
            picker,
            current,
            rows,
            back,
        }
    }

    /// One page of the known references, as buttons: those already chosen
    /// marked (✅ removes one from a list), the others to add or pick, then
    /// ◀ / ▶ to the neighbouring pages.
    async fn candidate_rows(
        &self,
        snapshot: &Snapshot<S>,
        setting: &Setting<'_>,
        question: &SettingQuestion,
        domain: &str,
        browse: &Browse,
        viewer: &Viewer<'_>,
    ) -> Vec<Vec<crate::ButtonView>> {
        let Some(names) = viewer.names else {
            return Vec::new();
        };
        let size = page_size(viewer.limits);
        let mut found = names
            .candidates_page(domain, &browse.query, browse.offset, size + 1)
            .await;
        let more = found.len() > size;
        found.truncate(size);

        let many = question.ask == AskKind::Items;
        let current = match &question.draft {
            Some(draft) => setting.value_in(draft),
            None => setting.value(snapshot),
        };
        let chosen = if many {
            list(current.as_ref())
        } else {
            current.into_iter().collect()
        };
        let target = &question.target;
        let wrap = |action| match &question.draft {
            Some(draft) => in_draft(draft, action),
            None => action,
        };

        let mut rows: Vec<_> = found
            .into_iter()
            .map(|(id, name)| {
                let (mark, action) = match (many, chosen.contains(&id)) {
                    (true, true) => ("✅", Action::Remove(target.clone(), id)),
                    (true, false) => ("➕", Action::Add(target.clone(), id)),
                    (false, true) => ("🔘", Action::PickRef(target.clone(), id)),
                    (false, false) => ("⚪", Action::PickRef(target.clone(), id)),
                };
                vec![btn(format!("{mark} {}", name.short), wrap(action))]
            })
            .collect();
        let page = |offset| {
            let query = browse.query.clone();
            wrap(Action::Find(
                target.clone(),
                question.ask,
                Browse { query, offset },
            ))
        };
        let mut nav = Vec::new();
        if browse.offset > 0 {
            nav.push(btn("◀", page(browse.offset.saturating_sub(size))));
        }
        if more {
            nav.push(btn("▶", page(browse.offset + size)));
        }
        if !nav.is_empty() {
            rows.push(nav);
        }
        self.issue_rows(rows, viewer.limits).await
    }

    /// The edit an answer asks for, and the page to show afterwards. The
    /// error says why the answer was not taken: the user can try again, or
    /// pick from what their text matches.
    pub(crate) async fn read(
        &self,
        snapshot: &Snapshot<S>,
        setting: &Setting<'_>,
        question: &SettingQuestion,
        input: Input,
        viewer: &Viewer<'_>,
    ) -> Result<(Edit, Page), Unread> {
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
                    return Err(Unread::Rejected(format!(
                        "`{name}` is not a valid name: use up to {MAX_ENTRY_NAME_LEN} bytes (a \
                         character or more each), without dots, colons or #"
                    )));
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

    /// The draft `draft` with the answer to `question` (about one of its
    /// fields) in it. The error says why the answer was not taken.
    pub(crate) async fn read_into_draft(
        &self,
        setting: &Setting<'_>,
        question: &SettingQuestion,
        draft: &Value,
        input: Input,
        viewer: &Viewer<'_>,
    ) -> Result<Value, Unread> {
        let field = match question.ask {
            AskKind::Value => self.value(setting.kind, input, viewer).await?,
            AskKind::Items => {
                let mut items = list(setting.value_in(draft).as_ref());
                for item in self.items(setting.kind, input, viewer).await? {
                    if !items.contains(&item) {
                        items.push(item);
                    }
                }
                Value::Array(items)
            }
            AskKind::Entry => return Err("a draft's field has no entries".into()),
        };
        let form = draft.as_object().cloned().unwrap_or_default();
        Ok(Value::Object(setting.set_in(form, Some(field))))
    }

    /// A whole value.
    async fn value(&self, kind: &Kind, input: Input, viewer: &Viewer<'_>) -> Result<Value, Unread> {
        if let Kind::Ref { domain, .. } = kind {
            return match input {
                Input::Picked {
                    domain: picked,
                    ids,
                } => Ok(picked_ids(domain, &picked, ids)?
                    .into_iter()
                    .next()
                    .ok_or_else(|| "pick one".to_string())?),
                Input::Text(text) => Ok(self
                    .references(domain, &text, false, viewer)
                    .await?
                    .remove(0)),
                Input::Cancel => Err("cancelled".into()),
            };
        }

        match input {
            Input::Text(text) if !text.trim().is_empty() => Ok(typed(kind, text.trim())?),
            _ => Err("send the value as text".into()),
        }
    }

    /// Items to add to a list.
    async fn items(
        &self,
        kind: &Kind,
        input: Input,
        viewer: &Viewer<'_>,
    ) -> Result<Vec<Value>, Unread> {
        let text = match (input, kind) {
            (
                Input::Picked {
                    domain: picked,
                    ids,
                },
                Kind::Ref { domain, .. },
            ) => {
                return Ok(picked_ids(domain, &picked, ids)?);
            }
            (Input::Text(text), Kind::Ref { domain, .. }) => {
                return self.references(domain, &text, true, viewer).await;
            }
            (Input::Text(text), _) => text,
            _ => return Err("send the items as text".into()),
        };
        let words = words(&text);
        if words.is_empty() {
            return Err("send at least one item".into());
        }
        Ok(words.into_iter().map(parse_value).collect())
    }

    /// The references of `domain` typed in `text`: one for a single value,
    /// or several (`many`), split on spaces and commas. In this order:
    ///
    /// 1. each word as one reference (see [`Self::found`]);
    /// 2. the whole text as a name, which may hold spaces;
    /// 3. a search, when the text, or a word of it, matches several candidates:
    ///    the user picks from those;
    /// 4. else the domain's "unknown" error. For a domain the front end did not
    ///    register, the words are taken as typed (JSON, or strings).
    async fn references(
        &self,
        domain: &str,
        text: &str,
        many: bool,
        viewer: &Viewer<'_>,
    ) -> Result<Vec<Value>, Unread> {
        let text = text.trim();
        let words = if many { words(text) } else { vec![text] };
        if words.iter().all(|word| word.is_empty()) {
            return Err(if many {
                "send at least one item"
            } else {
                "send the id as text"
            }
            .into());
        }

        let mut found = Vec::with_capacity(words.len());
        for word in &words {
            found.push(self.found(domain, word, true, viewer).await);
        }
        if found.iter().all(|found| matches!(found, Found::One(_))) {
            return Ok(found
                .into_iter()
                .filter_map(|found| match found {
                    Found::One(id) => Some(id),
                    _ => None,
                })
                .collect());
        }
        if words.len() > 1 {
            match self.found(domain, text, false, viewer).await {
                Found::One(id) => return Ok(vec![id]),
                Found::Several => return Err(Unread::Search(text.to_string())),
                Found::Nothing => {}
            }
        }
        if let Some(position) = found
            .iter()
            .position(|found| matches!(found, Found::Several))
        {
            return Err(Unread::Search(words[position].to_string()));
        }

        let known = self.domain_of(domain);
        let mut ids = Vec::with_capacity(words.len());
        for (word, found) in words.into_iter().zip(found) {
            match (found, known) {
                (Found::One(id), _) => ids.push(id),
                (_, Some(known)) => return Err(Unread::Rejected((known.unknown)(word))),
                (_, None) => ids.push(parse_value(word)),
            }
        }
        Ok(ids)
    }

    /// What `word` names in `domain`: an id as the front end's domain reads
    /// it (when `as_id`), else what the names resolve it to, else the one
    /// candidate that matches it.
    async fn found(&self, domain: &str, word: &str, as_id: bool, viewer: &Viewer<'_>) -> Found {
        let id = as_id
            .then(|| self.domain_of(domain))
            .flatten()
            .and_then(|known| (known.parse)(word));
        if let Some(id) = id {
            return Found::One(id);
        }
        let Some(names) = viewer.names else {
            return Found::Nothing;
        };
        if let Some(id) = names.resolve(domain, word).await {
            return Found::One(id);
        }
        let mut matches = names.candidates(domain, word, 2).await;
        match matches.len() {
            0 => Found::Nothing,
            1 => Found::One(matches.remove(0).0),
            _ => Found::Several,
        }
    }
}

/// Why an answer was not taken.
#[derive(Debug)]
pub(crate) enum Unread {
    /// It makes no sense; the user can try again.
    Rejected(String),
    /// It matches several references: ask again, showing those.
    Search(String),
}

impl From<String> for Unread {
    fn from(problem: String) -> Self {
        Self::Rejected(problem)
    }
}

impl From<&str> for Unread {
    fn from(problem: &str) -> Self {
        Self::Rejected(problem.to_string())
    }
}

/// What a typed word names.
enum Found {
    One(Value),
    Several,
    Nothing,
}

/// The words of a typed list: split on spaces and commas.
fn words(text: &str) -> Vec<&str> {
    text.split(|c: char| c.is_whitespace() || c == ',')
        .filter(|word| !word.is_empty())
        .collect()
}

/// How many candidates a question shows at once: [`MAX_CANDIDATES`], or
/// fewer when the front end's rows are limited (◀ / ▶ and ‹ Back need two).
fn page_size(limits: &crate::Limits) -> usize {
    limits
        .max_rows
        .map_or(MAX_CANDIDATES, |rows| rows.saturating_sub(2))
        .clamp(1, MAX_CANDIDATES)
}

/// `text` with its first letter in upper case.
fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
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
