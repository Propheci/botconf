//! An offline setup screen for a program's config file and the env file its
//! options name, over the program's [`Description`]: every option with its
//! description, edited without the program running.
//!
//! The file is written from the description, so it always reads like the
//! generated example; keys the description does not know are kept as
//! comments. An option naming an environment variable sets that variable's
//! value in the env file (masked when secret; a token can be generated); the
//! env file keeps its other lines. Before anything is written the host
//! checks the candidate with its own loader.
//!
//! Values of a domain (`#[botconf(domain = "…")]`, chats say) are suggested
//! by name as they are typed, from the host's `suggest`. The host's actions
//! (a login, a fetch) run from the root screen on the saved files, talking to
//! the screen through a [`Session`]: lines, a QR code, progress, questions.
//!
//! ```no_run
//! # use botconf::describe::Description;
//! # fn description() -> Description { unimplemented!() }
//! use botconf_tui::{Setup, Suggestion};
//!
//! let setup = Setup::new(
//!     description(),
//!     "config.local.toml",
//!     ".env.local",
//!     // e.g. the program's own loader over `candidate.config`.
//!     Box::new(|_candidate| Ok(())),
//! )
//! .suggest(|query| {
//!     Ok(vec![Suggestion {
//!         value: "chat:1".into(),
//!         label: "Family (chat:1)".into(),
//!     }])
//! })
//! .action(
//!     "Link account",
//!     "Log in and fetch the chats.",
//!     |session, _candidate| {
//!         let code = session
//!             .ask("The code you were sent:")
//!             .ok_or("stopped".to_owned())?;
//!         session.say(&format!("Logged in with {code}."));
//!         Ok("Linked.".into())
//!     },
//! );
//! botconf_tui::run(setup).unwrap();
//! ```

use std::collections::BTreeMap;
use std::io;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use botconf::describe::Description;
use crossterm::event::{self, Event, KeyEventKind};
use serde_json::Value;

pub mod env;
mod state;
mod view;

pub use env::{EnvFile, generate_token};
use state::{ActionEvent, ActionInfo, App, Effect};
pub use state::{Query, SuggestFn, Suggestion};

/// What a save is checked against: the config file and the env file's
/// variables, as they would be written.
pub struct Candidate<'a> {
    pub config: &'a str,
    pub env: &'a BTreeMap<String, String>,
}

/// The host's check of a candidate: its loader, say. An error is shown and
/// nothing is written.
pub type Validate<'a> = Box<dyn Fn(&Candidate) -> Result<(), String> + 'a>;

/// What an action runs: the host's code (a login, a fetch), on the saved
/// files, talking to the screen through the session. It gives a notice, or
/// the reason it failed.
pub type Run<'a> =
    Box<dyn Fn(&mut dyn Session, &Candidate) -> Result<String, String> + Send + Sync + 'a>;

/// One of the host's actions, listed on the root screen.
pub struct Action<'a> {
    pub title: String,
    pub description: String,
    pub run: Run<'a>,
}

/// How a running action talks to the screen. It runs on its own thread;
/// `ask` blocks until the answer.
pub trait Session {
    /// A line for the action's log.
    fn say(&mut self, text: &str);
    /// A QR code to scan, replacing the last one.
    fn qr(&mut self, data: &str);
    /// Where it has got to, replacing the last progress.
    fn progress(&mut self, text: &str);
    /// The answer to a question, or `None` when the user stopped the action.
    fn ask(&mut self, question: &str) -> Option<String>;
    /// As [`Session::ask`], the answer masked.
    fn ask_secret(&mut self, question: &str) -> Option<String>;
    /// The user asked to stop: return soon.
    fn cancelled(&self) -> bool;
}

pub struct Setup<'a> {
    pub description: Description,
    pub config: PathBuf,
    pub env_file: PathBuf,
    pub validate: Validate<'a>,
    /// Values of a domain (`#[botconf(domain = "…")]`), suggested by name.
    pub suggest: Option<SuggestFn<'a>>,
    pub actions: Vec<Action<'a>>,
}

