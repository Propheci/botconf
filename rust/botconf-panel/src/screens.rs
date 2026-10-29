//! The screens of the settings panel: the home screen, one per section, and
//! one per setting, with an editor fitting its [`Kind`].

use serde_json::{Map, Value};

use botconf::{Catalog, Choice, Choices, Kind, Schema, Snapshot, Source, keys::is_below};

use crate::{
    ButtonView, Limits, Name, Panel, Screen, Viewer,
    setting::Setting,
    text::{Span, Text, truncate},
    token::{Action, AskKind, Page, Target},
};

/// Longer values are cut short on the buttons.
pub(crate) const MAX_LABEL_CHARS: usize = 32;
const MAX_VALUE_CHARS: usize = 3000;

/// A button before it gets its token.
#[derive(Clone, Debug)]
pub(crate) struct Btn {
    pub(crate) label: String,
    pub(crate) action: Action,
}

pub(crate) fn btn(label: impl Into<String>, action: Action) -> Btn {
    Btn {
        label: label.into(),
        action,
    }
}

/// A screen before its buttons get their tokens: content rows, then one row
/// of the screen's own navigation (kept on every part of a long screen).
pub(crate) struct Building {
    pub(crate) text: Text,
    pub(crate) rows: Vec<Vec<Btn>>,
    pub(crate) nav: Vec<Btn>,
}

impl<S: Schema> Panel<S> {
    /// Renders `page`.
    pub async fn render(&self, page: &Page, viewer: &Viewer<'_>) -> Screen {
        self.render_part(page, 0, None, viewer).await
    }

    /// Renders a part of `page`, below `notice`.
    pub(crate) async fn render_part(
        &self,
        page: &Page,
        part: usize,
        notice: Option<&str>,
        viewer: &Viewer<'_>,
    ) -> Screen {
        self.render_in(page, None, part, notice, viewer).await
    }

    /// Renders a part of `page`, below `notice`. With `draft`, `page` is the
    /// page of a field of that draft: it shows the draft's value, and its
    /// buttons change the draft instead of the stored settings.
    pub(crate) async fn render_in(
        &self,
        page: &Page,
        draft: Option<&Value>,
        part: usize,
        notice: Option<&str>,
        viewer: &Viewer<'_>,
    ) -> Screen {
        let snapshot = self.store.current();
        let catalog = self.store.catalog();
        let gone = async || {
            let mut home = self.home(&snapshot, &catalog).await;
            home.text = home.text.with_notice("This setting no longer exists.");
            home
        };
        let mut building = match page {
            Page::Home => self.home(&snapshot, &catalog).await,
            Page::Section(id) => self.section(&snapshot, &catalog, id).await,
            Page::Setting(target) => match Setting::resolve(&catalog, target) {
                Some(setting) if draft.is_none() || setting.field.is_some() => {
                    self.setting(&snapshot, &catalog, target, setting, draft, viewer)
                        .await
                }
                _ => gone().await,
            },
            Page::Draft(target, value) => match Setting::resolve(&catalog, target) {
                Some(setting)
                    if setting.field.is_none() && matches!(setting.kind, Kind::Form(_)) =>
                {
                    self.draft(&snapshot, &catalog, target, setting, value)
                        .await
                }
                _ => gone().await,
            },
        };
        if let Some(notice) = notice {
            building.text = building.text.with_notice(notice);
        }
        self.finish(building, page, part, viewer.limits, draft)
            .await
    }

    /// Cuts a long screen into parts, splits long rows, and gives every
    /// button its token. With `draft`, every button that is not already a
    /// draft's page acts on that draft.
    async fn finish(
        &self,
        building: Building,
        page: &Page,
        part: usize,
        limits: &Limits,
        draft: Option<&Value>,
    ) -> Screen {
        let mut rows = paged(building.rows, building.nav, limits, part, |part| {
            Action::Open(page.clone(), part)
        });
        if let Some(draft) = draft {
            for button in rows.iter_mut().flatten() {
                if !matches!(button.action, Action::Open(Page::Draft(..), _)) {
                    button.action = in_draft(draft, button.action.clone());
                }
            }
        }
        Screen {
            text: building.text,
            rows: self.issue_rows(rows, limits).await,
        }
    }

