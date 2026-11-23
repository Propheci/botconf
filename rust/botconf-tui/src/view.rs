//! Draws the [`App`].

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, List, ListItem, ListState, Paragraph, Wrap};

use crate::state::{App, Confirm, Mode, Running, Shown};

/// Wide enough for the details beside the list.
const SIDE_BY_SIDE: u16 = 90;

pub fn draw(frame: &mut Frame, app: &App) {
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .areas(frame.area());

    let crumbs: Vec<&str> = app.screens.iter().map(|s| s.title.as_str()).collect();
    let mut title = vec![Span::styled(
        crumbs.join(" › "),
        Style::default().add_modifier(Modifier::BOLD),
    )];
    if app.dirty {
        title.push(Span::styled(
            "  (unsaved)",
            Style::default().fg(Color::Yellow),
        ));
    }
    frame.render_widget(Line::from(title), header);

    let [list_area, details_area] = if body.width >= SIDE_BY_SIDE {
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(body)
    } else {
        Layout::vertical([Constraint::Min(3), Constraint::Length(9)]).areas(body)
    };
    draw_rows(frame, app, list_area, details_area);

    let footer_line = match &app.status {
        Some(status) => Line::styled(status.as_str(), Style::default().fg(Color::Yellow)),
        None => Line::styled(app.hints(), Style::default().fg(Color::DarkGray)),
    };
    frame.render_widget(Paragraph::new(footer_line), footer);

    draw_mode(frame, app);
}

fn draw_rows(frame: &mut Frame, app: &App, list_area: Rect, details_area: Rect) {
    let rows = app.rows();
    let cursor = app.screen().cursor.min(rows.len().saturating_sub(1));
    if rows.is_empty() {
        let text = if app.is_collection() {
            "Nothing here yet: a adds one."
        } else {
            "No options."
        };
        frame.render_widget(Paragraph::new(text).block(Block::bordered()), list_area);
        frame.render_widget(Block::bordered(), details_area);
        return;
    }
    let width = rows
        .iter()
        .map(|row| app.title_of(row).chars().count())
        .max()
        .unwrap_or(0)
        .min(32);
    let items: Vec<ListItem> = rows
        .iter()
        .map(|row| {
            let title = format!("{:width$}  ", app.title_of(row));
            let value = match app.shown(row) {
                Shown::Set(text) => Span::raw(text),
                Shown::Default(text) => Span::styled(
                    format!("{text} (default)"),
                    Style::default().fg(Color::DarkGray),
                ),
                Shown::Unset => Span::styled("—", Style::default().fg(Color::DarkGray)),
                Shown::Missing => Span::styled("required", Style::default().fg(Color::Red)),
                Shown::Opens(text) => Span::styled(
                    format!("{text} ›").trim_start().to_owned(),
                    Style::default().fg(Color::Cyan),
                ),
            };
            ListItem::new(Line::from(vec![Span::raw(title), value]))
        })
        .collect();
    let list = List::new(items)
        .block(Block::bordered())
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
    let mut state = ListState::default().with_selected(Some(cursor));
    frame.render_stateful_widget(list, list_area, &mut state);

    let (title, paragraphs) = app.details(&rows[cursor]);
    let mut lines = Vec::new();
    for (index, paragraph) in paragraphs.iter().enumerate() {
        if index > 0 {
            lines.push(Line::raw(""));
        }
        lines.extend(paragraph.lines().map(|line| Line::raw(line.to_owned())));
    }
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: true })
            .block(Block::bordered().title(title)),
        details_area,
    );
}

