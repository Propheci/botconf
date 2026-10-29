//! The panel driven as a front end would: render, press a button's token,
//! answer a question.

use std::sync::{
    Arc,
    atomic::{AtomicI64, Ordering},
};

use botconf::{DynamicSection, Kind, RuntimeSetting};
use serde_json::{Value, json};

use crate::{
    Action, AskKind, Input, Limits, Page, Panel, PanelError, Question, Reaction, Screen, Target,
    testing::{FLAVOURS, FakeNames, LIMITS, Toy, panel, viewer},
    tokens::{HANDLE_LEN, MemoryTokenStore},
};

fn target(panel: &Panel<Toy>, key: &str) -> Target {
    let catalog = panel.store().catalog();
    let position = catalog
        .live()
        .find(|(_, entry)| entry.key == key)
        .unwrap()
        .0;
    Target::setting(position)
}

/// The buttons of a screen, as (label, token).
fn buttons(screen: &Screen) -> Vec<(String, String)> {
    screen
        .rows
        .iter()
        .flatten()
        .map(|button| (button.label.clone(), button.token.clone()))
        .collect()
}

fn labels(screen: &Screen) -> Vec<String> {
    buttons(screen)
        .into_iter()
        .map(|(label, _)| label)
        .collect()
}

/// The token of the first button whose label contains `label`.
fn find(screen: &Screen, label: &str) -> String {
    buttons(screen)
        .into_iter()
        .find(|(text, _)| text.contains(label))
        .unwrap_or_else(|| panic!("no {label:?} in {:?}", labels(screen)))
        .1
}

/// The action of a stateless token.
fn action(token: &str) -> Action {
    Action::parse(token).unwrap_or_else(|| panic!("{token} is not stateless"))
}

async fn render(panel: &Panel<Toy>, page: &Page) -> Screen {
    panel.render(page, &viewer(&FakeNames, &LIMITS)).await
}

async fn press(panel: &Panel<Toy>, token: &str) -> Reaction<Toy> {
    panel
        .press(token, &viewer(&FakeNames, &LIMITS))
        .await
        .unwrap()
}

/// Presses `token` and returns the redrawn screen.
async fn press_screen(panel: &Panel<Toy>, token: &str) -> Screen {
    press(panel, token).await.screen.expect("a screen")
}

fn value(panel: &Panel<Toy>, key: &str) -> Option<Value> {
    panel.store().current().value(key)
}

fn text(screen: &Screen) -> String {
    screen.text.to_plain()
}

/// Answers the panel's `ask` question about `key` with `input`.
async fn answer(
    panel: &Panel<Toy>,
    target: Target,
    ask: AskKind,
    input: Input,
) -> Result<Reaction<Toy>, PanelError> {
    let question = Question { target, ask };
    panel
        .answer(&question, input, &viewer(&FakeNames, &LIMITS))
        .await
}

fn typed(text: &str) -> Input {
    Input::Text(text.into())
}

#[tokio::test]
async fn the_home_screen_lists_the_settings_and_sections() {
    let panel = panel().await;
    let home = render(&panel, &Page::Home).await;
    let labels = labels(&home);

    for label in [
        "Features: 2 of 2 on",
        "Admins: none",
        "Greeting: not set",
        "Level: info",
        "Lamp ›",
    ] {
        assert!(labels.contains(&label.to_string()), "{label}: {labels:?}");
    }
    assert_eq!(
        action(&find(&home, "Lamp")),
        Action::open(Page::Section("lamp".into()))
    );

    // Sections get the host's note.
    panel
        .store()
        .set("disabled", json!(["lamp"]), None)
        .await
        .unwrap();
    let home = render(&panel, &Page::Home).await;
    find(&home, "Lamp (off) ›");
    let lamp = render(&panel, &Page::Section("lamp".into())).await;
    assert!(text(&lamp).contains("Note: off."), "{}", text(&lamp));
}

#[tokio::test]
async fn home_lists_settings_and_sections() {
    let panel = panel().await;
    let home = render(&panel, &Page::Home).await;
    let spans = &home.text.0;
    assert_eq!(spans[0].text, "⚙️ ");
    assert_eq!(spans[1], crate::Span::bold("Settings"));
    assert!(spans[2].text.starts_with("\nTap a setting to change it."));

    // The settings, then the sections, then Reload and Close.
    let rows: Vec<Vec<_>> = home
        .rows
        .iter()
        .map(|row| row.iter().map(|button| button.label.clone()).collect())
        .collect();
    assert_eq!(rows[0], ["Features: 2 of 2 on"]);
    assert_eq!(rows[rows.len() - 2], ["Lamp ›"]);
    assert_eq!(rows[rows.len() - 1], ["🔄 Reload", "✖️ Close"]);
}