    /// The buttons with their tokens. A button whose token can't be issued
    /// is left out.
    pub(crate) async fn issue_rows(
        &self,
        rows: Vec<Vec<Btn>>,
        limits: &Limits,
    ) -> Vec<Vec<ButtonView>> {
        let now = self.now();
        let mut issued = Vec::with_capacity(rows.len());
        for row in rows {
            let mut buttons = Vec::with_capacity(row.len());
            for Btn { label, action } in row {
                match self.tokens.issue(&action, limits.token_bytes, now).await {
                    Ok(Some(token)) => buttons.push(ButtonView {
                        label: truncate(&label, MAX_LABEL_CHARS + 16),
                        token,
                    }),
                    Ok(None) => {}
                    Err(error) => {
                        tracing::error!(%error, "the panel's token store failed; a button is left out");
                    }
                }
            }
            if !buttons.is_empty() {
                issued.push(buttons);
            }
        }
        issued
    }

    async fn home(&self, snapshot: &Snapshot<S>, catalog: &Catalog) -> Building {
        let mut text = Text::plain("⚙️ ");
        text.push(Span::bold("Settings")).push_plain(
            "\nTap a setting to change it. ✏️ marks the ones changed from here, which take \
             precedence over the config file.",
        );

        let mut rows = self.setting_rows(snapshot, catalog, None);
        let mut sections: Vec<_> = self
            .store
            .sections()
            .iter()
            .filter(|section| !section.settings.runtime.is_empty())
            .map(|section| (section.id.to_string(), section.title.to_string()))
            .collect();
        let mut dynamic: Vec<_> = self
            .store
            .dynamic_sections()
            .await
            .into_iter()
            .filter(|section| !section.settings.is_empty())
            .map(|section| (section.id.clone(), section.title.clone()))
            .collect();
        dynamic.sort_by(|a, b| a.1.cmp(&b.1));
        sections.extend(dynamic);

        let buttons: Vec<_> = sections
            .into_iter()
            .map(|(id, title)| {
                let note = self
                    .note(snapshot, &id)
                    .map(|note| format!(" ({note})"))
                    .unwrap_or_default();
                btn(format!("{title}{note} ›"), Action::open(Page::Section(id)))
            })
            .collect();
        rows.extend(buttons.chunks(2).map(<[_]>::to_vec));

        Building {
            text,
            rows,
            nav: vec![
                btn("🔄 Reload", Action::Reload),
                btn("✖️ Close", Action::Close),
            ],
        }
    }

    async fn section(&self, snapshot: &Snapshot<S>, catalog: &Catalog, id: &str) -> Building {
        let described = match self
            .store
            .sections()
            .iter()
            .find(|section| section.id == id)
        {
            Some(section) => Some((section.title.to_string(), section.description.to_string())),
            None => self
                .store
                .dynamic_sections()
                .await
                .into_iter()
                .find(|section| section.id == id)
                .map(|section| (section.title.clone(), section.description.clone())),
        };
        let Some((title, description)) = described else {
            return self.home(snapshot, catalog).await;
        };

        let mut text = Text::new();
        text.push(Span::bold(title))
            .push_plain("\n")
            .push(Span::italic(description));
        if let Some(note) = self.note(snapshot, id) {
            text.push_plain(format!("\n\nNote: {note}."));
        }

        Building {
            text,
            rows: self.setting_rows(snapshot, catalog, Some(id)),
            nav: vec![btn("‹ Back", Action::open(Page::Home))],
        }
    }

    /// A button per setting of `section` (the top-level ones for `None`).
    fn setting_rows(
        &self,
        snapshot: &Snapshot<S>,
        catalog: &Catalog,
        section: Option<&str>,
    ) -> Vec<Vec<Btn>> {
        catalog
            .live()
            .filter(|(_, entry)| entry.section.as_deref() == section)
            .filter_map(|(position, entry)| {
                let target = Target::setting(position);
                let setting = Setting::resolve(catalog, &target)?;
                let label = format!(
                    "{}: {}{}",
                    entry.title,
                    self.summary(snapshot, &setting),
                    changed_marker(snapshot, &entry.key)
                );
                Some(vec![btn(label, Action::open(Page::Setting(target)))])
            })
            .collect()
    }

