//! A settings server over a toy schema, for clients' cross-language tests
//! (the Go client's `integration` tests start it). Every request is allowed.
//!
//! ```sh
//! cargo run -p botconf-grpc --features testing --example toy_server -- --listen 127.0.0.1:0
//! ```
//!
//! It prints `listening on <address>` once it accepts connections.

use std::{io::Write, sync::Arc};

use botconf::{Choices, FixedChoice, Kind, MemoryStorage, RuntimeSetting, Schema, SettingsStore};
use botconf_grpc::{AllowAll, SettingsService};
use botconf_panel::Panel;
use figment::{
    Figment,
    providers::{Format, Toml},
};
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tokio_stream::wrappers::TcpListenerStream;
use tonic::transport::Server;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Config {
    #[serde(default)]
    greeting: Option<String>,
    limit: i64,
    #[serde(default)]
    disabled: Vec<String>,
}

struct Toy;

const FEATURES: &[FixedChoice] = &[
    FixedChoice::new("lamp", "Lamp"),
    FixedChoice::new("fan", "Fan"),
];

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
        ]
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let mut listen = "127.0.0.1:0".to_string();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--listen" => listen = args.next().ok_or("--listen needs an address")?,
            other => return Err(format!("unknown argument {other:?}").into()),
        }
    }

    let base = Figment::from(Toml::string("limit = 5"));
    let store = Arc::new(SettingsStore::load(Toy, base, MemoryStorage::new()).await?);
    let service = SettingsService::new(Arc::new(Panel::new(store)), AllowAll);

    let listener = TcpListener::bind(&listen).await?;
    println!("listening on {}", listener.local_addr()?);
    std::io::stdout().flush()?;

    Server::builder()
        .add_service(service.into_server())
        .serve_with_incoming(TcpListenerStream::new(listener))
        .await?;
    Ok(())
}