impl<'a> Setup<'a> {
    pub fn new(
        description: Description,
        config: impl Into<PathBuf>,
        env_file: impl Into<PathBuf>,
        validate: Validate<'a>,
    ) -> Self {
        Self {
            description,
            config: config.into(),
            env_file: env_file.into(),
            validate,
            suggest: None,
            actions: Vec::new(),
        }
    }

    /// Values of a domain are suggested by `suggest`.
    #[must_use]
    pub fn suggest(
        mut self,
        suggest: impl Fn(&Query) -> Result<Vec<Suggestion>, String> + 'a,
    ) -> Self {
        self.suggest = Some(Box::new(suggest));
        self
    }

    /// An action on the root screen.
    #[must_use]
    pub fn action(
        mut self,
        title: impl Into<String>,
        description: impl Into<String>,
        run: impl Fn(&mut dyn Session, &Candidate) -> Result<String, String> + Send + Sync + 'a,
    ) -> Self {
        self.actions.push(Action {
            title: title.into(),
            description: description.into(),
            run: Box::new(run),
        });
        self
    }
}

/// What a running action's thread sends the screen.
enum Message {
    Event(ActionEvent),
    Ask {
        question: String,
        secret: bool,
        reply: mpsc::Sender<Option<String>>,
    },
}

/// A [`Session`] over the screen's channel.
struct ChannelSession {
    messages: mpsc::Sender<Message>,
    cancelled: Arc<AtomicBool>,
}

impl ChannelSession {
    fn send(&self, event: ActionEvent) {
        let _ = self.messages.send(Message::Event(event));
    }

    fn question(&mut self, question: &str, secret: bool) -> Option<String> {
        if self.cancelled() {
            return None;
        }
        let (reply, answer) = mpsc::channel();
        self.messages
            .send(Message::Ask {
                question: question.to_owned(),
                secret,
                reply,
            })
            .ok()?;
        answer.recv().ok().flatten()
    }
}

impl Session for ChannelSession {
    fn say(&mut self, text: &str) {
        self.send(ActionEvent::Say(text.to_owned()));
    }

    fn qr(&mut self, data: &str) {
        self.send(ActionEvent::Qr(data.to_owned()));
    }

    fn progress(&mut self, text: &str) {
        self.send(ActionEvent::Progress(text.to_owned()));
    }

    fn ask(&mut self, question: &str) -> Option<String> {
        self.question(question, false)
    }

    fn ask_secret(&mut self, question: &str) -> Option<String> {
        self.question(question, true)
    }