    async fn setting(
        &self,
        snapshot: &Snapshot<S>,
        catalog: &Catalog,
        target: &Target,
        setting: Setting<'_>,
        draft: Option<&Value>,
        viewer: &Viewer<'_>,
    ) -> Building {
        let key = setting.key();
        let value = match draft {
            Some(draft) => setting.value_in(draft),
            None => setting.value(snapshot),
        };

        let mut text = Text::new();
        text.push(Span::bold(setting.title()));
        if draft.is_some() {
            text.push_plain(format!(" · {DRAFT_MARK}"));
        }
        text.push_plain("\n")
            .push(Span::italic(&*setting.setting.description));
        if setting.field.is_none() {
            let source = match snapshot.source(&key) {
                Source::Stored => self.stored_text().to_string(),
                Source::File => "from the config file".to_string(),
                Source::Environment => "from the environment".to_string(),
                Source::Default => "the default".to_string(),
                Source::Other(name) => name,
            };
            text.push_plain("\n")
                .push(Span::code(&key))
                .push_plain(format!(" · {source}"));
        }

        let mut rows: Vec<Vec<Btn>> = Vec::new();
        let mut row = |buttons: Vec<Option<Btn>>| {
            let buttons: Vec<_> = buttons.into_iter().flatten().collect();
            if !buttons.is_empty() {
                rows.push(buttons);
            }
        };
        let change = || {
            Some(btn(
                "✏️ Change",
                Action::Ask(target.clone(), AskKind::Value),
            ))
        };
        let clear = |label: &str, optional: bool| {
            (optional && value.is_some()).then(|| btn(label, Action::Clear(target.clone())))
        };
        let choices = setting.kind.choices(snapshot);

        let details = match setting.kind {
            Kind::Json => {
                row(vec![change()]);
                match &value {
                    Some(value) => {
                        let pretty = serde_json::to_string_pretty(value).unwrap_or_default();
                        Text(vec![Span::code_block(truncate(&pretty, MAX_VALUE_CHARS))])
                    }
                    None => Text::plain("Not set."),
                }
            }

            Kind::Text { optional } => {
                row(vec![change(), clear("🗑 Clear", *optional)]);
                current_line(value.as_ref())
            }

            Kind::Ref {
                domain,
                many: false,
            } => {
                row(vec![change()]);
                match &value {
                    Some(id) => {
                        let name = name_of(viewer, domain, id).await;
                        let mut text = Text::plain("Now: ");
                        text.append(item_line(name.as_ref(), id));
                        text
                    }
                    None => Text::plain("Not set."),
                }
            }

            Kind::Number { optional, .. } | Kind::OneOf { optional, .. } => {
                let (custom, unit) = match setting.kind {
                    Kind::OneOf { custom, .. } => (*custom, ""),
                    Kind::Number { unit, .. } => (true, &**unit),
                    _ => (false, ""),
                };
                let fixed = is_fixed(setting.kind);
                let picked = |choice: &Choice| {
                    value.as_ref() == Some(&setting.kind.choice_value(&choice.value))
                };
                let buttons: Vec<_> = choices
                    .iter()
                    .enumerate()
                    .map(|(position, choice)| {
                        let mark = if picked(choice) { "🔘" } else { "⚪" };
                        let action = if fixed {
                            Action::Pick(target.clone(), position)
                        } else {
                            Action::PickValue(target.clone(), Value::String(choice.value.clone()))
                        };
                        btn(format!("{mark} {}", choice.label), action)
                    })
                    .collect();
                let per_row = if matches!(setting.kind, Kind::Number { .. }) {
                    3
                } else {
                    2
                };
                for chunk in buttons.chunks(per_row) {
                    row(chunk.iter().cloned().map(Some).collect());
                }
                row(vec![
                    clear("🚫 None", *optional),
                    custom.then(|| btn("✏️ Other…", Action::Ask(target.clone(), AskKind::Value))),
                ]);

                let mut details = match &value {
                    Some(value) => {
                        let label = match setting.kind {
                            Kind::Number { .. } => format!("{}{unit}", plain(value)),
                            _ => label_of(value, &choices),
                        };
                        let mut text = Text::plain("Now: ");
                        text.push(Span::bold(label));
                        text
                    }
                    None => Text::plain("Not set."),
                };
                if let Some(excluded) = excluded_fields(&setting) {
                    details
                        .push_plain("\n")
                        .push(Span::italic(format!("Setting it clears {excluded}.")));
                }
                details
            }

            Kind::SetOf {
                inverted,
                choices: kind_choices,
            } => {
                let listed = list(value.as_ref());
                let fixed = matches!(kind_choices, Choices::Fixed(_));
                let buttons: Vec<_> = choices
                    .iter()
                    .enumerate()
                    .map(|(position, choice)| {
                        let item = Value::String(choice.value.clone());
                        let on = listed.contains(&item) != *inverted;
                        let label = format!("{} {}", if on { "✅" } else { "⬜" }, choice.label);
                        let action = if fixed {
                            Action::Toggle(target.clone(), position)
                        } else {
                            Action::ToggleValue(target.clone(), item)
                        };
                        btn(label, action)
                    })
                    .collect();
                for pair in buttons.chunks(2) {
                    row(pair.iter().cloned().map(Some).collect());
                }
                Text::plain("Tap to turn on or off.")
            }

            Kind::Ref { domain, many: true } => {
                let mut lines = Text::new();
                for (index, item) in list(value.as_ref()).into_iter().enumerate() {
                    let name = name_of(viewer, domain, &item).await;
                    let short = name
                        .as_ref()
                        .map_or_else(|| plain(&item), |name| name.short.clone());
                    if index > 0 {
                        lines.push_plain("\n");
                    }
                    lines
                        .push_plain("• ")
                        .append(item_line(name.as_ref(), &item));
                    row(vec![Some(btn(
                        format!("❌ {short}"),
                        Action::Remove(target.clone(), item),
                    ))]);
                }
                row(vec![Some(btn(
                    "➕ Add",
                    Action::Ask(target.clone(), AskKind::Items),
                ))]);
                if lines.is_empty() {
                    Text::plain("None yet.")
                } else {
                    lines
                }
            }

            Kind::Form(form) => {
                let empty = Map::new();
                let object = value.as_ref().and_then(Value::as_object).unwrap_or(&empty);
                let mut lines = Text::new();
                for (position, field) in form.fields.iter().enumerate() {
                    if !field.applies(object) {
                        continue;
                    }
                    let field_target = target.field(position);
                    let Some(field_setting) = Setting::resolve(catalog, &field_target) else {
                        continue;
                    };
                    let summary = match field_setting.value(snapshot) {
                        Some(_) => self.summary(snapshot, &field_setting),
                        None => "—".to_string(),
                    };
                    if !lines.0.is_empty() {
                        lines.push_plain("\n");
                    }
                    lines
                        .push_plain("• ")
                        .push(Span::bold(&*field.title))
                        .push_plain(format!(": {summary}"));
                    row(vec![Some(btn(
                        format!("{}: {summary} ›", field.title),
                        Action::open(Page::Setting(field_target)),
                    ))]);
                }
                lines
            }

            Kind::Map { names, value: _ } => {
                let entries = match &value {
                    Some(Value::Object(entries)) => entries.clone(),
                    _ => Map::new(),
                };
                let mut lines = Text::new();
                for name in entries.keys() {
                    let entry_target = target.entry(name);
                    let Some(entry) = Setting::resolve(catalog, &entry_target) else {
                        continue;
                    };
                    let summary = self.summary(snapshot, &entry);
                    if !lines.0.is_empty() {
                        lines.push_plain("\n");
                    }
                    lines
                        .push_plain("• ")
                        .push(Span::bold(name))
                        .push_plain(format!(": {summary}"));
                    row(vec![Some(btn(
                        format!(
                            "{name}: {summary}{} ›",
                            changed_marker(snapshot, &entry.key())
                        ),
                        Action::open(Page::Setting(entry_target)),
                    ))]);
                }

                match names {
                    Some(names) => {
                        let fixed = matches!(names, Choices::Fixed(_));
                        let missing: Vec<_> = choices
                            .iter()
                            .enumerate()
                            .filter(|(_, choice)| !entries.contains_key(&choice.value))
                            .map(|(position, choice)| {
                                let action = if fixed {
                                    Action::Pick(target.clone(), position)
                                } else {
                                    Action::PickValue(
                                        target.clone(),
                                        Value::String(choice.value.clone()),
                                    )
                                };
                                btn(format!("➕ {}", choice.label), action)
                            })
                            .collect();
                        for pair in missing.chunks(2) {
                            row(pair.iter().cloned().map(Some).collect());
                        }
                    }
                    None => row(vec![Some(btn(
                        "➕ Add",
                        Action::Ask(target.clone(), AskKind::Entry),
                    ))]),
                }

                if lines.is_empty() {
                    Text::plain("No entries yet.")
                } else {
                    lines
                }
            }
        };
        text.push_plain("\n\n").append(details);

        if setting.field.is_none() {
            if setting.entry.is_some() {
                row(vec![Some(btn(
                    "🗑 Delete entry",
                    Action::Delete(target.clone()),
                ))]);
            }
            if is_overridden(snapshot, &key) {
                row(vec![Some(btn(
                    "↩️ Use the config file's value",
                    Action::Reset(target.clone()),
                ))]);
            }
        }

        let back = match (draft, target.parent()) {
            (Some(draft), Some(form)) => Page::Draft(form, draft.clone()),
            (_, parent) => back_from(parent, &setting),
        };
        Building {
            text,
            rows,
            nav: vec![btn("‹ Back", Action::open(back))],
        }
    }

