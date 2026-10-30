//! The panel's entry points: the settings command, its buttons and the
//! answers to its questions, as Telegram updates. What they do is the
//! engine's; this module draws its screens and keeps its questions.

use std::sync::Arc;

use botconf::{Change, Schema};
use botconf_panel::{CommandReply, Limits, PanelError, Question, Reaction, Screen, Viewer};
use teloxide::{
    ApiError, RequestError,
    dispatching::UpdateHandler,
    prelude::*,
    types::{Chat, MessageId, ParseMode, ReplyParameters, UserId},
    utils::html::escape,
};

use crate::{
    PanelBot, SettingsPanel, actor, input,
    names::BotNames,
    prompts::{self, Answer},
    render, truncate,
};

const MAX_TOAST_CHARS: usize = 200;

/// A question of the panel, kept with its prompt until answered.
#[derive(Clone, Debug)]
pub(crate) struct Pending {
    pub(crate) question: Question,
    /// The panel to update once answered.
    pub(crate) panel: MessageId,
}

impl<S: Schema, R: PanelBot> SettingsPanel<S, R> {
    /// The handler tree of the panel's buttons and questions. It reads the
    /// panel from the dependencies, as `Arc<SettingsPanel<S, R>>`.
    pub fn handler() -> UpdateHandler<anyhow::Error> {
        dptree::entry()
            .branch(
                Update::filter_message()
                    .filter_map(|msg: Message, panel: Arc<Self>| {
                        panel.prompts.answer::<Pending>(panel.owner, &msg)
                    })
                    .endpoint(handle_answer::<S, R>),
            )
            .branch(
                Update::filter_callback_query()
                    .filter(|query: CallbackQuery, panel: Arc<Self>| {
                        query
                            .data
                            .as_deref()
                            .is_some_and(|data| data.starts_with(&panel.prefix))
                    })
                    .endpoint(handle_button::<S, R>),
            )
    }

    /// Runs the settings command with its `args`: no arguments open the
    /// panel, `help` lists the text subcommands (`list`, `set`, ...).
    pub async fn run_command(&self, bot: &R, msg: &Message, args: &str) -> anyhow::Result<()> {
        let user = msg.from.as_ref().map(|user| user.id);
        let limits = self.limits();
        let names = self.bot_names(bot);
        let viewer = viewer(user, &limits, names.as_ref(), &msg.chat);

        let (text, change) = match self.panel.command_text(args, &viewer).await? {
            CommandReply::Panel(screen) => {
                send(self, bot, msg.chat.id, &screen).await?;
                return Ok(());
            }
            CommandReply::Text { text, change } => (render::html(&text), change),
        };

        bot.send_message(msg.chat.id, truncate(&text, render::MAX_MESSAGE_CHARS))
            .parse_mode(ParseMode::Html)
            .reply_parameters(ReplyParameters::new(msg.id).allow_sending_without_reply())
            .await?;

        // After replying, since the hook may take a while.
        if let Some(change) = change {
            self.changed(bot, change).await;
        }
        Ok(())
    }

    async fn changed(&self, bot: &R, change: Change<S>) {
        if let Some(on_change) = &self.on_change {
            on_change(bot.clone(), Arc::new(change)).await;
        }
    }

    pub(crate) fn limits(&self) -> Limits {
        render::limits(&self.prefix)
    }

    fn bot_names<'a>(&'a self, bot: &'a R) -> Option<BotNames<'a, R>> {
        Some(BotNames {
            names: self.names.as_deref()?,
            bot,
        })
    }
}

/// Who is looking at the panel, from where.
fn viewer<'a, R: PanelBot>(
    user: Option<UserId>,
    limits: &'a Limits,
    names: Option<&'a BotNames<'a, R>>,
    chat: &Chat,
) -> Viewer<'a> {
    Viewer {
        actor: user.and_then(actor),
        limits,
        names: names.map(|names| names as &dyn botconf_panel::Names),
        pickers: prompts::pickers_work_in(chat),
    }
}

