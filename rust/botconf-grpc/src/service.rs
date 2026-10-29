//! The RPCs: each asks the host's authorizer first, then the store or the
//! panel.

use botconf::{
    Change, Schema,
    command::{self, Outcome, SettingsCommand},
};
use botconf_panel::{Question, Viewer};
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status};

use crate::{
    Operation, Principal, SettingsService, convert,
    proto::{self, settings_server::Settings},
    watch,
};

impl<S: Schema> SettingsService<S> {
    /// The host's decision on `request`.
    fn authorize<T>(
        &self,
        request: &Request<T>,
        actor: impl FnOnce(&T) -> Option<&proto::Actor>,
        operation: impl FnOnce(&T) -> Operation<'_>,
    ) -> Result<Principal, Status> {
        let message = request.get_ref();
        self.authorizer
            .authorize(request.metadata(), actor(message), operation(message))
    }

    /// Runs a store command for `principal`.
    async fn run(
        &self,
        command: SettingsCommand,
        principal: Principal,
    ) -> Result<Outcome<S>, Status> {
        command::execute(self.panel.store(), command, principal.actor)
            .await
            .map_err(|error| convert::settings_status(&error))
    }

    /// Answers a change command and tells the host's hook.
    async fn change(
        &self,
        command: SettingsCommand,
        key: Option<&str>,
        principal: Principal,
    ) -> Result<Response<proto::ChangeResponse>, Status> {
        let outcome = self.run(command, principal).await?;
        let response = convert::change_response(&outcome, &self.panel.store().current(), key);
        self.changed(into_change(outcome));
        Ok(Response::new(response))
    }
}

fn into_change<S: Schema>(outcome: Outcome<S>) -> Option<Change<S>> {
    match outcome {
        Outcome::Changed { change, .. } | Outcome::Reloaded { change, .. } => Some(change),
        Outcome::Listing(_) | Outcome::Value(_) | Outcome::NotOverridden(_) => None,
    }
}

#[tonic::async_trait]
impl<S: Schema> Settings for SettingsService<S> {
    type WatchStream = ReceiverStream<Result<proto::WatchEvent, Status>>;

    async fn describe(
        &self,
        request: Request<proto::DescribeRequest>,
    ) -> Result<Response<proto::DescribeResponse>, Status> {
        self.authorize(
            &request,
            |message| message.actor.as_ref(),
            |_| Operation::Describe,
        )?;
        let store = self.panel.store();
        let snapshot = store.current();
        let catalog = store.catalog();
        let settings = catalog
            .live()
            .map(|(position, entry)| convert::setting(position, entry, snapshot.as_ref()))
            .collect();
        let fixed = store
            .sections()
            .iter()
            .map(|section| proto::SectionDescriptor {
                id: section.id.to_string(),
                path: section.path.clone(),
                title: section.title.to_string(),
                description: section.description.to_string(),
                dynamic: false,
            });
        let dynamic =
            store
                .dynamic_sections()
                .await
                .into_iter()
                .map(|section| proto::SectionDescriptor {
                    id: section.id.clone(),
                    path: section.path.clone(),
                    title: section.title.clone(),
                    description: section.description.clone(),
                    dynamic: true,
                });
        Ok(Response::new(proto::DescribeResponse {
            settings,
            sections: fixed.chain(dynamic).collect(),
        }))
    }

    async fn list(
        &self,
        request: Request<proto::ListRequest>,
    ) -> Result<Response<proto::ListResponse>, Status> {
        let principal = self.authorize(
            &request,
            |message| message.actor.as_ref(),
            |_| Operation::Read { key: None },
        )?;
        match self.run(SettingsCommand::List, principal).await? {
            Outcome::Listing(listing) => Ok(Response::new(convert::listing(&listing))),
            _ => Err(Status::internal("list returned no listing")),
        }
    }

    async fn get(
        &self,
        request: Request<proto::GetRequest>,
    ) -> Result<Response<proto::ValueEntry>, Status> {
        let principal = self.authorize(
            &request,
            |message| message.actor.as_ref(),
            |message| Operation::Read {
                key: Some(&message.key),
            },
        )?;
        let key = request.into_inner().key;
        match self.run(SettingsCommand::Get(key), principal).await? {
            Outcome::Value(entry) => Ok(Response::new(convert::value_entry(&entry))),
            _ => Err(Status::internal("get returned no value")),
        }
    }

    async fn set(
        &self,
        request: Request<proto::EditRequest>,
    ) -> Result<Response<proto::ChangeResponse>, Status> {
        let principal = self.authorize(
            &request,
            |message| message.actor.as_ref(),
            |message| Operation::Change { key: &message.key },
        )?;
        let proto::EditRequest { key, value, .. } = request.into_inner();
        self.change(SettingsCommand::Set(key, value), None, principal)
            .await
    }