    /// The page of a draft: its fields, which change the draft, and 💾 Save
    /// once the draft is complete and the store would accept it.
    async fn draft(
        &self,
        snapshot: &Snapshot<S>,
        catalog: &Catalog,
        target: &Target,
        setting: Setting<'_>,
        value: &Value,
    ) -> Building {
        let Kind::Form(form) = setting.kind else {
            unreachable!("only forms have drafts");
        };
        let empty = Map::new();
        let object = value.as_object().unwrap_or(&empty);
        let missing: Vec<_> = form
            .missing(object)
            .into_iter()
            .map(|field| field.key.clone())
            .collect();

        let mut text = Text::new();
        text.push(Span::bold(setting.title()))
            .push_plain(format!(" · {DRAFT_MARK}\n"))
            .push(Span::italic(&*setting.setting.description))
            .push_plain("\n\n");

        let mut rows = Vec::new();
        let mut lines = Text::new();
        for (position, field) in form.fields.iter().enumerate() {
            if !field.applies(object) {
                continue;
            }
            let field_target = target.field(position);
            let Some(field_setting) = Setting::resolve(catalog, &field_target) else {
                continue;
            };
            let summary = match field_setting.value_in(value) {
                Some(field_value) => {
                    self.summary_value(snapshot, &field_setting, Some(field_value))
                }
                None if missing.contains(&field.key) => "⚠️ needed".to_string(),
                None => "—".to_string(),
            };
            if !lines.0.is_empty() {
                lines.push_plain("\n");
            }
            lines
                .push_plain("• ")
                .push(Span::bold(&*field.title))
                .push_plain(format!(": {summary}"));
            rows.push(vec![btn(
                format!("{}: {summary} ›", field.title),
                in_draft(value, Action::open(Page::Setting(field_target))),
            )]);
        }
        text.append(lines);

        if missing.is_empty() {
            match self.store.check(&setting.key(), value.clone()).await {
                Ok(()) => rows.push(vec![btn(
                    "💾 Save",
                    Action::SaveDraft(target.clone(), value.clone()),
                )]),
                Err(error) => {
                    text.push_plain(format!("\n\n❌ {error}"));
                }
            }
        } else {
            text.push_plain("\n\nFill in the fields marked ⚠️, then 💾 Save.");
        }

        // Cancel goes back to the stored value, or to the map for a new
        // entry.
        let stored = setting.value(snapshot).is_some();
        let cancel = if stored {
            Page::Setting(target.clone())
        } else {
            back_from(target.parent(), &setting)
        };
        Building {
            text,
            rows,
            nav: vec![btn("✖️ Cancel", Action::open(cancel))],
        }
    }