#[tokio::test]
async fn dynamic_sections_are_listed_after_the_schemas() {
    let panel = panel().await;
    let section = DynamicSection {
        id: "connector:tg-1".into(),
        path: "connectors.tg-1".into(),
        title: "Telegram".into(),
        description: "The Telegram connector".into(),
        settings: vec![RuntimeSetting::owned(
            "mode",
            "How edits show",
            Kind::text(false),
        )],
        validator: Arc::new(|value: &Value| Ok(value.clone())),
        base: json!({ "mode": "in_place" }),
        base_source: "connector tg-1".into(),
    };
    panel.store().register_section(section).await.unwrap();
    let home = render(&panel, &Page::Home).await;
    let rows = labels(&home);
    let lamp = rows.iter().position(|label| label == "Lamp ›").unwrap();
    let telegram = rows.iter().position(|label| label == "Telegram ›").unwrap();
    assert!(lamp < telegram, "{rows:?}");

    let page = press_screen(&panel, &find(&home, "Telegram")).await;
    assert!(
        text(&page).contains("The Telegram connector"),
        "{}",
        text(&page)
    );
    find(&page, "Mode: in_place");
}

#[tokio::test]
async fn toggles_turn_features_on_and_off() {
    let panel = panel().await;
    let features = Page::Setting(target(&panel, "disabled"));
    let screen = render(&panel, &features).await;

    let screen = press_screen(&panel, &find(&screen, "✅ Lamp")).await;
    assert_eq!(value(&panel, "disabled"), Some(json!(["lamp"])));
    assert!(text(&screen).starts_with("✅ Saved"), "{}", text(&screen));
    assert!(
        text(&screen).contains("changed from Telegram"),
        "{}",
        text(&screen)
    );

    let screen = press_screen(&panel, &find(&screen, "⬜ Lamp")).await;
    assert_eq!(value(&panel, "disabled"), Some(json!([])));

    // Back to the file's value.
    press(&panel, &find(&screen, "config file's value")).await;
    assert!(panel.store().current().overrides().is_empty());
}

#[tokio::test]
async fn choices_are_picked() {
    let panel = panel().await;
    let level = Page::Setting(target(&panel, "level"));
    let screen = render(&panel, &level).await;

    let screen = press_screen(&panel, &find(&screen, "debug (verbose)")).await;
    assert_eq!(panel.store().current().config.level, "debug");
    assert!(
        labels(&screen)
            .iter()
            .any(|label| label == "🔘 debug (verbose)")
    );
    assert_eq!(
        action(&find(&screen, "Other")),
        Action::Ask(target(&panel, "level"), AskKind::Value)
    );
}

#[tokio::test]
async fn map_entries_are_added_from_their_names_and_deleted() {
    let panel = panel().await;
    let access = target(&panel, "access");
    let screen = render(&panel, &Page::Setting(access.clone())).await;

    // A new entry, opened right away.
    let screen = press_screen(&panel, &find(&screen, "➕ Lamp")).await;
    assert_eq!(value(&panel, "access.lamp"), Some(json!([])));
    assert!(text(&screen).contains("Access › lamp"), "{}", text(&screen));
    assert_eq!(
        action(&find(&screen, "➕ Add")),
        Action::Ask(access.entry("lamp"), AskKind::Items)
    );

    panel
        .store()
        .extend("access.lamp", vec![json!(5)], None)
        .await
        .unwrap();
    let screen = render(&panel, &Page::Setting(access.entry("lamp"))).await;
    assert_eq!(
        action(&find(&screen, "❌ 5")),
        Action::Remove(access.entry("lamp"), json!(5))
    );

    let screen = press_screen(&panel, &find(&screen, "Delete entry")).await;
    assert!(panel.store().current().config.access.is_empty());
    assert!(
        text(&screen).contains("No entries yet"),
        "{}",
        text(&screen)
    );
}