async fn handle_button<S: Schema, R: PanelBot>(
    bot: R,
    query: CallbackQuery,
    panel: Arc<SettingsPanel<S, R>>,
) -> anyhow::Result<()> {
    let token = query
        .data
        .as_deref()
        .and_then(|data| data.strip_prefix(&panel.prefix));
    let (Some(token), Some(message)) = (token, query.regular_message()) else {
        bot.answer_callback_query(query.id.clone())
            .text(format!(
                "This button no longer works; send /{}",
                panel.panel.command_name()
            ))
            .show_alert(true)
            .await?;
        return Ok(());
    };
    let chat = message.chat.id;

    // A press on a question (a suggestion, ‹ Back) or on the panel that asked
    // it ends the question: the user moved on, so their next message is not
    // taken for an answer. The panel to update is the one that asked.
    let asked = panel
        .prompts
        .pending::<Pending>(panel.owner, chat, query.from.id)
        .filter(|pending| ends_question(pending, message.id));
    let shown = asked.as_ref().map_or(message.id, |asked| asked.data.panel);

    let limits = panel.limits();
    let names = panel.bot_names(&bot);
    let viewer = viewer(Some(query.from.id), &limits, names.as_ref(), &message.chat);
    let reaction = match panel.panel.press(token, &viewer).await {
        Ok(reaction) => reaction,
        Err(PanelError::Rejected(problem)) => {
            bot.answer_callback_query(query.id.clone())
                .text(truncate(&problem, MAX_TOAST_CHARS))
                .show_alert(true)
                .await?;
            return Ok(());
        }
        Err(PanelError::Stale { notice, screen }) => {
            bot.answer_callback_query(query.id.clone())
                .text(truncate(&notice, MAX_TOAST_CHARS))
                .await?;
            show(&panel, &bot, chat, shown, &screen).await?;
            return Ok(());
        }
        Err(error) => return Err(error.into()),
    };

    if let Some(asked) = &asked {
        panel.prompts.finish(chat, query.from.id);
        let notice = reaction.notice.as_deref().unwrap_or("OK");
        prompts::discard(&bot, chat, &asked.prompt, notice).await?;
    }
    react(&panel, &bot, &query, chat, shown, reaction).await
}

/// Whether a press on message `pressed` ends the pending question: a press on
/// the question itself (a suggestion, ‹ Back) or on the panel that asked it.
fn ends_question(pending: &Answer<Pending>, pressed: MessageId) -> bool {
    pending.prompt.message == pressed || pending.data.panel == pressed
}

/// Draws what a press led to.
async fn react<S: Schema, R: PanelBot>(
    panel: &SettingsPanel<S, R>,
    bot: &R,
    query: &CallbackQuery,
    chat: ChatId,
    shown: MessageId,
    reaction: Reaction<S>,
) -> anyhow::Result<()> {
    // First, so that the button stops spinning even if the rest fails.
    let notice = reaction.notice.as_deref().unwrap_or_default();
    bot.answer_callback_query(query.id.clone())
        .text(truncate(notice, MAX_TOAST_CHARS))
        .await?;

    if reaction.closed {
        if bot.delete_message(chat, shown).await.is_err()
            && let Some(screen) = &reaction.screen
        {
            show(panel, bot, chat, shown, screen).await?;
        }
        return Ok(());
    }
    if let Some(ask) = &reaction.ask {
        let (text, markup, keyboard) = input::question(&panel.prefix, ask, chat.is_user());
        let sent = bot
            .send_message(chat, text)
            .parse_mode(ParseMode::Html)
            .reply_markup(markup)
            .await?;
        let pending = Pending {
            question: ask.question.clone(),
            panel: shown,
        };
        let replaced =
            panel
                .prompts
                .ask(panel.owner, chat, query.from.id, sent.id, keyboard, pending);
        if let Some(replaced) = replaced {
            prompts::discard(bot, chat, &replaced, "Replaced").await?;
        }
    }
    if let Some(screen) = &reaction.post {
        send(panel, bot, chat, screen).await?;
    }
    if let Some(screen) = &reaction.screen {
        show(panel, bot, chat, shown, screen).await?;
    }
    if let Some(change) = reaction.change {
        panel.changed(bot, change).await;
    }
    Ok(())
}

