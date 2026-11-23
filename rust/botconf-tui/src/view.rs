//! Draws the [`App`].

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, List, ListItem, ListState, Paragraph, Wrap};

use crate::state::{App, Confirm, Mode, Shown};

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
        .map(|row| App::title_of(row).chars().count())
        .max()
        .unwrap_or(0)
        .min(32);
    let items: Vec<ListItem> = rows
        .iter()
        .map(|row| {
            let title = format!("{:width$}  ", App::title_of(row));
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
            let popup = centered(area, 64, 4);
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
            let text = vec![
                Line::raw(visible),
                Line::styled(edit.note.as_str(), Style::default().fg(Color::DarkGray)),
            ];
            frame.render_widget(
                Paragraph::new(text).block(
                    Block::bordered()
                        .title(edit.title.as_str())
                        .title_bottom("enter save · esc cancel · ctrl-u clear"),
                ),
                popup,
            );
            let x = popup.x + 1 + u16::try_from(edit.input.cursor - skip).unwrap_or(0);
            frame.set_cursor_position(Position::new(x, popup.y + 1));
        }
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

const HELP: [&str; 5] = [
    "↑ ↓ (j k): move · esc (← h): back",
    "enter (→ l): edit an option, open a table or an item",
    "u: unset (the default applies) · a: add an item · d: delete an item",
    "e: set an environment variable's value · g: generate a token",
    "s: save (checked first) · q: quit",
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
