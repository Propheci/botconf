//! The env file (`KEY=value` lines, as dotenv and shells read them): its
//! variables read and set, its other lines kept as they are.

use std::collections::BTreeMap;
use std::fmt::Write as _;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EnvFile {
    lines: Vec<Line>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Line {
    /// A variable. `raw` is the line as read, written back unless the value
    /// changed.
    Pair {
        key: String,
        value: String,
        raw: Option<String>,
    },
    /// A comment, a blank line, anything else.
    Other(String),
}

impl EnvFile {
    pub fn parse(text: &str) -> Self {
        let lines = text
            .lines()
            .map(|line| match parse_pair(line) {
                Some((key, value)) => Line::Pair {
                    key,
                    value,
                    raw: Some(line.to_owned()),
                },
                None => Line::Other(line.to_owned()),
            })
            .collect();
        Self { lines }
    }

    /// The value of `key`: its last assignment, as dotenv reads it.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.lines.iter().rev().find_map(|line| match line {
            Line::Pair { key: k, value, .. } if k == key => Some(value.as_str()),
            _ => None,
        })
    }

    /// Sets `key`, in place when it is already assigned, else at the end.
    pub fn set(&mut self, key: &str, value: &str) {
        let mut found = false;
        for line in self.lines.iter_mut().rev() {
            if let Line::Pair {
                key: k,
                value: v,
                raw,
            } = line
                && k == key
            {
                if v != value {
                    *v = value.to_owned();
                    *raw = None;
                }
                found = true;
                break;
            }
        }
        if !found {
            self.lines.push(Line::Pair {
                key: key.to_owned(),
                value: value.to_owned(),
                raw: None,
            });
        }
    }

    /// Every variable, its last value.
    pub fn vars(&self) -> BTreeMap<String, String> {
        self.lines
            .iter()
            .filter_map(|line| match line {
                Line::Pair { key, value, .. } => Some((key.clone(), value.clone())),
                Line::Other(_) => None,
            })
            .collect()
    }

    pub fn render(&self) -> String {
        let mut out = String::new();
        for line in &self.lines {
            match line {
                Line::Pair { raw: Some(raw), .. } | Line::Other(raw) => out.push_str(raw),
                Line::Pair {
                    key,
                    value,
                    raw: None,
                } => {
                    let _ = write!(out, "{key}={}", quote(value));
                }
            }
            out.push('\n');
        }
        out
    }
}

/// `KEY=value`, `export KEY=value`, the value bare (up to a ` #` comment),
/// `"double quoted"` (with `\` escapes) or `'single quoted'`.
fn parse_pair(line: &str) -> Option<(String, String)> {
    let line = line.trim_start();
    if line.starts_with('#') {
        return None;
    }
    let line = line.strip_prefix("export ").unwrap_or(line);
    let (key, value) = line.split_once('=')?;
    let key = key.trim();
    let valid = !key.is_empty()
        && !key.starts_with(|c: char| c.is_ascii_digit())
        && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if !valid {
        return None;
    }
    let value = value.trim_start();
    let value = if let Some(rest) = value.strip_prefix('"') {
        let mut out = String::new();
        let mut chars = rest.chars();
        while let Some(c) = chars.next() {
            match c {
                '"' => break,
                '\\' => match chars.next() {
                    Some('n') => out.push('\n'),
                    Some('t') => out.push('\t'),
                    Some(other) => out.push(other),
                    None => break,
                },
                c => out.push(c),
            }
        }
        out
    } else if let Some(rest) = value.strip_prefix('\'') {
        rest.split('\'').next().unwrap_or_default().to_owned()
    } else {
        let bare = value.find(" #").map_or(value, |end| &value[..end]);
        bare.trim_end().to_owned()
    };
    Some((key.to_owned(), value))
}

/// A value as it is written: bare when it can be, else double quoted.
fn quote(value: &str) -> String {
    let bare = !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.:/@+=,".contains(c));
    if bare {
        return value.to_owned();
    }
    let mut out = String::from("\"");
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '$' => out.push_str("\\$"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A new random token: 32 bytes from the OS, as 64 hex digits.
pub fn generate_token() -> Result<String, String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|error| format!("no randomness: {error}"))?;
    Ok(bytes
        .iter()
        .fold(String::with_capacity(64), |mut out, byte| {
            let _ = write!(out, "{byte:02x}");
            out
        }))
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = "\
# The core's tokens
export WUPHF_TOKEN=abc # set by hand
QUOTED=\"a \\\"b\\\" c\"
SINGLE='x y'

not a pair
";

    #[test]
    fn reads_every_form() {
        let env = EnvFile::parse(FILE);
        assert_eq!(env.get("WUPHF_TOKEN"), Some("abc"));
        assert_eq!(env.get("QUOTED"), Some("a \"b\" c"));
        assert_eq!(env.get("SINGLE"), Some("x y"));
        assert_eq!(env.get("not"), None);
    }

    #[test]
    fn keeps_the_other_lines_and_rewrites_only_changes() {
        let mut env = EnvFile::parse(FILE);
        assert_eq!(env.render(), FILE, "unchanged, it writes back as read");
        env.set("SINGLE", "x y");
        assert_eq!(env.render(), FILE, "the same value keeps the line");
        env.set("WUPHF_TOKEN", "new value");
        env.set("ADDED", "1");
        let rendered = env.render();
        assert!(rendered.contains("\nWUPHF_TOKEN=\"new value\"\nQUOTED="));
        assert!(rendered.ends_with("not a pair\nADDED=1\n"));
        assert_eq!(
            EnvFile::parse(&rendered).get("WUPHF_TOKEN"),
            Some("new value")
        );
    }

    #[test]
    fn quoted_values_read_back() {
        let mut env = EnvFile::default();
        for value in ["", "a b", "q\"uote", "back\\slash", "$HOME", "multi\nline"] {
            env.set("V", value);
            assert_eq!(EnvFile::parse(&env.render()).get("V"), Some(value));
        }
    }

    #[test]
    fn tokens_are_new_each_time() {
        let (first, second) = (generate_token().unwrap(), generate_token().unwrap());
        assert_eq!(first.len(), 64);
        assert!(first.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(first, second);
    }
}