fn draw_mode(frame: &mut Frame, app: &App) {
    let area = frame.area();
    match &app.mode {
        Mode::Browse => {}
        Mode::Edit(edit) => {
            // The matches go under the note, one a line.
            let listed = u16::try_from(edit.suggestions.len()).unwrap_or(u16::MAX);
            let extra = match (edit.domain.is_some(), listed) {
                (false, _) => 0,
                (true, 0) => 1,
                (true, listed) => listed,
            };
            let popup = centered(area, 72, 4u16.saturating_add(extra));
            frame.render_widget(Clear, popup);
            let shown: String = if edit.secret {
                "•".repeat(edit.input.text.chars().count())
            } else {
                edit.input.text.clone()
            };
            let inner_width = usize::from(popup.width.saturating_sub(2)).max(1);
            // Keep the cursor in view on long input.
            let skip = edit.input.cursor.saturating_sub(inner_width - 1);
            let visible: String = shown.chars().skip(skip).collect();
            let mut text = vec![
                Line::raw(visible),
                Line::styled(edit.note.as_str(), Style::default().fg(Color::DarkGray)),
            ];
            if edit.domain.is_some() {
                if edit.suggestions.is_empty() {
                    let why = edit.unsuggested.as_deref().unwrap_or("No matches.");
                    text.push(Line::styled(why, Style::default().fg(Color::Yellow)));
                }
                for (index, suggestion) in edit.suggestions.iter().enumerate() {
                    let style = if edit.highlight == Some(index) {
                        Style::default().add_modifier(Modifier::REVERSED)
                    } else {
                        Style::default().fg(Color::Cyan)
                    };
                    text.push(Line::styled(suggestion.label.clone(), style));
                }
            }
            let bottom = if edit.domain.is_some() {
                "↓↑ pick · enter save · esc cancel"
            } else {
                "enter save · esc cancel · ctrl-u clear"
            };
            frame.render_widget(
                Paragraph::new(text).block(
                    Block::bordered()
                        .title(edit.title.as_str())
                        .title_bottom(bottom),
                ),
                popup,
            );
            let x = popup.x + 1 + u16::try_from(edit.input.cursor - skip).unwrap_or(0);
            frame.set_cursor_position(Position::new(x, popup.y + 1));
        }
        Mode::Running(running) => draw_running(frame, area, running),
        Mode::Choose(choose) => {
            let height = u16::try_from(choose.options.len()).unwrap_or(u16::MAX);
            let popup = centered(area, 48, height.saturating_add(2));
            frame.render_widget(Clear, popup);
            let items: Vec<ListItem> = choose
                .options
                .iter()
                .map(|(value, label)| {
                    ListItem::new(if value == label {
                        label.clone()
                    } else {
                        format!("{label} ({value})")
                    })
                })
                .collect();
            let list = List::new(items)
                .block(Block::bordered().title(choose.title.as_str()))
                .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
            let mut state = ListState::default().with_selected(Some(choose.cursor));
            frame.render_stateful_widget(list, popup, &mut state);
        }
        Mode::Confirm(confirm) => {
            let text = match confirm {
                Confirm::Quit => "Quit without saving the changes?".to_owned(),
                Confirm::Delete { .. } => "Delete this item?".to_owned(),
                Confirm::Regenerate { var } => format!(
                    "{var} is already set. A new value must be given to the other side too. \
                     Replace it?"
                ),
            };
            message(frame, area, "Sure?", &text, "y yes · any other key no");
        }
        Mode::Message(text) => message(frame, area, "Note", text, "any key"),
        Mode::Help => message(frame, area, "Keys", &HELP.join("\n"), "any key"),
    }
}

const HELP: [&str; 6] = [
    "↑ ↓ (j k): move · esc (← h): back",
    "enter (→ l): edit an option, open a table or an item",
    "u: unset (the default applies) · a: add an item · d: delete an item",
    "e: set an environment variable's value · g: generate a token",
    "s: save (checked first) · q: quit",
    "▶ rows: the program's actions (saved first; esc stops one)",
];

fn message(frame: &mut Frame, area: Rect, title: &str, text: &str, bottom: &str) {
    let popup = centered(area, 64, 8);
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(text)
            .wrap(Wrap { trim: true })
            .block(Block::bordered().title(title).title_bottom(bottom)),
        popup,
    );
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width.saturating_sub(2)).max(1);
    let height = height.min(area.height.saturating_sub(2)).max(1);
    Rect::new(
        area.x + (area.width.saturating_sub(width)) / 2,
        area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    )
}

