//! Data pages, driven as a front end would.

use std::sync::{Arc, Mutex};

use futures::future::BoxFuture;

use crate::{
    ActOutcome, Context, DataAction, DataActionKind, DataChoice, DataInput, DataItem, DataPage,
    DataSource, DataViewer, Input, InputKind, Page, Paging, Panel, PanelError, Question, Screen,
    Text,
    testing::{FakeNames, LIMITS, Toy, panel, viewer},
};

const PER_PAGE: usize = 10;
const ITEMS: usize = 25;

/// What the fake source was asked to run: (route, action, input).
type Calls = Arc<Mutex<Vec<(String, String, Option<DataInput>)>>>;

/// A list of 25 items, 10 per page (`list?p=N`), with an action of each
/// kind; `gone` stops offering `purge`.
#[derive(Clone, Default)]
struct Fake {
    calls: Calls,
    gone: Arc<Mutex<bool>>,
}

impl Fake {
    fn list(&self, page: usize) -> DataPage {
        let first = page * PER_PAGE;
        let items = (first..ITEMS.min(first + PER_PAGE))
            .map(|n| DataItem {
                label: format!("item {n}"),
                route: format!("item/{n}"),
            })
            .collect();
        let mut actions = vec![
            DataAction {
                id: "run".into(),
                label: "▶ Run".into(),
                kind: DataActionKind::Run,
            },
            DataAction {
                id: "older".into(),
                label: "🧹 Older than…".into(),
                kind: DataActionKind::Ask {
                    prompt: "How many days?".into(),
                    input: InputKind::Number,
                },
            },
            DataAction {
                id: "mode".into(),
                label: "Mode".into(),
                kind: DataActionKind::Choose {
                    prompt: "Which?".into(),
                    options: vec![
                        DataChoice {
                            id: "records".into(),
                            label: "Records only".into(),
                        },
                        DataChoice {
                            id: "all".into(),
                            label: "With copies".into(),
                        },
                    ],
                },
            },
        ];
        if !*self.gone.lock().unwrap() {
            actions.push(DataAction {
                id: "purge".into(),
                label: "🗑 Purge".into(),
                kind: DataActionKind::Confirm {
                    prompt: "Purge everything?".into(),
                },
            });
        }
        DataPage {
            title: "List".into(),
            body: Text::plain(format!("{ITEMS} items")),
            items,
            actions,
            paging: Some(Paging {
                prev: page.checked_sub(1).map(|prev| format!("list?p={prev}")),
                next: (first + PER_PAGE < ITEMS).then(|| format!("list?p={}", page + 1)),
            }),
            back: Some(String::new()),
        }
    }

    fn calls(&self) -> Vec<(String, String, Option<DataInput>)> {
        self.calls.lock().unwrap().clone()
    }
}

impl DataSource for Fake {
    fn page<'a>(
        &'a self,
        _viewer: &'a DataViewer,
        route: &'a str,
    ) -> BoxFuture<'a, Result<DataPage, String>> {
        Box::pin(async move {
            if route.is_empty() {
                return Ok(DataPage {
                    title: "Data".into(),
                    items: vec![DataItem {
                        label: "List".into(),
                        route: "list?p=0".into(),
                    }],
                    ..DataPage::default()
                });
            }
            match route.strip_prefix("list?p=") {
                Some(page) => Ok(self.list(page.parse().map_err(|_| "bad page")?)),
                None => Err(format!("no page {route}")),
            }
        })
    }

    fn act<'a>(
        &'a self,
        _viewer: &'a DataViewer,
        route: &'a str,
        action: &'a str,
        input: Option<DataInput>,
    ) -> BoxFuture<'a, Result<ActOutcome, String>> {
        Box::pin(async move {
            self.calls
                .lock()
                .unwrap()
                .push((route.into(), action.into(), input));
            match action {
                "run" => Ok(ActOutcome::Show {
                    route: route.into(),
                    notice: Some("✅ Ran".into()),
                }),
                "mode" => Ok(ActOutcome::Message(Text::plain("Done"))),
                _ => Err("it failed".into()),
            }
        })
    }

    fn open<'a>(
        &'a self,
        _viewer: &'a DataViewer,
        context: &'a Context,
    ) -> BoxFuture<'a, Option<String>> {
        Box::pin(async move { (context.chat == "c1").then(|| "list?p=0".to_string()) })
    }
}