    async fn unset(
        &self,
        request: Request<proto::UnsetRequest>,
    ) -> Result<Response<proto::ChangeResponse>, Status> {
        let principal = self.authorize(
            &request,
            |message| message.actor.as_ref(),
            |message| Operation::Change { key: &message.key },
        )?;
        let key = request.into_inner().key;
        self.change(SettingsCommand::Unset(key.clone()), Some(&key), principal)
            .await
    }

    async fn add(
        &self,
        request: Request<proto::EditRequest>,
    ) -> Result<Response<proto::ChangeResponse>, Status> {
        let principal = self.authorize(
            &request,
            |message| message.actor.as_ref(),
            |message| Operation::Change { key: &message.key },
        )?;
        let proto::EditRequest { key, value, .. } = request.into_inner();
        self.change(SettingsCommand::Add(key, value), None, principal)
            .await
    }

    async fn remove(
        &self,
        request: Request<proto::EditRequest>,
    ) -> Result<Response<proto::ChangeResponse>, Status> {
        let principal = self.authorize(
            &request,
            |message| message.actor.as_ref(),
            |message| Operation::Change { key: &message.key },
        )?;
        let proto::EditRequest { key, value, .. } = request.into_inner();
        self.change(SettingsCommand::Remove(key, value), None, principal)
            .await
    }

    async fn reload(
        &self,
        request: Request<proto::ReloadRequest>,
    ) -> Result<Response<proto::ChangeResponse>, Status> {
        let principal = self.authorize(
            &request,
            |message| message.actor.as_ref(),
            |_| Operation::Reload,
        )?;
        self.change(SettingsCommand::Reload, None, principal).await
    }

    async fn render(
        &self,
        request: Request<proto::RenderRequest>,
    ) -> Result<Response<proto::RenderResponse>, Status> {
        let principal = self.authorize(
            &request,
            |message| message.actor.as_ref(),
            |_| Operation::Panel,
        )?;
        let message = request.into_inner();
        let limits = convert::limits(message.limits.as_ref());
        let viewer = Viewer {
            actor: principal.actor,
            limits: &limits,
            names: self.names.as_deref(),
            pickers: message.pickers,
        };
        let screen = self
            .panel
            .render(&convert::page(message.page.as_ref()), &viewer)
            .await;
        Ok(Response::new(proto::RenderResponse {
            screen: Some(convert::screen(&screen)),
        }))
    }

    async fn act(
        &self,
        request: Request<proto::ActRequest>,
    ) -> Result<Response<proto::ActResponse>, Status> {
        use proto::act_request::Act;

        let principal = self.authorize(
            &request,
            |message| message.actor.as_ref(),
            |_| Operation::Panel,
        )?;
        let message = request.into_inner();
        let limits = convert::limits(message.limits.as_ref());
        let viewer = Viewer {
            actor: principal.actor,
            limits: &limits,
            names: self.names.as_deref(),
            pickers: message.pickers,
        };

        let outcome = match message.act {
            None => return Err(Status::invalid_argument("nothing to act on")),
            Some(Act::Press(token)) => self
                .panel
                .press(&token, &viewer)
                .await
                .map(convert::reaction),
            Some(Act::Answer(answer)) => {
                let question = Question::parse(&answer.question).ok_or_else(|| {
                    Status::invalid_argument(format!(
                        "{:?} is not a question of the panel",
                        answer.question
                    ))
                })?;
                let input = convert::input(answer.input.as_ref())?;
                self.panel
                    .answer(&question, input, &viewer)
                    .await
                    .map(convert::reaction)
            }
            Some(Act::Command(args)) => self
                .panel
                .command_text(&args, &viewer)
                .await
                .map(convert::command_reply),
        };
        match outcome {
            Ok((response, change)) => {
                self.changed(change);
                Ok(Response::new(response))
            }
            Err(error) => convert::panel_error(error).map(Response::new),
        }
    }

    async fn watch(
        &self,
        request: Request<proto::WatchRequest>,
    ) -> Result<Response<Self::WatchStream>, Status> {
        self.authorize(
            &request,
            |message| message.actor.as_ref(),
            |message| Operation::Watch {
                prefixes: &message.prefixes,
            },
        )?;
        let prefixes = request.into_inner().prefixes;
        Ok(Response::new(watch::stream(
            std::sync::Arc::clone(self.panel.store()),
            prefixes,
        )))
    }
}