    /// A short description of the value, for buttons.
    pub(crate) fn summary(&self, snapshot: &Snapshot<S>, setting: &Setting<'_>) -> String {
        self.summary_value(snapshot, setting, setting.value(snapshot))
    }

    /// A short description of `value`, a value of `setting`.
    pub(crate) fn summary_value(
        &self,
        snapshot: &Snapshot<S>,
        setting: &Setting<'_>,
        value: Option<Value>,
    ) -> String {
        summary(snapshot, setting.kind, value, |domain| {
            self.domain_of(domain)
                .map_or(("item", "items"), |domain| domain.noun)
        })
    }
}

/// Marks a draft in its title.
const DRAFT_MARK: &str = "📝 draft, not saved";

/// `action`, pressed on the draft `value`.
pub(crate) fn in_draft(value: &Value, action: Action) -> Action {
    Action::InDraft {
        value: value.clone(),
        then: Box::new(action),
    }
}

/// Where ‹ Back leads from `setting`: the form or the map holding it
/// (`parent`), else its section, else home.
fn back_from(parent: Option<Target>, setting: &Setting<'_>) -> Page {
    match (parent, &setting.setting.section) {
        (Some(parent), _) => Page::Setting(parent),
        (None, Some(section)) => Page::Section(section.to_string()),
        (None, None) => Page::Home,
    }
}

