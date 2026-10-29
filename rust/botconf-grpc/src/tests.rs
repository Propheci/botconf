//! The service, served in process on a local port and driven by the
//! generated client, over a toy schema.

use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use botconf::{
    Choice, Choices, DynamicChoices, DynamicSection, Field, FixedChoice, Form, Kind, MemoryStorage,
    RuntimeSetting, Schema, SettingsStore, Storage, StorageError, StoredOverride, View,
};
use botconf_panel::{MemoryTokenStore, Panel};
use figment::{
    Figment,
    providers::{Format, Toml},
};
use futures::future::BoxFuture;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio_stream::{StreamExt, wrappers::TcpListenerStream};
use tonic::{
    Code, Status,
    metadata::MetadataMap,
    transport::{Channel, Server},
};

use crate::{
    Authorizer, Operation, Principal, SettingsService,
    proto::{self, settings_client::SettingsClient},
};

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Config {
    #[serde(default)]
    greeting: Option<String>,
    #[serde(default = "five")]
    limit: i64,
    #[serde(default)]
    disabled: Vec<String>,
    #[serde(default)]
    admins: Vec<u64>,
    #[serde(default)]
    flavour: Option<String>,
    #[serde(default)]
    presets: BTreeMap<String, Value>,
    #[serde(default)]
    notes: BTreeMap<String, String>,
}

fn five() -> i64 {
    5
}

struct Toy;

const FEATURES: &[FixedChoice] = &[
    FixedChoice::new("lamp", "Lamp"),
    FixedChoice::new("fan", "Fan"),
];
const PRESET_FIELDS: &[Field] = &[Field::new(
    "brightness",
    "Brightness",
    Kind::number(1, 100, "%", &[50, 100], true),
)];
const PRESET_FORM: &Form = &Form::new(PRESET_FIELDS, r#"{"brightness": 100}"#);
const PRESET: &Kind = &Kind::form(PRESET_FORM);
const NOTE: &Kind = &Kind::text(false);

fn flavours(_: &dyn View) -> Vec<Choice> {
    vec![
        Choice::new("mint", "🌿 mint"),
        Choice::new("lime", "🍋 lime"),
    ]
}

impl Schema for Toy {
    type Config = Config;
    type Derived = ();

    fn derive(&self, config: &Config) -> Result<(), String> {
        if (1..=10).contains(&config.limit) {
            Ok(())
        } else {
            Err(format!(
                "`limit` must be from 1 to 10, not {}",
                config.limit
            ))
        }
    }

    fn settings(&self) -> Vec<RuntimeSetting> {
        vec![
            RuntimeSetting::new("greeting", "What to say").kind(Kind::text(true)),
            RuntimeSetting::new("limit", "How many at once").kind(Kind::number(
                1,
                10,
                "",
                &[1, 5],
                false,
            )),
            RuntimeSetting::new("disabled", "Features that are off")
                .titled("Features")
                .kind(Kind::set_of(Choices::fixed(FEATURES), true)),
            RuntimeSetting::new("admins", "Who's in charge")
                .kind(Kind::reference("telegram.user", true)),
            RuntimeSetting::new("flavour", "Today's flavour").kind(Kind::one_of(
                Choices::Dynamic(DynamicChoices(flavours)),
                false,
                true,
            )),
            RuntimeSetting::per_entry("presets", "Named looks", PRESET),
            RuntimeSetting::per_entry("notes", "Notes by topic", NOTE),
        ]
    }

    fn dynamic_namespaces(&self) -> Vec<&'static str> {
        vec!["connectors"]
    }
}

/// Records every decision, maps the claimed actor to `tg:<id>`, and refuses
/// changes when told to.
#[derive(Clone, Default)]
struct Recording {
    seen: Arc<Mutex<Vec<String>>>,
    refuse_changes: bool,
}

impl Authorizer for Recording {
    fn authorize(
        &self,
        _metadata: &MetadataMap,
        actor: Option<&proto::Actor>,
        operation: Operation<'_>,
    ) -> Result<Principal, Status> {
        self.seen.lock().unwrap().push(format!("{operation:?}"));
        if self.refuse_changes && matches!(operation, Operation::Change { .. }) {
            return Err(Status::permission_denied("not an owner"));
        }
        Ok(Principal {
            actor: actor.map(|actor| format!("tg:{}", actor.id)),
        })
    }
}

/// Fails every write.
struct BrokenStorage;