async fn data_panel() -> (Panel<Toy>, Fake) {
    let fake = Fake::default();
    (panel().await.data("🗄 Data", fake.clone()), fake)
}

fn labels(screen: &Screen) -> Vec<String> {
    screen
        .rows
        .iter()
        .flatten()
        .map(|button| button.label.clone())
        .collect()
}

fn find(screen: &Screen, label: &str) -> String {
    screen
        .rows
        .iter()
        .flatten()
        .find(|button| button.label.contains(label))
        .unwrap_or_else(|| panic!("no {label:?} in {:?}", labels(screen)))
        .token
        .clone()
}

async fn render(panel: &Panel<Toy>, route: &str) -> Screen {
    panel
        .render(&Page::Data(route.into()), &viewer(&FakeNames, &LIMITS))
        .await
}

async fn press(panel: &Panel<Toy>, token: &str) -> Result<crate::Reaction<Toy>, PanelError> {
    panel.press(token, &viewer(&FakeNames, &LIMITS)).await
}

#[tokio::test]
async fn the_home_screen_opens_the_data_pages() {
    let (panel, _) = data_panel().await;
    let home = panel
        .render(&Page::Home, &viewer(&FakeNames, &LIMITS))
        .await;
    let root = press(&panel, &find(&home, "🗄 Data ›"))
        .await
        .unwrap()
        .screen
        .unwrap();
    assert!(root.text.to_plain().starts_with("Data"));
    let list = press(&panel, &find(&root, "List"))
        .await
        .unwrap()
        .screen
        .unwrap();
    assert!(list.text.to_plain().contains("25 items"));
}

#[tokio::test]
async fn a_long_list_is_paged() {
    let (panel, _) = data_panel().await;
    let first = render(&panel, "list?p=0").await;
    let shown = labels(&first);
    assert_eq!(
        shown
            .iter()
            .filter(|label| label.starts_with("item "))
            .count(),
        10
    );
    assert!(shown.contains(&"Next ›".to_string()));
    assert!(!shown.contains(&"‹ Prev".to_string()));
    assert_eq!(shown.last().map(String::as_str), Some("‹ Back"));

    let second = press(&panel, &find(&first, "Next ›"))
        .await
        .unwrap()
        .screen
        .unwrap();
    assert!(labels(&second).contains(&"item 10".to_string()));
    let last = press(&panel, &find(&second, "Next ›"))
        .await
        .unwrap()
        .screen
        .unwrap();
    let shown = labels(&last);
    assert_eq!(
        shown
            .iter()
            .filter(|label| label.starts_with("item "))
            .count(),
        5
    );
    assert!(shown.contains(&"‹ Prev".to_string()));
    assert!(!shown.contains(&"Next ›".to_string()));
}

#[tokio::test]
async fn a_run_action_runs_at_once_and_shows_its_page() {
    let (panel, fake) = data_panel().await;
    let list = render(&panel, "list?p=1").await;
    let reaction = press(&panel, &find(&list, "▶ Run")).await.unwrap();
    assert_eq!(reaction.notice.as_deref(), Some("✅ Ran"));
    assert!(reaction.screen.unwrap().text.to_plain().contains("✅ Ran"));
    assert_eq!(fake.calls(), [("list?p=1".into(), "run".into(), None)]);
}

#[tokio::test]
async fn a_confirmed_action_runs_only_on_yes() {
    let (panel, fake) = data_panel().await;
    let list = render(&panel, "list?p=0").await;
    let confirm = press(&panel, &find(&list, "🗑 Purge"))
        .await
        .unwrap()
        .screen
        .unwrap();
    assert_eq!(confirm.text.to_plain(), "Purge everything?");
    assert!(fake.calls().is_empty(), "nothing runs before Yes");

    let back = press(&panel, &find(&confirm, "✖️ No"))
        .await
        .unwrap()
        .screen
        .unwrap();
    assert!(back.text.to_plain().contains("25 items"));
    assert!(fake.calls().is_empty(), "No runs nothing");

    let failed = press(&panel, &find(&confirm, "✅ Yes")).await.unwrap();
    assert_eq!(failed.notice.as_deref(), Some("❌ it failed"));
    assert_eq!(fake.calls(), [("list?p=0".into(), "purge".into(), None)]);
}

