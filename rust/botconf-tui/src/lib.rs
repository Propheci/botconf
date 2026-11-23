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
//! ```no_run
//! # use botconf::describe::Description;
//! # fn description() -> Description { unimplemented!() }
//! botconf_tui::run(botconf_tui::Setup {
//!     description: description(),
//!     config: "config.local.toml".into(),
//!     env_file: ".env.local".into(),
//!     validate: Box::new(|candidate| {
//!         // e.g. the program's own loader over `candidate.config`.
//!         let _ = candidate;
//!         Ok(())
//!     }),
//! })
//! .unwrap();
//! ```

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use botconf::describe::Description;
use crossterm::event::{self, Event, KeyEventKind};
use serde_json::Value;

pub mod env;
mod state;
mod view;

pub use env::{EnvFile, generate_token};
use state::{App, Effect};

/// What a save is checked against: the config file and the env file's
/// variables, as they would be written.
pub struct Candidate<'a> {
    pub config: &'a str,
    pub env: &'a BTreeMap<String, String>,
}

/// The host's check of a candidate: its loader, say. An error is shown and
/// nothing is written.
pub type Validate<'a> = Box<dyn Fn(&Candidate) -> Result<(), String> + 'a>;

pub struct Setup<'a> {
    pub description: Description,
    pub config: PathBuf,
    pub env_file: PathBuf,
    pub validate: Validate<'a>,
}

/// Opens the setup screen until the user quits. A missing file starts
/// empty; a config file that is not TOML is an error (fix it first).
pub fn run(setup: Setup) -> io::Result<()> {
    let values = read_config(&setup.config)?;
    let env_text = read_optional(&setup.env_file)?;
    let env_name = setup.env_file.file_name().map_or_else(
        || setup.env_file.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    );
    let mut app = App::new(
        setup.description,
        values,
        EnvFile::parse(&env_text),
        env_name,
        generate_token,
    );
    let mut saved_env = env_text;
    let mut terminal = ratatui::init();
    let result = (|| -> io::Result<()> {
        loop {
            terminal.draw(|frame| view::draw(frame, &app))?;
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
                    let outcome = save(
                        &app,
                        &setup.config,
                        &setup.env_file,
                        &saved_env,
                        &setup.validate,
                    );
                    if let Ok((_, env)) = &outcome {
                        saved_env.clone_from(env);
                    }
                    app.saved(outcome.map(|(notice, _)| notice));
                }
            }
        }
    })();
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