/// Whether the choices of a pick-one kind are a fixed list (picked by
/// position) rather than computed (picked by value).
fn is_fixed(kind: &Kind) -> bool {
    match kind {
        Kind::OneOf { choices, .. } => matches!(choices, Choices::Fixed(_)),
        // Suggested numbers are compiled in.
        Kind::Number { .. } => true,
        _ => false,
    }
}

/// The rows of `part` of a screen: all of them, then `nav`, when they fit
/// `limits.max_rows`; otherwise the part's share of the content rows, then
/// one row with ◀ / ▶ around `nav`. Rows longer than `limits.max_per_row`
/// are split.
pub(crate) fn paged(
    rows: Vec<Vec<Btn>>,
    nav: Vec<Btn>,
    limits: &Limits,
    part: usize,
    arrow: impl Fn(usize) -> Action,
) -> Vec<Vec<Btn>> {
    let per_row = limits.max_per_row.max(1);
    let split = |rows: Vec<Vec<Btn>>| -> Vec<Vec<Btn>> {
        rows.into_iter()
            .flat_map(|row| row.chunks(per_row).map(<[Btn]>::to_vec).collect::<Vec<_>>())
            .collect()
    };
    let rows = split(rows);
    let nav_rows = usize::from(!nav.is_empty());

    let max_rows = match limits.max_rows {
        Some(max) if rows.len() + nav_rows > max && max >= 2 => max,
        _ => {
            let mut all = rows;
            all.extend(split(vec![nav]).into_iter().filter(|row| !row.is_empty()));
            return all;
        }
    };

    let per_part = max_rows - 1;
    let parts = rows.len().div_ceil(per_part);
    let part = part.min(parts - 1);
    let mut shown: Vec<_> = rows
        .into_iter()
        .skip(part * per_part)
        .take(per_part)
        .collect();
    let mut last = Vec::new();
    if part > 0 {
        last.push(btn("◀", arrow(part - 1)));
    }
    last.extend(nav);
    if part + 1 < parts {
        last.push(btn("▶", arrow(part + 1)));
    }
    shown.extend(split(vec![last]));
    shown
}

/// The titles of the fields that setting this one clears, e.g. `Colour or
/// Scene`.
fn excluded_fields(setting: &Setting<'_>) -> Option<String> {
    let (field, Kind::Form(form)) = (setting.field?, setting.form_kind()) else {
        return None;
    };
    let group = field.group.as_ref()?;
    let others: Vec<_> = form
        .fields
        .iter()
        .filter(|other| other.group.as_ref() == Some(group) && other.key != field.key)
        .map(|other| &*other.title)
        .collect();
    (!others.is_empty()).then(|| others.join(" or "))
}

