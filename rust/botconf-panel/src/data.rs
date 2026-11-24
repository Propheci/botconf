//! Data pages: screens the host defines, fed by a [`DataSource`] (e.g. a
//! bridge's health checks or stored messages), next to the settings.
//!
//! The host names its pages by routes, opaque strings such as
//! `conv/7/messages?p=2`; `""` is the root, opened from the home screen's
//! entry button (see [`crate::Panel::data`]). A [`DataPage`] becomes an
//! ordinary [`crate::Screen`]: its items one button per row, its actions
//! as buttons, ‹ Prev / Next ›, then ‹ Back. An action goes to another page,
//! runs at once, asks ✅ Yes / ✖️ No first, asks for a typed answer, or
//! offers choices. A press reads the page again, so an action the host no
//! longer offers does not run.

use futures::future::BoxFuture;

use botconf::Schema;

use crate::{
    Ask, Context, DataQuestion, Input, Panel, PanelError, Question, Reaction, Viewer,
    screens::{Building, btn},
    text::{Span, Text},
    token::{Action, Page},
};

const NOT_OFFERED: &str = "This action is no longer offered";
const NO_DATA: &str = "This panel has no data pages";

/// Who is looking at a data page.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DataViewer {
    /// The viewer's actor, as for settings changes.
    pub actor: Option<String>,
    /// Where the panel was opened, when the front end knows.
    pub context: Option<Context>,
}

/// A page of the host's data.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DataPage {
    pub title: String,
    pub body: Text,
    /// One button per row, each opening its route.
    pub items: Vec<DataItem>,
    /// One button per row, after the items.
    pub actions: Vec<DataAction>,
    pub paging: Option<Paging>,
    /// Where ‹ Back leads; the settings' home screen when `None`.
    pub back: Option<String>,
}

/// An item of a data page: a button opening its route.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DataItem {
    pub label: String,
    pub route: String,
}

/// The neighbouring pages of a long list, as routes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Paging {
    pub prev: Option<String>,
    pub next: Option<String>,
}

/// Something a data page offers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DataAction {
    /// Names the action to [`DataSource::act`].
    pub id: String,
    pub label: String,
    pub kind: DataActionKind,
}

/// What pressing a [`DataAction`] does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DataActionKind {
    /// Opens the page at this route.
    Go(String),
    /// Runs at once.
    Run,
    /// Asks ✅ Yes / ✖️ No first; destructive actions use it.
    Confirm { prompt: String },
    /// Asks for a typed answer, then runs with it.
    Ask { prompt: String, input: InputKind },
    /// Offers buttons, then runs with the one picked.
    Choose {
        prompt: String,
        options: Vec<DataChoice>,
    },
}

/// What an `Ask` action takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputKind {
    Text,
    /// A whole number.
    Number,
}

/// A choice of a `Choose` action.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DataChoice {
    pub id: String,
    pub label: String,
}

/// What an action runs with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DataInput {
    Text(String),
    Number(i64),
    /// The id of the [`DataChoice`] picked.
    Choice(String),
}

/// What running an action leads to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ActOutcome {
    /// Show the page at `route`, with `notice` as a toast.
    Show {
        route: String,
        notice: Option<String>,
    },
    /// Show this text, with ‹ Back to the page the action was on.
    Message(Text),
}

/// The host's data, for the panel's data pages. An `Err` is a short reason
/// shown to the viewer.
pub trait DataSource: Send + Sync + 'static {
    /// The page at `route`.
    fn page<'a>(
        &'a self,
        viewer: &'a DataViewer,
        route: &'a str,
    ) -> BoxFuture<'a, Result<DataPage, String>>;

    /// Runs the action `action` of the page at `route`.
    fn act<'a>(
        &'a self,
        viewer: &'a DataViewer,
        route: &'a str,
        action: &'a str,
        input: Option<DataInput>,
    ) -> BoxFuture<'a, Result<ActOutcome, String>>;

    /// The page to open the panel on in `context` (where it was opened),
    /// instead of the home screen. Default: none.
    fn open<'a>(
        &'a self,
        _viewer: &'a DataViewer,
        _context: &'a Context,
    ) -> BoxFuture<'a, Option<String>> {
        Box::pin(async { None })
    }
}

fn data_viewer(viewer: &Viewer<'_>) -> DataViewer {
    DataViewer {
        actor: viewer.actor.clone(),
        context: viewer.context.clone(),
    }
}