#[tokio::test]
async fn known_users_and_chats_are_shown_by_name() {
    let panel = panel().await;
    for (key, value) in [("admins", json!([7, 8])), ("alerts_chat", json!(-100))] {
        panel.store().set(key, value, None).await.unwrap();
    }

    let admins = render(&panel, &Page::Setting(target(&panel, "admins"))).await;
    assert!(text(&admins).contains("Ann (@ann)"), "{}", text(&admins));
    assert_eq!(
        action(&find(&admins, "❌ Ann")),
        Action::Remove(target(&panel, "admins"), json!(7))
    );
    find(&admins, "❌ 8");

    let alerts = render(&panel, &Page::Setting(target(&panel, "alerts_chat"))).await;
    assert!(text(&alerts).contains("Now: Family"), "{}", text(&alerts));
}

#[tokio::test]
async fn references_of_unknown_domains_are_typed_and_listed() {
    let panel = panel().await;
    // A quoted id stays a string, even when it looks like a number.
    let things = target(&panel, "things");
    answer(
        &panel,
        things.clone(),
        AskKind::Items,
        typed("\"42\" a@b 42"),
    )
    .await
    .unwrap();
    assert_eq!(value(&panel, "things"), Some(json!(["42", "a@b", 42])));

    let screen = render(&panel, &Page::Setting(things.clone())).await;
    assert!(text(&screen).contains("a@b"), "{}", text(&screen));
    assert!(text(&screen).contains("42"), "{}", text(&screen));

    press(&panel, &find(&screen, "❌ a@b")).await;
    assert_eq!(value(&panel, "things"), Some(json!(["42", 42])));

    // ❌ removes the string, not the number that looks the same.
    let screen = render(&panel, &Page::Setting(things)).await;
    press(&panel, &find(&screen, "❌ 42")).await;
    assert_eq!(value(&panel, "things"), Some(json!([42])));
}

#[tokio::test]
async fn usernames_resolve_through_the_user_domain() {
    let panel = panel().await;
    answer(
        &panel,
        target(&panel, "admins"),
        AskKind::Items,
        typed("@ann"),
    )
    .await
    .unwrap();
    assert_eq!(value(&panel, "admins"), Some(json!([7])));
}

#[tokio::test]
async fn forms_are_edited_field_by_field() {
    let panel = panel().await;
    let presets = target(&panel, "features.lamp.presets");
    panel
        .store()
        .set(
            "features.lamp.presets.night",
            json!({ "brightness": 10, "white": "warm" }),
            None,
        )
        .await
        .unwrap();
    let night = presets.entry("night");
    let preset = |panel: &Panel<Toy>| value(panel, "features.lamp.presets.night");

    let screen = render(&panel, &Page::Setting(night.clone())).await;
    find(&screen, "Brightness: 10% ›");

    // A suggested number.
    let brightness = render(&panel, &Page::Setting(night.field(0))).await;
    press(&panel, &find(&brightness, "50%")).await;
    assert_eq!(
        preset(&panel),
        Some(json!({ "brightness": 50, "white": "warm" }))
    );

    // Fields of a group exclude each other.
    let colour = render(&panel, &Page::Setting(night.field(2))).await;
    assert!(text(&colour).contains("clears White"), "{}", text(&colour));
    press(&panel, &find(&colour, "red")).await;
    assert_eq!(
        preset(&panel),
        Some(json!({ "brightness": 50, "color": "red" }))
    );

    // Fields are cleared with 🚫 None.
    let colour = render(&panel, &Page::Setting(night.field(2))).await;
    press(&panel, &find(&colour, "None")).await;
    assert_eq!(preset(&panel), Some(json!({ "brightness": 50 })));
}

#[tokio::test]
async fn clearing_an_optional_text() {
    let panel = panel().await;
    let greeting = target(&panel, "greeting");
    panel
        .store()
        .set("greeting", json!("hi"), None)
        .await
        .unwrap();
    press(&panel, &Action::Clear(greeting).encode()).await;
    assert_eq!(panel.store().current().config.greeting, None);
}