async fn handle_answer<S: Schema, R: PanelBot>(
    bot: R,
    msg: Message,
    answer: Answer<Pending>,
    panel: Arc<SettingsPanel<S, R>>,
) -> anyhow::Result<()> {
    let chat = msg.chat.id;
    let Some(user) = msg.from.as_ref().map(|user| user.id) else {
        return Ok(());
    };
    let Answer { prompt, data } = answer;
    remember_picked(&panel, &msg).await;

    let limits = panel.limits();
    let names = panel.bot_names(&bot);
    let viewer = viewer(Some(user), &limits, names.as_ref(), &msg.chat);
    let answered = panel
        .panel
        .answer(&data.question, input::input(&msg), &viewer)
        .await;
    let (notice, screen, change) = match answered {
        Ok(reaction) => (
            reaction.notice.unwrap_or_default(),
            reaction.screen,
            reaction.change,
        ),
        // The setting is gone: drop the question and show where it was.
        Err(PanelError::Stale { notice, screen }) => (notice, Some(screen), None),
        Err(PanelError::Rejected(problem)) => {
            let how_to_stop = if prompt.keyboard {
                "tap Cancel"
            } else {
                "send /cancel"
            };
            bot.send_message(
                chat,
                escape(&format!("❌ {problem}\nTry again, or {how_to_stop}.")),
            )
            .parse_mode(ParseMode::Html)
            .reply_parameters(ReplyParameters::new(msg.id).allow_sending_without_reply())
            .await?;
            return Ok(());
        }
        Err(error) => return Err(error.into()),
    };

    panel.prompts.finish(chat, user);
    prompts::clean_up(&bot, &msg, &prompt, &notice).await?;
    if let Some(screen) = &screen
        && show(&panel, &bot, chat, data.panel, screen).await.is_err()
    {
        // The panel is gone: post a new one.
        send(&panel, &bot, chat, screen).await?;
    }
    if let Some(change) = change {
        panel.changed(&bot, change).await;
    }
    Ok(())
}

/// Remembers the names of the users and chats picked with Telegram's
/// pickers, to show them in the panel.
pub(crate) async fn remember_picked<S: Schema, R: PanelBot>(
    panel: &SettingsPanel<S, R>,
    message: &Message,
) {
    let Some(names) = &panel.names else {
        return;
    };
    if let Some(shared) = message.shared_users() {
        names.remember_users(&shared.users).await;
    }
    if let Some(chat) = message.shared_chat() {
        names.remember_chat(chat).await;
    }
}

/// Posts `page` as a new panel.
pub(crate) async fn post<S: Schema, R: PanelBot>(
    panel: &SettingsPanel<S, R>,
    bot: &R,
    chat: ChatId,
    page: &botconf_panel::Page,
) -> anyhow::Result<()> {
    let limits = panel.limits();
    let names = panel.bot_names(bot);
    let viewer = Viewer {
        names: names
            .as_ref()
            .map(|names| names as &dyn botconf_panel::Names),
        ..Viewer::new(&limits)
    };
    let screen = panel.panel.render(page, &viewer).await;
    send(panel, bot, chat, &screen).await?;
    Ok(())
}

async fn send<S: Schema, R: PanelBot>(
    panel: &SettingsPanel<S, R>,
    bot: &R,
    chat: ChatId,
    screen: &Screen,
) -> Result<(), RequestError> {
    let (text, keyboard) = render::message(&panel.prefix, screen);
    bot.send_message(chat, text)
        .parse_mode(ParseMode::Html)
        .reply_markup(keyboard)
        .await?;
    Ok(())
}