impl<S: Schema> Panel<S> {
    /// The panel as the command opens it: the data page about where it was
    /// opened, when the data source has one, else the home screen.
    pub async fn open(&self, viewer: &Viewer<'_>) -> crate::Screen {
        if let (Some(entry), Some(context)) = (&self.data, &viewer.context) {
            let data_viewer = data_viewer(viewer);
            if let Some(route) = entry.source.open(&data_viewer, context).await {
                return self.render(&Page::Data(route), viewer).await;
            }
        }
        self.render(&Page::Home, viewer).await
    }

    /// The home screen's entry button to the data pages, if any.
    pub(crate) fn data_entry(&self) -> Option<crate::screens::Btn> {
        let entry = self.data.as_ref()?;
        Some(btn(
            format!("{} ›", entry.title),
            Action::open(Page::Data(String::new())),
        ))
    }

    /// The data page at `route`, ready to be finished into a screen.
    pub(crate) async fn data_page(&self, route: &str, viewer: &Viewer<'_>) -> Building {
        let Some(entry) = &self.data else {
            return failed(NO_DATA, None);
        };
        let page = match entry.source.page(&data_viewer(viewer), route).await {
            Ok(page) => page,
            Err(reason) => return failed(&reason, None),
        };

        let mut text = Text::new();
        text.push(Span::bold(page.title));
        if !page.body.is_empty() {
            text.push_plain("\n\n").append(page.body);
        }
        let mut rows: Vec<_> = page
            .items
            .into_iter()
            .map(|item| vec![btn(item.label, Action::open(Page::Data(item.route)))])
            .collect();
        rows.extend(page.actions.into_iter().map(|action| {
            let pressed = match action.kind {
                DataActionKind::Go(to) => Action::open(Page::Data(to)),
                _ => Action::DataPress(route.to_owned(), action.id),
            };
            vec![btn(action.label, pressed)]
        }));
        if let Some(paging) = page.paging {
            let mut row = Vec::new();
            if let Some(prev) = paging.prev {
                row.push(btn("‹ Prev", Action::open(Page::Data(prev))));
            }
            if let Some(next) = paging.next {
                row.push(btn("Next ›", Action::open(Page::Data(next))));
            }
            if !row.is_empty() {
                rows.push(row);
            }
        }
        Building {
            text,
            rows,
            nav: vec![back_button(page.back)],
        }
    }

    /// A data action was pressed: read the page again and do what the
    /// action's kind says.
    pub(crate) async fn data_press(
        &self,
        route: String,
        id: String,
        viewer: &Viewer<'_>,
    ) -> Result<Reaction<S>, PanelError> {
        let action = self.offered(&route, &id, viewer).await?;
        let stay = Page::Data(route.clone());
        match action.kind {
            DataActionKind::Go(to) => {
                Ok(Reaction::screen(self.render(&Page::Data(to), viewer).await))
            }
            DataActionKind::Run => self.data_do(route, id, None, viewer).await,
            DataActionKind::Confirm { prompt } => {
                let rows = vec![
                    vec![btn("✅ Yes", Action::DataDo(route.clone(), id, None))],
                    vec![btn("✖️ No", Action::open(stay))],
                ];
                Ok(Reaction::screen(crate::Screen {
                    text: Text::plain(prompt),
                    rows: self.issue_rows(rows, viewer.limits).await,
                }))
            }
            DataActionKind::Choose { prompt, options } => {
                let mut rows: Vec<_> = options
                    .into_iter()
                    .map(|option| {
                        vec![btn(
                            option.label,
                            Action::DataDo(route.clone(), id.clone(), Some(option.id)),
                        )]
                    })
                    .collect();
                rows.push(vec![btn("‹ Back", Action::open(stay))]);
                Ok(Reaction::screen(crate::Screen {
                    text: Text::plain(prompt),
                    rows: self.issue_rows(rows, viewer.limits).await,
                }))
            }
            DataActionKind::Ask { prompt, .. } => {
                let back = self
                    .issue_rows(vec![vec![btn("‹ Back", Action::open(stay))]], viewer.limits)
                    .await
                    .into_iter()
                    .flatten()
                    .next();
                Ok(Reaction {
                    ask: Some(Ask {
                        question: Question::Data(DataQuestion { route, action: id }),
                        title: action.label,
                        text: Text::plain(prompt),
                        picker: None,
                        current: None,
                        rows: Vec::new(),
                        back,
                    }),
                    notice: Some("✏️ Waiting for your answer".into()),
                    ..Reaction::none()
                })
            }
        }
    }