/// A short description of the value, for buttons.
pub(crate) fn summary<S: Schema>(
    snapshot: &Snapshot<S>,
    kind: &Kind,
    value: Option<Value>,
    nouns: impl Fn(&str) -> (&'static str, &'static str),
) -> String {
    let Some(value) = value else {
        return "not set".to_string();
    };
    let count = |count: usize, one: &str, many: &str| match count {
        0 => "none".to_string(),
        1 => format!("1 {one}"),
        count => format!("{count} {many}"),
    };

    let summary = match kind {
        Kind::SetOf { choices, inverted } => {
            let listed = list(Some(&value));
            let choices = choices.resolve(snapshot);
            let on = choices
                .iter()
                .filter(|choice| listed.contains(&Value::String(choice.value.clone())) != *inverted)
                .count();
            format!("{on} of {} on", choices.len())
        }
        Kind::Ref { domain, many: true } => {
            let (one, many) = nouns(domain);
            count(list(Some(&value)).len(), one, many)
        }
        Kind::Map { .. } => match &value {
            Value::Object(entries) if !entries.is_empty() => {
                entries.keys().cloned().collect::<Vec<_>>().join(", ")
            }
            _ => "none".to_string(),
        },
        Kind::Form(form) => {
            let parts: Vec<_> = form
                .fields
                .iter()
                .filter_map(|field| {
                    let value = value.get(&*field.key).filter(|value| !value.is_null())?;
                    Some(match &field.kind {
                        Kind::Number { unit, .. } => format!("{}{unit}", plain(value)),
                        _ => plain(value),
                    })
                })
                .collect();
            if parts.is_empty() {
                "empty".to_string()
            } else {
                parts.join(" · ")
            }
        }
        Kind::Number { unit, .. } => format!("{}{unit}", plain(&value)),
        Kind::OneOf { choices, .. } => label_of(&value, &choices.resolve(snapshot)),
        Kind::Text { .. } | Kind::Ref { many: false, .. } | Kind::Json => plain(&value),
    };
    truncate(&summary, MAX_LABEL_CHARS)
}

/// `Name — <id>`, or the id alone.
fn item_line(name: Option<&Name>, id: &Value) -> Text {
    let mut text = Text::new();
    if let Some(name) = name {
        text.push_plain(format!("{} — ", name.full));
    }
    text.push(Span::code(plain(id)));
    text
}

/// The name of the reference `id` of `domain`, if the front end knows it.
pub(crate) async fn name_of(viewer: &Viewer<'_>, domain: &str, id: &Value) -> Option<Name> {
    viewer.names?.name(domain, id).await
}

/// `Now: <value>`.
fn current_line(value: Option<&Value>) -> Text {
    match value {
        Some(value) => {
            let mut text = Text::plain("Now: ");
            text.push(Span::code(plain(value)));
            text
        }
        None => Text::plain("Not set."),
    }
}

/// Strings without their quotes, the rest as JSON.
pub(crate) fn plain(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        value => value.to_string(),
    }
}

/// The label of the choice `value` is, or the value itself.
fn label_of(value: &Value, choices: &[Choice]) -> String {
    choices
        .iter()
        .find(|choice| value.as_str() == Some(&choice.value))
        .map_or_else(|| plain(value), |choice| choice.label.clone())
}

pub(crate) fn list(value: Option<&Value>) -> Vec<Value> {
    match value {
        Some(Value::Array(items)) => items.clone(),
        _ => Vec::new(),
    }
}

/// Whether the key (or one of its entries) is stored in the database, so
/// that it can be reset to the config file's value.
fn is_overridden<S: Schema>(snapshot: &Snapshot<S>, key: &str) -> bool {
    snapshot
        .overrides()
        .keys()
        .chain(snapshot.ignored().keys())
        .any(|stored| stored == key || is_below(stored, key))
}

fn changed_marker<S: Schema>(snapshot: &Snapshot<S>, key: &str) -> &'static str {
    if snapshot.source(key) == Source::Stored || is_overridden(snapshot, key) {
        " ✏️"
    } else {
        ""
    }
}
