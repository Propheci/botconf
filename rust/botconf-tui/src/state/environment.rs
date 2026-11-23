//! The Environment screen: the variables the program reads itself
//! ([`Description::env`](botconf::describe::Description::env)), their values
//! set in the env file.

use super::{App, Edit, Mode, Row, Screen, Shown, Target};

impl App<'_> {
    /// The Environment screen's rows: one a variable.
    pub(super) fn environment_rows(&self) -> Vec<Row> {
        (0..self.description.env.len()).map(Row::Variable).collect()
    }

    pub(super) fn variable_title(&self, index: usize) -> String {
        let variable = &self.description.env[index];
        if variable.title.is_empty() {
            variable.name.clone()
        } else {
            variable.title.clone()
        }
    }

    pub(super) fn environment_shown(&self, row: &Row) -> Shown {
        let is_set = |name: &str| self.env.get(name).is_some_and(|value| !value.is_empty());
        match row {
            Row::Variable(index) => {
                let variable = &self.description.env[*index];
                match self
                    .env
                    .get(&variable.name)
                    .filter(|value| !value.is_empty())
                {
                    Some(_) if variable.secret => Shown::Set("•••••• (set)".into()),
                    Some(value) => Shown::Set(value.to_owned()),
                    None if variable.required => Shown::Missing,
                    None => Shown::Unset,
                }
            }
            _ => {
                let set = self
                    .description
                    .env
                    .iter()
                    .filter(|variable| is_set(&variable.name))
                    .count();
                Shown::Opens(format!("{set} of {} set", self.description.env.len()))
            }
        }
    }

    pub(super) fn environment_details(&self, row: &Row) -> (String, Vec<String>) {
        let Row::Variable(index) = row else {
            return (
                "Environment".into(),
                vec![
                    format!(
                        "The environment variables {} reads itself, by their names: their values \
                         are set in {}.",
                        self.description.title, self.env_name
                    ),
                    "Enter: open".into(),
                ],
            );
        };
        let variable = &self.description.env[*index];
        let mut lines: Vec<String> = if variable.description.is_empty() {
            Vec::new()
        } else {
            variable
                .description
                .split("\n\n")
                .map(str::to_owned)
                .collect()
        };
        let mut facts = vec![format!(
            "Variable: {}, set in {}",
            variable.name, self.env_name
        )];
        if variable.required {
            facts.push("Required".into());
        }
        if variable.secret {
            facts.push("Secret: masked here".into());
        }
        facts.push("Enter: set its value (empty: unset)".into());
        lines.push(facts.join("\n"));
        (self.variable_title(*index), lines)
    }

    pub(super) fn open_environment(&mut self) {
        self.screens.push(Screen {
            path: Vec::new(),
            title: "Environment".into(),
            cursor: 0,
            domain: None,
            env: true,
        });
    }

    pub(super) fn edit_variable(&mut self, index: usize) {
        let variable = &self.description.env[index];
        let note = if variable.required {
            "Required."
        } else {
            "Empty: unset."
        };
        let edit = Edit::new(
            format!("{} in {}", variable.name, self.env_name),
            Target::Env {
                var: variable.name.clone(),
            },
            self.env.get(&variable.name).unwrap_or_default().to_owned(),
            note.into(),
        );
        self.mode = Mode::Edit(Edit {
            secret: variable.secret,
            ..edit
        });
    }
}