#[tokio::test]
async fn reads_answers_by_kind() {
    let panel = panel().await;
    let edit = |reaction: Result<Reaction<Toy>, PanelError>| reaction.unwrap().change.is_some();

    let admins = target(&panel, "admins");
    assert!(edit(
        answer(&panel, admins.clone(), AskKind::Items, typed("12 @ann, 13")).await
    ));
    assert_eq!(value(&panel, "admins"), Some(json!([12, 7, 13])));
    let picked = Input::Picked {
        domain: crate::testing::USER.into(),
        ids: vec![json!(8), json!(9)],
    };
    assert!(edit(
        answer(&panel, admins.clone(), AskKind::Items, picked).await
    ));
    assert_eq!(value(&panel, "admins"), Some(json!([12, 7, 13, 8, 9])));
    assert!(matches!(
        answer(&panel, admins.clone(), AskKind::Items, typed("@bob")).await,
        Err(PanelError::Rejected(_))
    ));
    // A chat is not a user.
    let chat = Input::Picked {
        domain: crate::testing::CHAT.into(),
        ids: vec![json!(-100)],
    };
    assert!(
        answer(&panel, admins, AskKind::Items, chat.clone())
            .await
            .is_err()
    );

    let alerts = target(&panel, "alerts_chat");
    assert!(edit(
        answer(&panel, alerts.clone(), AskKind::Value, chat).await
    ));
    assert_eq!(value(&panel, "alerts_chat"), Some(json!(-100)));
    assert!(
        answer(&panel, alerts, AskKind::Value, typed("nope"))
            .await
            .is_err()
    );

    // Text is taken as typed, even when it looks like JSON.
    let greeting = target(&panel, "greeting");
    answer(&panel, greeting, AskKind::Value, typed(" true "))
        .await
        .unwrap();
    assert_eq!(value(&panel, "greeting"), Some(json!("true")));
}