/// A running action: what it said, its QR code, its progress and its
/// question, over the whole screen.
fn draw_running(frame: &mut Frame, area: Rect, running: &Running) {
    frame.render_widget(Clear, area);
    let mut lines: Vec<Line> = Vec::new();
    if let Some(data) = &running.qr {
        let warn = |text: String| Line::styled(text, Style::default().fg(Color::Yellow));
        match qr_lines(data) {
            // A wrapped or cut QR code does not scan: say what it needs.
            Some(qr) => {
                let width = qr.first().map_or(0, |line| line.chars().count());
                let height = qr.len();
                let fits =
                    width + 2 <= usize::from(area.width) && height + 4 <= usize::from(area.height);
                if fits {
                    lines.extend(qr.into_iter().map(Line::raw));
                } else {
                    lines.push(warn(format!(
                        "The QR code needs a terminal of at least {} × {}: enlarge it.",
                        width + 2,
                        height + 4
                    )));
                }
            }
            None => lines.push(warn("The QR code could not be drawn.".into())),
        }
    }
    // The latest lines that fit below the QR code.
    let room = usize::from(area.height.saturating_sub(6)).saturating_sub(lines.len());
    let skip = running.lines.len().saturating_sub(room);
    lines.extend(
        running
            .lines
            .iter()
            .skip(skip)
            .map(|line| Line::raw(line.clone())),
    );
    if let Some(progress) = &running.progress {
        lines.push(Line::styled(
            progress.clone(),
            Style::default().fg(Color::Cyan),
        ));
    }
    if running.cancelling {
        lines.push(Line::styled(
            "Stopping…",
            Style::default().fg(Color::Yellow),
        ));
    }
    let mut cursor = None;
    if let Some(question) = &running.question {
        let shown = if question.secret {
            "•".repeat(question.input.text.chars().count())
        } else {
            question.input.text.clone()
        };
        lines.push(Line::styled(
            question.prompt.clone(),
            Style::default().add_modifier(Modifier::BOLD),
        ));
        let prompt_width = u16::try_from(question.input.cursor).unwrap_or(u16::MAX);
        cursor = Some((prompt_width, u16::try_from(lines.len()).unwrap_or(u16::MAX)));
        lines.push(Line::raw(shown));
    }
    let bottom = match (&running.done, &running.question) {
        (Some(Ok(_)), _) => "done · any key",
        (Some(Err(_)), _) => "failed · any key",
        (None, Some(_)) => "enter answer · esc stop",
        (None, None) => "esc stop",
    };
    if let Some(done) = &running.done {
        let (text, color) = match done {
            Ok(notice) => (notice.as_str(), Color::Green),
            Err(error) => (error.as_str(), Color::Red),
        };
        lines.push(Line::styled(text.to_owned(), Style::default().fg(color)));
    }
    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }).block(
            Block::bordered()
                .title(running.title.as_str())
                .title_bottom(bottom),
        ),
        area,
    );
    if let Some((x, y)) = cursor {
        frame.set_cursor_position(Position::new(
            area.x + 1 + x,
            area.y + 1 + y.min(area.height.saturating_sub(2)),
        ));
    }
}

/// A QR code as lines of half blocks (two modules a character, light on dark
/// inverted so phones read it on dark terminals too), or `None` when it
/// cannot be encoded.
pub fn qr_lines(data: &str) -> Option<Vec<String>> {
    let code = qrcode::QrCode::new(data.as_bytes()).ok()?;
    let rendered = code
        .render::<qrcode::render::unicode::Dense1x2>()
        .dark_color(qrcode::render::unicode::Dense1x2::Light)
        .light_color(qrcode::render::unicode::Dense1x2::Dark)
        .quiet_zone(true)
        .build();
    Some(rendered.lines().map(str::to_owned).collect())
}