    /// Runs a data action, still offered, with `choice` picked, if any.
    pub(crate) async fn data_do(
        &self,
        route: String,
        id: String,
        choice: Option<String>,
        viewer: &Viewer<'_>,
    ) -> Result<Reaction<S>, PanelError> {
        let action = self.offered(&route, &id, viewer).await?;
        if let (Some(choice), DataActionKind::Choose { options, .. }) = (&choice, &action.kind)
            && !options.iter().any(|option| &option.id == choice)
        {
            return Err(PanelError::Stale {
                notice: "This choice no longer exists".into(),
                screen: self.render(&Page::Data(route), viewer).await,
            });
        }
        self.run_data(route, &id, choice.map(DataInput::Choice), viewer)
            .await
    }

    /// The answer to a data action's question: read as the action asks,
    /// then the action runs with it.
    pub(crate) async fn data_answer(
        &self,
        question: &DataQuestion,
        input: Input,
        viewer: &Viewer<'_>,
    ) -> Result<Reaction<S>, PanelError> {
        let page = Page::Data(question.route.clone());
        if input == Input::Cancel {
            return Ok(Reaction {
                notice: Some("Cancelled".into()),
                ..Reaction::screen(self.render(&page, viewer).await)
            });
        }
        let action = self
            .offered(&question.route, &question.action, viewer)
            .await?;
        let DataActionKind::Ask { input: kind, .. } = action.kind else {
            return Err(PanelError::Stale {
                notice: NOT_OFFERED.into(),
                screen: self.render(&page, viewer).await,
            });
        };
        let Input::Text(text) = input else {
            return Err(PanelError::Rejected("send the answer as text".into()));
        };
        let text = text.trim();
        let input = match kind {
            InputKind::Text => DataInput::Text(text.to_owned()),
            InputKind::Number => DataInput::Number(
                text.parse()
                    .map_err(|_| PanelError::Rejected(format!("`{text}` is not a whole number")))?,
            ),
        };
        self.run_data(
            question.route.clone(),
            &question.action,
            Some(input),
            viewer,
        )
        .await
    }

    /// The action `id` of the page at `route`, when the page still offers
    /// it; otherwise the page is redrawn.
    async fn offered(
        &self,
        route: &str,
        id: &str,
        viewer: &Viewer<'_>,
    ) -> Result<DataAction, PanelError> {
        let entry = self
            .data
            .as_ref()
            .ok_or_else(|| PanelError::Rejected(NO_DATA.into()))?;
        let page = entry
            .source
            .page(&data_viewer(viewer), route)
            .await
            .map_err(PanelError::Rejected)?;
        match page.actions.into_iter().find(|action| action.id == id) {
            Some(action) => Ok(action),
            None => Err(PanelError::Stale {
                notice: NOT_OFFERED.into(),
                screen: self.render(&Page::Data(route.to_owned()), viewer).await,
            }),
        }
    }

    /// Runs the action and shows what it led to. A failure is a notice over
    /// the page the action was on.
    async fn run_data(
        &self,
        route: String,
        id: &str,
        input: Option<DataInput>,
        viewer: &Viewer<'_>,
    ) -> Result<Reaction<S>, PanelError> {
        let entry = self
            .data
            .as_ref()
            .ok_or_else(|| PanelError::Rejected(NO_DATA.into()))?;
        let outcome = entry
            .source
            .act(&data_viewer(viewer), &route, id, input)
            .await;
        match outcome {
            Ok(ActOutcome::Show { route, notice }) => Ok(Reaction {
                screen: Some(
                    self.render_part(&Page::Data(route), 0, notice.as_deref(), viewer)
                        .await,
                ),
                notice,
                ..Reaction::none()
            }),
            Ok(ActOutcome::Message(text)) => {
                let rows = vec![vec![btn("‹ Back", Action::open(Page::Data(route)))]];
                Ok(Reaction::screen(crate::Screen {
                    text,
                    rows: self.issue_rows(rows, viewer.limits).await,
                }))
            }
            Err(reason) => {
                let notice = format!("❌ {reason}");
                Ok(Reaction {
                    screen: Some(
                        self.render_part(&Page::Data(route), 0, Some(&notice), viewer)
                            .await,
                    ),
                    notice: Some(notice),
                    ..Reaction::none()
                })
            }
        }
    }
}

/// ‹ Back to `back`, or to the settings' home screen.
fn back_button(back: Option<String>) -> crate::screens::Btn {
    let page = back.map_or(Page::Home, Page::Data);
    btn("‹ Back", Action::open(page))
}

/// A page that could not be read: why, and ‹ Back.
fn failed(reason: &str, back: Option<String>) -> Building {
    Building {
        text: Text::plain(format!("❌ {reason}")),
        rows: Vec::new(),
        nav: vec![back_button(back)],
    }
}
