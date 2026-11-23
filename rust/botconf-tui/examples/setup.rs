//! Try the setup screen on a sample program:
//!
//! ```sh
//! cargo run -p botconf-tui --example setup -- /tmp/sample.toml /tmp/sample.env
//! ```

use std::collections::BTreeMap;

use botconf::{Describe, describe::Description};
use serde::Deserialize;

/// A sample bot's configuration.
#[derive(Describe, Deserialize)]
#[allow(dead_code)]
struct Config {
    /// Where the bot's API listens.
    #[botconf(example = "127.0.0.1:8080")]
    listen: String,
    /// The environment variable holding the token its clients present.
    #[botconf(env = token, hint = "SAMPLE_TOKEN")]
    token_env: Option<String>,
    /// How much it logs.
    #[serde(default)]
    #[botconf(runtime)]
    level: Level,
    /// Longer replies are sent as a file; 0 means no limit.
    #[serde(default)]
    #[botconf(runtime, max = 100_000)]
    max_reply_length: u32,
    /// The chats it answers in.
    #[serde(default)]
    chats: Vec<Chat>,
    /// Forwarding rules by name.
    #[serde(default)]
    rules: BTreeMap<String, Rule>,
}

#[derive(Default, Describe, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Level {
    #[default]
    Info,
    Debug,
    Trace,
}

#[derive(Describe, Deserialize)]
#[allow(dead_code)]
struct Chat {
    /// The chat's id.
    id: String,
    /// Answer only these users (empty: everyone).
    #[serde(default)]
    users: Vec<String>,
}

#[derive(Describe, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
#[allow(dead_code)]
enum Rule {
    /// Forward one chat to another.
    Forward { from: String, to: String },
    /// Drop what a chat sends.
    Mute { chat: String },
}

fn main() -> std::io::Result<()> {
    let mut arguments = std::env::args().skip(1);
    let config = arguments.next().unwrap_or_else(|| "sample.toml".into());
    let env_file = arguments.next().unwrap_or_else(|| "sample.env".into());
    botconf_tui::run(botconf_tui::Setup {
        description: Description::of::<Config>("Sample bot", "A sample bot's settings."),
        config: config.into(),
        env_file: env_file.into(),
        validate: Box::new(|candidate| {
            toml::from_str::<Config>(candidate.config)
                .map(drop)
                .map_err(|error| error.message().to_owned())
        }),
    })
}
