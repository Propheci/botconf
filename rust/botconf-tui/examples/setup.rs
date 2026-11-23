//! Try the setup screen on a sample program:
//!
//! ```sh
//! cargo run -p botconf-tui --example setup -- /tmp/sample.toml /tmp/sample.env
//! ```

use std::collections::BTreeMap;

use botconf::{Describe, describe::Description};
use botconf_tui::{Setup, Suggestion};
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
    #[botconf(domain = "sample.chat")]
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
    Forward {
        #[botconf(domain = "sample.chat")]
        from: String,
        #[botconf(domain = "sample.chat")]
        to: String,
    },
    /// Drop what a chat sends.
    Mute { chat: String },
}

/// The chats a "linked account" of the sample knows.
const CHATS: [(&str, &str); 4] = [
    ("chat:100", "Family"),
    ("chat:200", "Work"),
    ("chat:300", "Football club"),
    ("chat:400", "Alice"),
];

fn main() -> std::io::Result<()> {
    let mut arguments = std::env::args().skip(1);
    let config = arguments.next().unwrap_or_else(|| "sample.toml".into());
    let env_file = arguments.next().unwrap_or_else(|| "sample.env".into());
    let setup = Setup::new(
        Description::of::<Config>("Sample bot", "A sample bot's settings."),
        config,
        env_file,
        Box::new(|candidate| {
            toml::from_str::<Config>(candidate.config)
                .map(drop)
                .map_err(|error| error.message().to_owned())
        }),
    )
    .suggest(|query| {
        let text = query.text.to_lowercase();
        Ok(CHATS
            .iter()
            .filter(|(id, title)| id.contains(&text) || title.to_lowercase().contains(&text))
            .take(query.limit)
            .map(|(id, title)| Suggestion {
                value: (*id).to_owned(),
                label: format!("{title} ({id})"),
            })
            .collect())
    })
    .action(
        "Link account",
        "A pretend login: shows a QR code, asks for a code and a password.",
        |session, _candidate| {
            session.say("Scan this with the app:");
            session.qr("https://example.com/link/sample");
            for step in 1..=3 {
                if session.cancelled() {
                    return Err("Stopped.".into());
                }
                session.progress(&format!("Waiting for the phone… {step}/3"));
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
            let code = session.ask("The code the app shows:").ok_or("Stopped.")?;
            session.ask_secret("Your password:").ok_or("Stopped.")?;
            Ok(format!(
                "Linked with code {code}; {} chats fetched.",
                CHATS.len()
            ))
        },
    );
    botconf_tui::run(setup)
}