#[tokio::test]
async fn reads_new_map_entries_and_form_fields() {
    let panel = panel().await;
    let presets = target(&panel, "features.lamp.presets");

    // A form's entries start from its initial value, and are opened.
    let reaction = answer(&panel, presets.clone(), AskKind::Entry, typed("cozy"))
        .await
        .unwrap();
    assert_eq!(
        value(&panel, "features.lamp.presets.cozy"),
        Some(json!({ "brightness": 100 }))
    );
    let screen = reaction.screen.unwrap();
    assert!(
        text(&screen).contains("Presets › cozy"),
        "{}",
        text(&screen)
    );

    for invalid in ["a.b", "a:b", "a#b", &"x".repeat(40)] {
        assert!(
            answer(&panel, presets.clone(), AskKind::Entry, typed(invalid))
                .await
                .is_err(),
            "{invalid}"
        );
    }

    // Other maps take the name, then the value.
    let notes = target(&panel, "features.lamp.notes");
    answer(
        &panel,
        notes.clone(),
        AskKind::Entry,
        typed("door \"shut\""),
    )
    .await
    .unwrap();
    assert_eq!(
        value(&panel, "features.lamp.notes.door"),
        Some(json!("shut"))
    );
    assert!(
        answer(&panel, notes, AskKind::Entry, typed("door"))
            .await
            .is_err()
    );

    // A number field, checked against its range.
    let brightness = presets.entry("cozy").field(0);
    panel
        .store()
        .set(
            "features.lamp.presets.cozy",
            json!({ "white": "warm" }),
            None,
        )
        .await
        .unwrap();
    answer(&panel, brightness.clone(), AskKind::Value, typed("40%"))
        .await
        .unwrap();
    assert_eq!(
        value(&panel, "features.lamp.presets.cozy"),
        Some(json!({ "brightness": 40, "white": "warm" }))
    );
    assert!(
        answer(&panel, brightness, AskKind::Value, typed("400"))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn questions_name_the_setting_and_offer_pickers() {
    let panel = panel().await;
    let admins = target(&panel, "admins");
    let screen = render(&panel, &Page::Setting(admins.clone())).await;
    let reaction = press(&panel, &find(&screen, "➕ Add")).await;
    let ask = reaction.ask.unwrap();
    assert_eq!(
        ask.question,
        Question {
            target: admins,
            ask: AskKind::Items
        }
    );
    assert_eq!(
        ask.text.to_plain(),
        "Pick users with the buttons below, or send the ids or @usernames (of people who have \
         talked to me) to add to Admins."
    );
    assert_eq!(ask.picker.unwrap().domain, crate::testing::USER);
    assert_eq!(
        reaction.notice.as_deref(),
        Some("✏️ Waiting for your answer")
    );
    assert_eq!(Question::parse(&ask.question.encode()), Some(ask.question));
}

#[tokio::test]
async fn tokens_fit_the_limit() {
    let panel = panel().await;
    panel
        .store()
        .set("notes", json!({ "x".repeat(40): "long" }), None)
        .await
        .unwrap();
    let limits = Limits {
        token_bytes: 20,
        ..LIMITS
    };
    let view = viewer(&FakeNames, &limits);
    let catalog = panel.store().catalog();
    for (position, _) in catalog.live() {
        let page = Page::Setting(Target::setting(position));
        let wide = panel.render(&page, &viewer(&FakeNames, &LIMITS)).await;
        let narrow = panel.render(&page, &view).await;
        assert_eq!(labels(&wide), labels(&narrow), "{page:?}");
        for (_, token) in buttons(&narrow) {
            assert!(token.len() <= 20, "{token}");
        }
    }
}

#[tokio::test]
async fn long_entry_names_get_buttons_through_handles() {
    let panel = panel().await;
    let name = "n".repeat(60);
    panel
        .store()
        .set("notes", json!({ &name: "long" }), None)
        .await
        .unwrap();
    let notes = target(&panel, "notes");
    let screen = render(&panel, &Page::Setting(notes.clone())).await;
    let token = find(&screen, &name[..20]);
    assert!(
        token.starts_with('~') && token.len() == HANDLE_LEN,
        "{token}"
    );

    let opened = press_screen(&panel, &token).await;
    assert!(
        text(&opened).contains(&format!("Notes › {name}")),
        "{}",
        text(&opened)
    );
}

#[tokio::test]
async fn short_tokens_are_unchanged() {
    let panel = panel().await;
    let home = render(&panel, &Page::Home).await;
    assert_eq!(find(&home, "Features"), "o:s:0");
    assert_eq!(find(&home, "Lamp"), "o:m:lamp");
    assert_eq!(find(&home, "Reload"), "R");
    assert_eq!(find(&home, "Close"), "x");

    let features = render(&panel, &Page::Setting(target(&panel, "disabled"))).await;
    assert_eq!(find(&features, "Lamp"), "t:0:0");
    assert_eq!(find(&features, "Fan"), "t:0:1");
    assert_eq!(find(&features, "Back"), "o:h");

    let level = render(&panel, &Page::Setting(target(&panel, "level"))).await;
    assert_eq!(find(&level, "debug"), "p:3:1");
    assert_eq!(find(&level, "Other"), "a:3:v");

    panel.store().set("admins", json!([7]), None).await.unwrap();
    let admins = render(&panel, &Page::Setting(target(&panel, "admins"))).await;
    assert_eq!(find(&admins, "❌ Ann"), "r:2:7");
}

#[tokio::test]
async fn dynamic_picks_bind_the_value() {
    let panel = panel().await;
    *FLAVOURS.lock().unwrap() = vec!["a", "b", "c"];
    let flavour = Page::Setting(target(&panel, "flavour"));
    let screen = render(&panel, &flavour).await;
    let b = find(&screen, "⚪ b");

    // The list changes behind the drawn buttons.
    *FLAVOURS.lock().unwrap() = vec!["c", "b"];
    press(&panel, &b).await;
    assert_eq!(value(&panel, "flavour"), Some(json!("b")));

    // A value that is no longer offered: the list is redrawn.
    let a = find(&screen, "⚪ a");
    let stale = panel.press(&a, &viewer(&FakeNames, &LIMITS)).await;
    assert!(
        matches!(&stale, Err(PanelError::Stale { notice, .. }) if notice == "This list changed"),
        "{stale:?}"
    );
    assert_eq!(value(&panel, "flavour"), Some(json!("b")));
}

#[tokio::test]
async fn removing_an_item_removes_that_value() {
    let panel = panel().await;
    panel
        .store()
        .set("admins", json!([5, 6, 7]), None)
        .await
        .unwrap();
    let admins = target(&panel, "admins");
    let screen = render(&panel, &Page::Setting(admins)).await;
    let six = find(&screen, "❌ 6");

    // Another item goes first, moving 6.
    panel
        .store()
        .remove("admins", json!(5), None)
        .await
        .unwrap();
    press(&panel, &six).await;
    assert_eq!(value(&panel, "admins"), Some(json!([7])));

    let again = panel.press(&six, &viewer(&FakeNames, &LIMITS)).await;
    match again {
        Err(PanelError::Stale { notice, screen }) => {
            assert_eq!(notice, "This list changed");
            assert!(text(&screen).contains("Admins"), "{}", text(&screen));
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(value(&panel, "admins"), Some(json!([7])));
}

#[tokio::test]
async fn expired_handles_are_stale() {
    let now = Arc::new(AtomicI64::new(0));
    let clock = Arc::clone(&now);
    let panel = panel()
        .await
        .token_store(MemoryTokenStore::new())
        .token_ttl(std::time::Duration::from_secs(60))
        .clock(move || clock.load(Ordering::Relaxed));
    let name = "e".repeat(60);
    panel
        .store()
        .set("notes", json!({ &name: "long" }), None)
        .await
        .unwrap();
    let screen = render(&panel, &Page::Setting(target(&panel, "notes"))).await;
    let token = find(&screen, &name[..20]);

    now.store(2 * 60 * 1000, Ordering::Relaxed);
    for token in [token.as_str(), "~0000000000"] {
        match panel.press(token, &viewer(&FakeNames, &LIMITS)).await {
            Err(PanelError::Stale { notice, screen }) => {
                assert_eq!(notice, "This panel is out of date");
                assert!(text(&screen).contains("Settings"), "{}", text(&screen));
            }
            other => panic!("{other:?}"),
        }
    }
}

#[tokio::test]
async fn screens_are_paged_when_rows_are_limited() {
    let panel = panel().await;
    let limits = Limits {
        max_rows: Some(5),
        max_per_row: 5,
        ..LIMITS
    };
    let view = viewer(&FakeNames, &limits);
    // The top-level settings make more rows than fit.
    let first = panel.render(&Page::Home, &view).await;
    assert_eq!(first.rows.len(), 5);
    let nav = first.rows.last().unwrap();
    let nav_labels: Vec<_> = nav.iter().map(|button| button.label.as_str()).collect();
    assert_eq!(nav_labels, ["🔄 Reload", "✖️ Close", "▶"]);

    let next = nav.last().unwrap().token.clone();
    let second = panel.press(&next, &view).await.unwrap().screen.unwrap();
    let nav = second.rows.last().unwrap();
    assert_eq!(nav[0].label, "◀");
    let all: Vec<_> = [&first, &second]
        .iter()
        .flat_map(|screen| labels(screen))
        .collect();
    assert!(
        all.iter().any(|label| label.starts_with("Features")),
        "{all:?}"
    );
    assert!(
        all.iter().any(|label| label.starts_with("Admins")),
        "{all:?}"
    );

    // Without a limit, one screen holds everything.
    let whole = render(&panel, &Page::Home).await;
    assert!(whole.rows.len() > 5);
}

#[tokio::test]
async fn ref_items_offer_candidates() {
    let panel = panel().await;
    panel
        .store()
        .set("rooms", json!(["r1"]), None)
        .await
        .unwrap();
    let rooms = target(&panel, "rooms");
    let screen = render(&panel, &Page::Setting(rooms.clone())).await;
    let ask = press(&panel, &find(&screen, "➕ Add")).await.ask.unwrap();
    let suggested: Vec<_> = ask
        .rows
        .iter()
        .flatten()
        .map(|b| b.label.as_str())
        .collect();
    // Kitchen (r1) is listed already.
    assert_eq!(suggested, ["➕ Garden", "➕ Garage"]);

    let garden = ask.rows[0][0].token.clone();
    press(&panel, &garden).await;
    assert_eq!(value(&panel, "rooms"), Some(json!(["r1", "r2"])));

    // A typed name that matches one suggestion adds it; an ambiguous one is
    // taken as typed.
    answer(&panel, rooms.clone(), AskKind::Items, typed("garage"))
        .await
        .unwrap();
    assert_eq!(value(&panel, "rooms"), Some(json!(["r1", "r2", "r3"])));
    answer(&panel, rooms, AskKind::Items, typed("ga"))
        .await
        .unwrap();
    assert_eq!(
        value(&panel, "rooms"),
        Some(json!(["r1", "r2", "r3", "ga"]))
    );
}

#[tokio::test]
async fn closing_and_posting() {
    let panel = panel().await;
    let closed = press(&panel, "x").await;
    assert!(closed.closed);
    assert_eq!(
        closed.screen.unwrap().text.to_plain(),
        "Settings closed; send /config to open them again."
    );

    let token = panel
        .post_token(&Page::Section("lamp".into()), &LIMITS)
        .unwrap();
    let posted = press(&panel, &token).await.post.unwrap();
    assert!(text(&posted).contains("The lamp"), "{}", text(&posted));

    assert!(matches!(
        panel.press("nonsense", &viewer(&FakeNames, &LIMITS)).await,
        Err(PanelError::Rejected(_))
    ));
}