/// Shows `screen` on the panel `message`.
async fn show<S: Schema, R: PanelBot>(
    panel: &SettingsPanel<S, R>,
    bot: &R,
    chat: ChatId,
    message: MessageId,
    screen: &Screen,
) -> Result<(), RequestError> {
    let (text, keyboard) = render::message(&panel.prefix, screen);
    let edit = bot
        .edit_message_text(chat, message, text)
        .parse_mode(ParseMode::Html)
        .reply_markup(keyboard)
        .await;
    match edit {
        Ok(_) | Err(RequestError::Api(ApiError::MessageNotModified)) => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use botconf_panel::{Action, Page, Target};
    use serde_json::json;
    use teloxide::types::InlineKeyboardButtonKind;

    use super::*;
    use crate::{
        prompts::tests::message,
        testing::{FakeNames, panel},
    };

    const PREFIX: &str = crate::DEFAULT_PREFIX;

    #[test]
    fn buttons_can_post_pages_from_elsewhere() {
        let button =
            crate::post_button(PREFIX, "⚙️ Settings", Page::Section("lamp".into())).unwrap();
        let InlineKeyboardButtonKind::CallbackData(data) = &button.kind else {
            panic!()
        };
        let token = data.strip_prefix(PREFIX).unwrap();
        assert_eq!(
            Action::parse(token),
            Some(Action::Post(Page::Section("lamp".into())))
        );
        assert!(data.strip_prefix("other:").is_none());
    }

    #[tokio::test]
    async fn picked_users_are_remembered() {
        let names = FakeNames::default();
        let remembered = Arc::clone(&names.remembered);
        let (panel, _) = panel().await;
        let panel = SettingsPanel::new(Arc::clone(panel.store()), Default::default()).names(names);

        let shared = message(json!({
            "users_shared": { "request_id": 1, "users": [{ "user_id": 8 }] }
        }));
        remember_picked(&panel, &shared).await;
        assert_eq!(*remembered.lock().unwrap(), [UserId(8)]);
    }

    #[tokio::test]
    async fn screens_are_drawn_with_the_prefix_and_html() {
        let (panel, bot) = panel().await;
        let limits = panel.limits();
        let names = panel.bot_names(&bot);
        let viewer = Viewer {
            names: names
                .as_ref()
                .map(|names| names as &dyn botconf_panel::Names),
            ..Viewer::new(&limits)
        };
        panel
            .store()
            .set("admins", json!([7]), Some("1".into()))
            .await
            .unwrap();
        let catalog = panel.store().catalog();
        let admins = catalog
            .live()
            .find(|(_, entry)| entry.key == "admins")
            .unwrap()
            .0;
        let screen = panel
            .panel
            .render(&Page::Setting(Target::setting(admins)), &viewer)
            .await;
        let (text, keyboard) = render::message(PREFIX, &screen);
        assert!(text.starts_with("<b>Admins</b>\n<i>"), "{text}");
        assert!(text.contains("Ann (@ann) — <code>7</code>"), "{text}");
        assert!(text.contains("changed from Telegram"), "{text}");
        let data: Vec<_> = keyboard
            .inline_keyboard
            .iter()
            .flatten()
            .map(|button| match &button.kind {
                InlineKeyboardButtonKind::CallbackData(data) => data.clone(),
                other => panic!("{other:?}"),
            })
            .collect();
        assert!(
            data.iter()
                .all(|data| data.starts_with(PREFIX) && data.len() <= 64)
        );
        assert!(data.contains(&format!("{PREFIX}r:{admins}:7")), "{data:?}");
    }

    #[test]
    fn a_press_on_the_question_or_its_panel_ends_the_question() {
        let prompts = prompts::Prompts::default();
        let (chat, user) = (ChatId(5), UserId(9));
        let question =
            botconf_panel::Question::new(Target::setting(1), botconf_panel::AskKind::Value);
        prompts.ask(
            "settings",
            chat,
            user,
            MessageId(20),
            false,
            Pending {
                question,
                panel: MessageId(10),
            },
        );
        let pending = prompts.pending::<Pending>("settings", chat, user).unwrap();
        // ‹ Back or a suggestion, on the question.
        assert!(ends_question(&pending, MessageId(20)));
        // Any button of the panel that asked.
        assert!(ends_question(&pending, MessageId(10)));
        // Another panel in the chat leaves the question alone.
        assert!(!ends_question(&pending, MessageId(30)));
    }
}