impl Storage for BrokenStorage {
    fn load(&self) -> BoxFuture<'_, Result<Vec<StoredOverride>, StorageError>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn write<'a>(
        &'a self,
        _deleted: &'a [String],
        _upserted: Option<&'a StoredOverride>,
    ) -> BoxFuture<'a, Result<(), StorageError>> {
        Box::pin(async { Err("disk full".into()) })
    }
}

async fn store(storage: impl Storage) -> Arc<SettingsStore<Toy>> {
    let base = Figment::from(Toml::string("limit = 5"));
    Arc::new(
        SettingsStore::load(Toy, base, storage)
            .await
            .expect("valid"),
    )
}

/// Serves `service` on a local port and returns a client.
async fn serve(service: SettingsService<Toy>) -> SettingsClient<Channel> {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(
        Server::builder()
            .add_service(service.into_server())
            .serve_with_incoming(TcpListenerStream::new(listener)),
    );
    SettingsClient::connect(format!("http://{address}"))
        .await
        .unwrap()
}

/// A client of a fresh service over a memory store, with the recording
/// authorizer.
async fn client() -> (
    SettingsClient<Channel>,
    Arc<SettingsStore<Toy>>,
    Arc<MemoryStorage>,
    Recording,
) {
    let storage = Arc::new(MemoryStorage::new());
    let store = store(Arc::clone(&storage)).await;
    let recording = Recording::default();
    let panel = Arc::new(Panel::new(Arc::clone(&store)));
    let client = serve(SettingsService::new(panel, recording.clone())).await;
    (client, store, storage, recording)
}

fn actor(id: &str) -> Option<proto::Actor> {
    Some(proto::Actor {
        domain: "telegram.user".into(),
        id: id.into(),
    })
}

fn edit(key: &str, value: &str) -> proto::EditRequest {
    proto::EditRequest {
        actor: actor("42"),
        key: key.into(),
        value: value.into(),
    }
}

fn position(store: &SettingsStore<Toy>, key: &str) -> u32 {
    let catalog = store.catalog();
    let position = catalog
        .live()
        .find(|(_, entry)| entry.key == key)
        .unwrap()
        .0;
    u32::try_from(position).unwrap()
}

/// The token of the first button whose label contains `label`.
fn find(screen: &proto::Screen, label: &str) -> String {
    let buttons: Vec<_> = screen.rows.iter().flat_map(|row| &row.buttons).collect();
    buttons
        .iter()
        .find(|button| button.label.contains(label))
        .unwrap_or_else(|| {
            let labels: Vec<_> = buttons.iter().map(|button| &button.label).collect();
            panic!("no {label:?} in {labels:?}")
        })
        .token
        .clone()
}

fn plain(text: Option<&proto::Text>) -> String {
    text.map(|text| text.spans.iter().map(|span| span.text.as_str()).collect())
        .unwrap_or_default()
}

fn press(token: String) -> proto::ActRequest {
    proto::ActRequest {
        actor: actor("42"),
        act: Some(proto::act_request::Act::Press(token)),
        ..Default::default()
    }
}

