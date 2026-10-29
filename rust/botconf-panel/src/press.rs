//! What pressing a button and answering a question do.

use serde_json::Value;

use botconf::{Catalog, Choices, Kind, Schema, SettingsError};

use crate::{
    Input, Panel, PanelError, Question, Reaction, Screen, Viewer,
    edit::{self, Edit},
    setting::Setting,
    text::Text,
    token::{Action, Page, Target},
    tokens::Redeemed,
};

const GONE: &str = "This setting no longer exists";
const NO_CHOICE: &str = "This choice no longer exists";
const LIST_CHANGED: &str = "This list changed";
const OUT_OF_DATE: &str = "This panel is out of date";

/// Why a button can't do what it says.
enum Problem {
    /// Tell the user.
    Rejected(&'static str),
    /// The button was drawn for a list that has changed: redraw `page`.
    Stale(&'static str, Page),
}

impl<S: Schema> Panel<S> {
    /// A button was pressed: `token` is what it carried.
    pub async fn press(&self, token: &str, viewer: &Viewer<'_>) -> Result<Reaction<S>, PanelError> {
        let redeemed = self
            .tokens
            .redeem(token, self.now())
            .await
            .map_err(PanelError::Tokens)?;
        let action = match redeemed {
            Redeemed::Action(action) => action,
            Redeemed::Expired => {
                return Err(PanelError::Stale {
                    notice: OUT_OF_DATE.into(),
                    screen: self.render(&Page::Home, viewer).await,
                });
            }
            Redeemed::Malformed => {
                return Err(PanelError::Rejected(format!(
                    "This button no longer works; send /{}",
                    self.command
                )));
            }
        };

        match action {
            Action::Close => Ok(Reaction {
                closed: true,
                ..Reaction::screen(self.closed())
            }),
            Action::Open(page, part) => Ok(Reaction::screen(
                self.render_part(&page, part, None, viewer).await,
            )),
            Action::Post(page) => Ok(Reaction {
                post: Some(self.render(&page, viewer).await),
                ..Reaction::none()
            }),
            Action::Ask(target, ask) => {
                let catalog = self.store.catalog();
                let question = Question { target, ask };
                let Some(setting) = Setting::resolve(&catalog, &question.target) else {
                    return Err(PanelError::Rejected(GONE.into()));
                };
                let snapshot = self.store.current();
                Ok(Reaction {
                    ask: Some(
                        self.question(&snapshot, &setting, question.clone(), viewer)
                            .await,
                    ),
                    notice: Some("✏️ Waiting for your answer".into()),
                    ..Reaction::none()
                })
            }
            action => {
                let (edit, page) = match self.edit_for(action) {
                    Ok(edit) => edit,
                    Err(Problem::Rejected(problem)) => {
                        return Err(PanelError::Rejected(problem.into()));
                    }
                    Err(Problem::Stale(notice, page)) => {
                        return Err(PanelError::Stale {
                            notice: notice.into(),
                            screen: self.render(&page, viewer).await,
                        });
                    }
                };
                self.apply(edit, &page, viewer, |error| format!("❌ {error}"))
                    .await
            }
        }
    }

    /// The user answered `question`.
    pub async fn answer(
        &self,
        question: &Question,
        input: Input,
        viewer: &Viewer<'_>,
    ) -> Result<Reaction<S>, PanelError> {
        let catalog = self.store.catalog();
        let Some(setting) = Setting::resolve(&catalog, &question.target) else {
            return Err(PanelError::Stale {
                notice: GONE.into(),
                screen: self.render(&Page::Home, viewer).await,
            });
        };
        let page = Page::Setting(question.target.clone());
        if input == Input::Cancel {
            return Ok(Reaction {
                notice: Some("Cancelled".into()),
                ..Reaction::screen(self.render(&page, viewer).await)
            });
        }

        let snapshot = self.store.current();
        match self
            .read(&snapshot, &setting, question, input, viewer)
            .await
        {
            Ok((edit, page)) => {
                self.apply(edit, &page, viewer, |error| error.to_string())
                    .await
            }
            Err(problem) => Err(PanelError::Rejected(problem)),
        }
    }

    /// Applies `edit` and shows `page` with the outcome. A rejected change is
    /// described by `rejected`.
    async fn apply(
        &self,
        edit: Edit,
        page: &Page,
        viewer: &Viewer<'_>,
        rejected: impl Fn(&SettingsError) -> String,
    ) -> Result<Reaction<S>, PanelError> {
        match edit::apply(&self.store, edit, viewer.actor.clone()).await {
            Ok(applied) => Ok(Reaction {
                screen: Some(
                    self.render_part(page, 0, Some(&applied.notice), viewer)
                        .await,
                ),
                notice: Some(applied.notice),
                change: applied.change,
                ..Reaction::none()
            }),
            Err(error @ SettingsError::Storage(_)) => Err(PanelError::Storage(error)),
            Err(error) => Err(PanelError::Rejected(rejected(&error))),
        }
    }

    pub(crate) fn closed(&self) -> Screen {
        Screen {
            text: Text::plain(format!(
                "Settings closed; send /{} to open them again.",
                self.command
            )),
            rows: Vec::new(),
        }
    }

    /// The edit a button makes, and the page to show afterwards.
    fn edit_for(&self, action: Action) -> Result<(Edit, Page), Problem> {
        let snapshot = self.store.current();
        let catalog = self.store.catalog();
        let resolve = |target| resolve(&catalog, target);
        let changed = |target: &Target| Problem::Stale(LIST_CHANGED, Page::Setting(target.clone()));

        let (target, edit) = match action {
            Action::Reload => return Ok((Edit::Reload, Page::Home)),
            Action::Toggle(target, choice) => {
                let setting = resolve(&target)?;
                if !has_fixed_choices(setting.kind) {
                    return Err(changed(&target));
                }
                let choice = setting
                    .kind
                    .choices(snapshot.as_ref())
                    .into_iter()
                    .nth(choice)
                    .ok_or(Problem::Rejected(NO_CHOICE))?;
                let edit = toggle(&setting, &snapshot, Value::String(choice.value));
                (target, edit)
            }
            Action::ToggleValue(target, item) => {
                let setting = resolve(&target)?;
                if !is_choice(&setting, &snapshot, &item) {
                    return Err(changed(&target));
                }
                let edit = toggle(&setting, &snapshot, item);
                (target, edit)
            }
            Action::Pick(target, choice) => {
                let setting = resolve(&target)?;
                if !has_fixed_choices(setting.kind) {
                    return Err(changed(&target));
                }
                let choice = setting
                    .kind
                    .choices(snapshot.as_ref())
                    .into_iter()
                    .nth(choice)
                    .ok_or(Problem::Rejected(NO_CHOICE))?;
                return pick(&setting, &snapshot, &target, &choice.value);
            }
            Action::PickValue(target, value) => {
                let setting = resolve(&target)?;
                if !is_choice(&setting, &snapshot, &value) {
                    return Err(changed(&target));
                }
                let Value::String(value) = value else {
                    return Err(changed(&target));
                };
                return pick(&setting, &snapshot, &target, &value);
            }
            Action::Add(target, item) => {
                let setting = resolve(&target)?;
                let edit = setting.change_list(&snapshot, vec![item], None);
                (target, edit)
            }
            Action::Remove(target, item) => {
                let setting = resolve(&target)?;
                let listed = matches!(
                    setting.value(&snapshot),
                    Some(Value::Array(items)) if items.contains(&item)
                );
                if !listed {
                    return Err(changed(&target));
                }
                let edit = setting.change_list(&snapshot, Vec::new(), Some(item));
                (target, edit)
            }
            Action::Clear(target) => {
                let setting = resolve(&target)?;
                let edit = setting.set(&snapshot, None);
                (target, edit)
            }
            Action::Reset(target) => {
                let key = resolve(&target)?.key();
                (target, Edit::Reset(key))
            }
            Action::Delete(target) => {
                let key = resolve(&target)?.key();
                let parent = target.parent().ok_or(Problem::Rejected(GONE))?;
                return Ok((Edit::DeleteEntry(key), Page::Setting(parent)));
            }
            Action::Open(..) | Action::Post(_) | Action::Ask(..) | Action::Close => {
                unreachable!("handled by press")
            }
        };
        Ok((edit, Page::Setting(target)))
    }
}

/// The setting `target` refers to.
fn resolve<'a>(catalog: &'a Catalog, target: &'a Target) -> Result<Setting<'a>, Problem> {
    Setting::resolve(catalog, target).ok_or(Problem::Rejected(GONE))
}

/// Whether the choices a setting offers are compiled in, so that buttons may
/// pick them by position.
fn has_fixed_choices(kind: &Kind) -> bool {
    match kind {
        Kind::OneOf { choices, .. } | Kind::SetOf { choices, .. } => {
            matches!(choices, Choices::Fixed(_))
        }
        Kind::Map {
            names: Some(names), ..
        } => matches!(names, Choices::Fixed(_)),
        Kind::Number { .. } => true,
        _ => false,
    }
}

/// Whether `value` is one of the setting's current choices.
fn is_choice<S: Schema>(
    setting: &Setting<'_>,
    snapshot: &botconf::Snapshot<S>,
    value: &Value,
) -> bool {
    setting
        .kind
        .choices(snapshot)
        .iter()
        .any(|choice| value.as_str() == Some(choice.value.as_str()))
}

/// Adds `item` to the list, or removes it if it is there.
fn toggle<S: Schema>(setting: &Setting<'_>, snapshot: &botconf::Snapshot<S>, item: Value) -> Edit {
    let listed = matches!(
        setting.value(snapshot),
        Some(Value::Array(items)) if items.contains(&item)
    );
    if listed {
        setting.change_list(snapshot, Vec::new(), Some(item))
    } else {
        setting.change_list(snapshot, vec![item], None)
    }
}

/// Picks the choice `value`: the value of a setting, or a new map entry
/// named after it.
fn pick<S: Schema>(
    setting: &Setting<'_>,
    snapshot: &botconf::Snapshot<S>,
    target: &Target,
    value: &str,
) -> Result<(Edit, Page), Problem> {
    if let Kind::Map { value: entry, .. } = setting.kind {
        let empty = entry
            .empty_value()
            .ok_or(Problem::Rejected("This entry must be typed in; use ➕ Add"))?;
        let key = format!("{}.{value}", setting.key());
        return Ok((Edit::Set(key, empty), Page::Setting(target.entry(value))));
    }
    let picked = setting.kind.choice_value(value);
    Ok((
        setting.set(snapshot, Some(picked)),
        Page::Setting(target.clone()),
    ))
}