    fn cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

/// How long the screen waits for a key before it looks at the running
/// action again.
const TICK: Duration = Duration::from_millis(100);

/// Opens the setup screen until the user quits. A missing file starts
/// empty; a config file that is not TOML is an error (fix it first).
pub fn run(setup: Setup) -> io::Result<()> {
    let Setup {
        description,
        config,
        env_file,
        validate,
        suggest,
        actions,
    } = setup;
    let values = read_config(&config)?;
    let env_text = read_optional(&env_file)?;
    let env_name = env_file.file_name().map_or_else(
        || env_file.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    );
    let mut app = App::new(
        description,
        values,
        EnvFile::parse(&env_text),
        env_name,
        generate_token,
    );
    if let Some(suggest) = suggest {
        app = app.with_suggestions(suggest);
    }
    app.actions = actions
        .iter()
        .map(|action| ActionInfo {
            title: action.title.clone(),
            description: action.description.clone(),
        })
        .collect();
    let mut saved_env = env_text;
    let mut terminal = ratatui::init();
    let result = std::thread::scope(|scope| -> io::Result<()> {
        let (messages, received) = mpsc::channel::<Message>();
        let mut reply: Option<mpsc::Sender<Option<String>>> = None;
        let mut cancel = Arc::new(AtomicBool::new(false));
        loop {
            while let Ok(message) = received.try_recv() {
                match message {
                    Message::Event(event) => app.action_event(event),
                    Message::Ask {
                        question,
                        secret,
                        reply: sender,
                    } => {
                        reply = Some(sender);
                        app.action_event(ActionEvent::Ask { question, secret });
                    }
                }
            }
            terminal.draw(|frame| view::draw(frame, &app))?;
            if !event::poll(TICK)? {
                continue;
            }
            let Event::Key(key) = event::read()? else {
                continue;
            };
            if key.kind != KeyEventKind::Press {
                continue;
            }
            match app.key(key) {
                Effect::None => {}
                Effect::Quit => return Ok(()),
                Effect::Save => {
                    let outcome = save(&app, &config, &env_file, &saved_env, &validate);
                    if let Ok((_, env)) = &outcome {
                        saved_env.clone_from(env);
                    }
                    app.saved(outcome.map(|(notice, _)| notice));
                }
                Effect::RunAction(index) => {
                    // An action runs on the saved files.
                    if app.dirty {
                        match save(&app, &config, &env_file, &saved_env, &validate) {
                            Ok((notice, env)) => {
                                saved_env = env;
                                app.saved(Ok(notice));
                            }
                            Err(error) => {
                                app.not_started(error);
                                continue;
                            }
                        }
                    }
                    let (config_text, env_text) = app.candidate();
                    let vars = EnvFile::parse(&env_text).vars();
                    cancel = Arc::new(AtomicBool::new(false));
                    let mut session = ChannelSession {
                        messages: messages.clone(),
                        cancelled: Arc::clone(&cancel),
                    };
                    let done = messages.clone();
                    let action = &actions[index];
                    app.started(index);
                    scope.spawn(move || {
                        let candidate = Candidate {
                            config: &config_text,
                            env: &vars,
                        };
                        let outcome = catch_unwind(AssertUnwindSafe(|| {
                            (action.run)(&mut session, &candidate)
                        }))
                        .unwrap_or_else(|_| Err("The action failed unexpectedly.".into()));
                        let _ = done.send(Message::Event(ActionEvent::Done(outcome)));
                    });
                }
                Effect::Answer(answer) => {
                    if let Some(sender) = reply.take() {
                        let _ = sender.send(answer);
                    }
                }
                Effect::Cancel => {
                    cancel.store(true, Ordering::SeqCst);
                    if let Some(sender) = reply.take() {
                        let _ = sender.send(None);
                    }
                }
            }
        }
    });
    ratatui::restore();
    result
}

/// Checks, then writes the config file and (when it changed) the env file.
/// Gives the notice and the env file written.
fn save(
    app: &App,
    config_path: &Path,
    env_path: &Path,
    saved_env: &str,
    validate: &Validate,
) -> Result<(String, String), String> {
    let (config, env) = app.candidate();
    let vars = EnvFile::parse(&env).vars();
    validate(&Candidate {
        config: &config,
        env: &vars,
    })?;
    write(config_path, &config, false)
        .map_err(|error| format!("writing {}: {error}", config_path.display()))?;
    let mut notice = format!("Saved {}", config_path.display());
    if env != saved_env {
        write(env_path, &env, true)
            .map_err(|error| format!("writing {}: {error}", env_path.display()))?;
        notice.push_str(&format!(" and {}", env_path.display()));
    }
    Ok((notice, env))
}

fn read_config(path: &Path) -> io::Result<Value> {
    let text = read_optional(path)?;
    if text.trim().is_empty() {
        return Ok(Value::Object(serde_json::Map::new()));
    }
    toml::from_str(&text).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{} is not valid TOML: {error}", path.display()),
        )
    })
}

fn read_optional(path: &Path) -> io::Result<String> {
    match std::fs::read_to_string(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(String::new()),
        other => other,
    }
}

/// Writes through a temporary file, so a crash leaves the old file whole.
/// A new private file is readable by its owner only.
fn write(path: &Path, text: &str, private: bool) -> io::Result<()> {
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".tmp");
    let temporary = PathBuf::from(temporary);
    std::fs::write(&temporary, text)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = match std::fs::metadata(path) {
            Ok(metadata) => metadata.permissions().mode(),
            Err(_) if private => 0o600,
            Err(_) => 0o644,
        };
        std::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(mode))?;
    }
    #[cfg(not(unix))]
    let _ = private;
    std::fs::rename(&temporary, path)
}