#[tokio::test]
async fn an_action_no_longer_offered_does_not_run() {
    let (panel, fake) = data_panel().await;
    let list = render(&panel, "list?p=0").await;
    let confirm = press(&panel, &find(&list, "🗑 Purge"))
        .await
        .unwrap()
        .screen
        .unwrap();
    *fake.gone.lock().unwrap() = true;

    let Err(PanelError::Stale { notice, screen }) = press(&panel, &find(&confirm, "✅ Yes")).await
    else {
        panic!("a stale action");
    };
    assert_eq!(notice, "This action is no longer offered");
    assert!(!labels(&screen).contains(&"🗑 Purge".to_string()));
    assert!(fake.calls().is_empty());
}

#[tokio::test]
async fn an_ask_action_takes_a_number() {
    let (panel, fake) = data_panel().await;
    let list = render(&panel, "list?p=0").await;
    let ask = press(&panel, &find(&list, "🧹 Older"))
        .await
        .unwrap()
        .ask
        .expect("a question");
    assert_eq!(ask.text.to_plain(), "How many days?");
    let question = Question::parse(&ask.question.encode()).unwrap();
    assert_eq!(question, ask.question);
    let viewer = viewer(&FakeNames, &LIMITS);

    let Err(PanelError::Rejected(problem)) = panel
        .answer(&question, Input::Text("soon".into()), &viewer)
        .await
    else {
        panic!("not a number");
    };
    assert!(problem.contains("not a whole number"), "{problem}");
    assert!(fake.calls().is_empty());

    panel
        .answer(&question, Input::Text(" 30 ".into()), &viewer)
        .await
        .unwrap();
    assert_eq!(
        fake.calls(),
        [(
            "list?p=0".into(),
            "older".into(),
            Some(DataInput::Number(30))
        )]
    );

    let cancelled = panel
        .answer(&question, Input::Cancel, &viewer)
        .await
        .unwrap();
    assert_eq!(cancelled.notice.as_deref(), Some("Cancelled"));
}

#[tokio::test]
async fn a_choose_action_offers_its_options() {
    let (panel, fake) = data_panel().await;
    let list = render(&panel, "list?p=0").await;
    let choose = press(&panel, &find(&list, "Mode"))
        .await
        .unwrap()
        .screen
        .unwrap();
    assert_eq!(labels(&choose), ["Records only", "With copies", "‹ Back"]);

    let done = press(&panel, &find(&choose, "With copies"))
        .await
        .unwrap()
        .screen
        .unwrap();
    assert_eq!(done.text.to_plain(), "Done");
    assert_eq!(labels(&done), ["‹ Back"]);
    assert_eq!(
        fake.calls(),
        [(
            "list?p=0".into(),
            "mode".into(),
            Some(DataInput::Choice("all".into()))
        )]
    );
}

#[tokio::test]
async fn a_page_that_cannot_be_read_says_why() {
    let (panel, _) = data_panel().await;
    let screen = render(&panel, "nowhere").await;
    assert_eq!(screen.text.to_plain(), "❌ no page nowhere");
    assert_eq!(labels(&screen), ["‹ Back"]);
}

#[tokio::test]
async fn the_panel_opens_on_the_page_about_where_it_was_opened() {
    let (panel, _) = data_panel().await;
    let mut viewer = viewer(&FakeNames, &LIMITS);
    viewer.context = Some(Context {
        chat: "c1".into(),
        thread: None,
    });
    assert!(
        panel
            .open(&viewer)
            .await
            .text
            .to_plain()
            .contains("25 items")
    );

    viewer.context = Some(Context {
        chat: "elsewhere".into(),
        thread: Some("7".into()),
    });
    let home = panel.open(&viewer).await;
    assert!(labels(&home).contains(&"🗄 Data ›".to_string()));

    viewer.context = None;
    assert_eq!(panel.open(&viewer).await, home);
}

#[tokio::test]
async fn without_data_pages_the_home_screen_has_no_entry() {
    let panel = panel().await;
    let home = panel
        .render(&Page::Home, &viewer(&FakeNames, &LIMITS))
        .await;
    assert!(!labels(&home).iter().any(|label| label.contains("Data")));
}