async fn home(client: &mut SettingsClient<Channel>) -> proto::Screen {
    client
        .render(proto::RenderRequest {
            actor: actor("42"),
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner()
        .screen
        .unwrap()
}

#[tokio::test]
async fn describe_resolves_choices() {
    let (mut client, store, _, _) = client().await;
    let described = client
        .describe(proto::DescribeRequest { actor: None })
        .await
        .unwrap()
        .into_inner();

    let flavour = described
        .settings
        .iter()
        .find(|setting| setting.key == "flavour")
        .unwrap();
    assert_eq!(flavour.position, position(&store, "flavour"));
    let Some(proto::kind::Kind::OneOf(one_of)) = &flavour.kind.as_ref().unwrap().kind else {
        panic!("{flavour:?}");
    };
    let values: Vec<_> = one_of.choices.iter().map(|choice| &choice.value).collect();
    assert_eq!(values, ["mint", "lime"]);

    let positions: Vec<_> = described
        .settings
        .iter()
        .map(|setting| setting.position)
        .collect();
    let live: Vec<_> = store
        .catalog()
        .live()
        .map(|(position, _)| u32::try_from(position).unwrap())
        .collect();
    assert_eq!(positions, live);
}

#[tokio::test]
async fn set_validates_like_the_store() {
    let (mut client, store, storage, _) = client().await;
    let status = client.set(edit("limit", "42")).await.unwrap_err();
    assert_eq!(status.code(), Code::InvalidArgument);
    assert!(status.message().contains("from 1 to 10"), "{status:?}");
    assert!(store.current().overrides().is_empty());
    assert!(storage.load().await.unwrap().is_empty());

    let changed = client.set(edit("limit", "7")).await.unwrap().into_inner();
    assert!(changed.changed);
    assert_eq!(changed.previous_json.as_deref(), Some("5"));
    let current = changed.current.unwrap();
    assert_eq!(current.value_json.as_deref(), Some("7"));
    assert_eq!(current.source(), proto::Source::Stored);
}

#[tokio::test]
async fn set_records_the_principal() {
    let (mut client, _, storage, recording) = client().await;
    client.set(edit("greeting", "hello")).await.unwrap();
    let stored = storage.load().await.unwrap();
    assert_eq!(stored[0].by.as_deref(), Some("tg:42"));
    assert_eq!(stored[0].value, r#""hello""#);
    assert_eq!(
        recording.seen.lock().unwrap().as_slice(),
        [r#"Change { key: "greeting" }"#]
    );
}

#[tokio::test]
async fn refused_requests_change_nothing() {
    let storage = Arc::new(MemoryStorage::new());
    let store = store(Arc::clone(&storage)).await;
    let refusing = Recording {
        refuse_changes: true,
        ..Recording::default()
    };
    let panel = Arc::new(Panel::new(Arc::clone(&store)));
    let mut client = serve(SettingsService::new(panel, refusing)).await;

    for status in [
        client.set(edit("limit", "7")).await.unwrap_err(),
        client.add(edit("admins", "1")).await.unwrap_err(),
        client
            .unset(proto::UnsetRequest {
                actor: actor("42"),
                key: "limit".into(),
            })
            .await
            .unwrap_err(),
    ] {
        assert_eq!(status.code(), Code::PermissionDenied);
    }
    assert!(store.current().overrides().is_empty());
    assert!(storage.load().await.unwrap().is_empty());
}

#[tokio::test]
async fn unknown_keys_are_not_found() {
    let (mut client, _, _, _) = client().await;
    let status = client.set(edit("token", "x")).await.unwrap_err();
    assert_eq!(status.code(), Code::NotFound);
    let status = client
        .get(proto::GetRequest {
            actor: None,
            key: "token".into(),
        })
        .await
        .unwrap_err();
    assert_eq!(status.code(), Code::NotFound);
}

#[tokio::test]
async fn unset_without_override_reports_unchanged() {
    let (mut client, _, _, _) = client().await;
    let unset = |key: &str| proto::UnsetRequest {
        actor: actor("42"),
        key: key.into(),
    };
    let response = client.unset(unset("limit")).await.unwrap().into_inner();
    assert!(!response.changed);
    assert_eq!(response.current.unwrap().value_json.as_deref(), Some("5"));

    client.set(edit("limit", "7")).await.unwrap();
    let response = client.unset(unset("limit")).await.unwrap().into_inner();
    assert!(response.changed);
    assert_eq!(response.previous_json.as_deref(), Some("7"));
    assert_eq!(response.current.unwrap().value_json.as_deref(), Some("5"));
}

#[tokio::test]
async fn render_and_press_toggle_a_choice() {
    let (mut client, store, _, _) = client().await;
    let screen = home(&mut client).await;
    let features = client
        .act(press(find(&screen, "Features")))
        .await
        .unwrap()
        .into_inner()
        .screen
        .expect("the features screen");

    let toggled = client
        .act(press(find(&features, "✅ Lamp")))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(store.current().config.disabled, ["lamp"]);
    let redrawn = toggled.screen.expect("a redrawn screen");
    find(&redrawn, "⬜ Lamp");
    assert_eq!(
        store.current().overrides().keys().collect::<Vec<_>>(),
        ["disabled"]
    );
}

#[tokio::test]
async fn ask_and_answer_round_trip() {
    let (mut client, store, storage, _) = client().await;
    let screen = home(&mut client).await;
    let greeting = client
        .act(press(find(&screen, "Greeting")))
        .await
        .unwrap()
        .into_inner()
        .screen
        .unwrap();
    let asked = client
        .act(press(find(&greeting, "Change")))
        .await
        .unwrap()
        .into_inner()
        .ask
        .expect("a question");
    assert_eq!(asked.title, "Greeting");

    let answered = client
        .act(proto::ActRequest {
            actor: actor("42"),
            act: Some(proto::act_request::Act::Answer(proto::Answer {
                question: asked.question,
                input: Some(proto::answer::Input::Text("hello".into())),
            })),
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner();
    assert!(answered.screen.is_some());
    assert_eq!(store.current().config.greeting.as_deref(), Some("hello"));
    assert_eq!(
        storage.load().await.unwrap()[0].by.as_deref(),
        Some("tg:42")
    );

    let status = client
        .act(proto::ActRequest {
            act: Some(proto::act_request::Act::Answer(proto::Answer {
                question: "nonsense".into(),
                input: Some(proto::answer::Input::Text("x".into())),
            })),
            ..Default::default()
        })
        .await
        .unwrap_err();
    assert_eq!(status.code(), Code::InvalidArgument);
}

#[tokio::test]
async fn rejected_presses_come_back_in_the_response() {
    let (mut client, _, _, _) = client().await;
    let response = client
        .act(press("garbage".into()))
        .await
        .unwrap()
        .into_inner();
    assert!(response.rejected.is_some(), "{response:?}");
    assert!(response.screen.is_none());
}

#[tokio::test]
async fn text_commands_reply() {
    let (mut client, _, _, _) = client().await;
    let command = |args: &str| proto::ActRequest {
        actor: actor("42"),
        act: Some(proto::act_request::Act::Command(args.into())),
        ..Default::default()
    };
    let reply = client
        .act(command("get limit"))
        .await
        .unwrap()
        .into_inner()
        .reply;
    assert!(plain(reply.as_ref()).contains('5'), "{reply:?}");

    let opened = client.act(command("")).await.unwrap().into_inner();
    assert!(opened.post.is_some(), "no arguments post the panel");
}

#[tokio::test]
async fn watch_sends_initial_values_then_changes() {
    let (mut client, store, _, _) = client().await;
    let mut events = client
        .watch(proto::WatchRequest {
            actor: None,
            prefixes: vec!["greeting".into()],
        })
        .await
        .unwrap()
        .into_inner();

    let initial = events.next().await.unwrap().unwrap();
    assert!(initial.initial);
    let keys: Vec<_> = initial.values.iter().map(|value| &value.key).collect();
    assert_eq!(keys, ["greeting"]);
    assert_eq!(initial.values[0].value_json.as_deref(), Some("null"));

    // A change elsewhere sends nothing; the next event is the greeting's.
    store.set("limit", json!(7), None).await.unwrap();
    store.set("greeting", json!("hello"), None).await.unwrap();
    let changed = events.next().await.unwrap().unwrap();
    assert!(!changed.initial);
    assert_eq!(changed.values.len(), 1);
    assert_eq!(changed.values[0].key, "greeting");
    assert_eq!(changed.values[0].value_json.as_deref(), Some(r#""hello""#));
    assert_eq!(changed.values[0].source(), proto::Source::Stored);
}

fn connector(id: &str) -> DynamicSection {
    DynamicSection {
        id: format!("connector:{id}"),
        path: format!("connectors.{id}"),
        title: id.into(),
        description: format!("The {id} connector"),
        settings: vec![RuntimeSetting::owned(
            "rendering",
            "How messages look",
            Kind::Json,
        )],
        validator: Arc::new(|section: &Value| Ok(section.clone())),
        base: json!({ "rendering": { "edit_mode": "in_place" } }),
        base_source: format!("connector {id}"),
    }
}

#[tokio::test]
async fn watch_reports_dynamic_sections() {
    let (mut client, store, _, _) = client().await;
    let mut events = client
        .watch(proto::WatchRequest {
            actor: None,
            prefixes: vec!["connectors.tg".into()],
        })
        .await
        .unwrap()
        .into_inner();
    let initial = events.next().await.unwrap().unwrap();
    assert!(initial.values.is_empty());

    store.register_section(connector("tg")).await.unwrap();
    let registered = events.next().await.unwrap().unwrap();
    assert!(registered.catalog_changed);
    assert_eq!(registered.values.len(), 1);
    let rendering = &registered.values[0];
    assert_eq!(rendering.key, "connectors.tg.rendering");
    assert_eq!(
        rendering.value_json.as_deref(),
        Some(r#"{"edit_mode":"in_place"}"#)
    );
    assert_eq!(rendering.source(), proto::Source::Other);
    assert_eq!(rendering.source_name, "connector tg");

    // Another connector's section is not watched.
    store.register_section(connector("wa")).await.unwrap();
    store.remove_section("connector:tg").await.unwrap();
    let removed = events.next().await.unwrap().unwrap();
    assert!(removed.catalog_changed);
    assert_eq!(removed.values[0].key, "connectors.tg.rendering");
    assert_eq!(removed.values[0].value_json, None);

    let described = client
        .describe(proto::DescribeRequest { actor: None })
        .await
        .unwrap()
        .into_inner();
    let dynamic: Vec<_> = described
        .sections
        .iter()
        .filter(|section| section.dynamic)
        .map(|section| section.id.as_str())
        .collect();
    assert_eq!(dynamic, ["connector:wa"]);
}

#[tokio::test]
async fn storage_failures_are_internal() {
    let store = store(BrokenStorage).await;
    let panel = Arc::new(Panel::new(Arc::clone(&store)));
    let mut client = serve(SettingsService::new(panel, Recording::default())).await;
    let status = client.set(edit("limit", "7")).await.unwrap_err();
    assert_eq!(status.code(), Code::Internal);
    assert!(!status.message().contains("disk full"));
}

/// A service over `store` whose panel keeps its handles in `tokens`.
async fn served_with(
    store: &Arc<SettingsStore<Toy>>,
    tokens: Arc<MemoryTokenStore>,
) -> SettingsClient<Channel> {
    let panel = Arc::new(Panel::new(Arc::clone(store)).token_store(tokens));
    serve(SettingsService::new(panel, Recording::default())).await
}

/// The token of a button that needs a handle: a note with a long name.
async fn long_note_token(client: &mut SettingsClient<Channel>) -> String {
    let screen = home(client).await;
    let notes = client
        .act(press(find(&screen, "Notes")))
        .await
        .unwrap()
        .into_inner()
        .screen
        .unwrap();
    let token = find(&notes, &"n".repeat(20));
    assert!(token.starts_with('~'), "{token}");
    token
}

#[tokio::test]
async fn stale_handles_come_back_as_stale() {
    let store = store(MemoryStorage::new()).await;
    store
        .set("notes", json!({ "n".repeat(60): "long" }), None)
        .await
        .unwrap();
    let mut first = served_with(&store, Arc::default()).await;
    let token = long_note_token(&mut first).await;

    // A restart without a persistent token store.
    let mut second = served_with(&store, Arc::default()).await;
    let response = second.act(press(token)).await.unwrap().into_inner();
    assert!(response.stale, "{response:?}");
    assert!(response.notice.is_some());
    assert!(response.rejected.is_none());
    let screen = response.screen.expect("the home screen");
    find(&screen, "Notes");
}

#[tokio::test]
async fn handles_survive_a_new_service_with_the_same_store() {
    let store = store(MemoryStorage::new()).await;
    store
        .set("notes", json!({ "n".repeat(60): "long" }), None)
        .await
        .unwrap();
    let tokens = Arc::new(MemoryTokenStore::new());
    let mut first = served_with(&store, Arc::clone(&tokens)).await;
    let token = long_note_token(&mut first).await;

    let mut second = served_with(&store, tokens).await;
    let response = second.act(press(token)).await.unwrap().into_inner();
    assert!(!response.stale, "{response:?}");
    let screen = response.screen.expect("the note's screen");
    assert!(
        plain(screen.text.as_ref()).contains(&"n".repeat(60)),
        "{screen:?}"
    );
}

#[tokio::test]
async fn changes_through_the_service_reach_the_hook() {
    let store = store(MemoryStorage::new()).await;
    let heard = Arc::new(Mutex::new(Vec::new()));
    let hook = Arc::clone(&heard);
    let panel = Arc::new(Panel::new(Arc::clone(&store)));
    let service = SettingsService::new(panel, Recording::default()).on_change(move |change| {
        hook.lock().unwrap().push(change.current.config.limit);
    });
    let mut client = serve(service).await;

    client.set(edit("limit", "7")).await.unwrap();
    client
        .get(proto::GetRequest {
            actor: None,
            key: "limit".into(),
        })
        .await
        .unwrap();
    client
        .act(proto::ActRequest {
            act: Some(proto::act_request::Act::Command("set limit 8".into())),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(heard.lock().unwrap().as_slice(), [7, 8]);
}
