//! Serves [`botconf`] settings and the [`botconf_panel`] settings panel over
//! gRPC, as the `botconf.v1.Settings` service
//! (`proto/botconf/v1/settings.proto` in this repository), so that programs in
//! any language can read a host's settings, change them, watch them, and show
//! its settings panel.
//!
//! A host builds a [`SettingsService`] over its panel (which holds the
//! settings store and the panel's token store) and an [`Authorizer`], and
//! adds [`SettingsService::into_server`] to its tonic server:
//!
//! ```ignore
//! let panel = Arc::new(
//!     botconf_panel::Panel::new(Arc::clone(&store))
//!         // Long buttons' handles survive restarts and serve every client.
//!         .token_store(MyPersistentTokenStore::new(db))
//!         .domain(MY_DOMAIN),
//! );
//! let settings = botconf_grpc::SettingsService::new(panel, MyAuthorizer::new(…))
//!     .names(MyNames::new(…))
//!     .into_server();
//! Server::builder().add_service(settings) /* … */;
//! ```
//!
//! The service decides nothing about who may do what: every request goes to
//! the host's [`Authorizer`] first, with the request's metadata, the actor
//! the client claims and what the request wants to do, and a refusal touches
//! nothing.
//!
//! The service is built on tonic 0.12 (prost 0.13), and a host's server must
//! use the same tonic major version: upgrading tonic is a change made here
//! first, then in the hosts.

// Handlers and the authorizer answer with `tonic::Status`, as tonic expects,
// however large it is.
#![allow(clippy::result_large_err)]

mod convert;
mod service;
#[cfg(test)]
mod tests;
mod watch;

use std::sync::Arc;

use botconf::{Change, Schema};
use botconf_panel::{Names, Panel};
use tonic::{Status, metadata::MetadataMap};

/// The generated messages, client and server of `botconf.v1`.
#[allow(clippy::all, clippy::pedantic, missing_docs)]
pub mod proto {
    tonic::include_proto!("botconf.v1");
}

/// What a request wants to do, for the [`Authorizer`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation<'a> {
    /// `Describe`: the settings and their kinds.
    Describe,
    /// `List` (no key) and `Get`.
    Read { key: Option<&'a str> },
    /// `Set`, `Unset`, `Add` and `Remove` of `key`.
    Change { key: &'a str },
    /// `Reload`.
    Reload,
    /// `Render` and `Act`: the settings panel, which can change any setting.
    Panel,
    /// `Watch` of these key prefixes (empty: every key).
    Watch { prefixes: &'a [String] },
}

/// Who the host lets a request act as.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Principal {
    /// Recorded as the author of the changes the request makes.
    pub actor: Option<String>,
}

/// The host's decision on each request.
pub trait Authorizer: Send + Sync + 'static {
    /// Lets the request through as a [`Principal`], or refuses it with the
    /// status to answer with (e.g. `PERMISSION_DENIED`, `UNAUTHENTICATED`).
    ///
    /// `actor` is who the client claims to act for (e.g. the Telegram user
    /// who pressed a button); trust it only as far as the connection's
    /// credentials in `metadata` allow.
    fn authorize(
        &self,
        metadata: &MetadataMap,
        actor: Option<&proto::Actor>,
        operation: Operation<'_>,
    ) -> Result<Principal, Status>;
}

impl<F> Authorizer for F
where
    F: Fn(&MetadataMap, Option<&proto::Actor>, Operation<'_>) -> Result<Principal, Status>
        + Send
        + Sync
        + 'static,
{
    fn authorize(
        &self,
        metadata: &MetadataMap,
        actor: Option<&proto::Actor>,
        operation: Operation<'_>,
    ) -> Result<Principal, Status> {
        self(metadata, actor, operation)
    }
}

/// Lets every request through, recording the claimed actor as
/// `<domain>:<id>`. For tests only: a real host must check who is asking.
#[cfg(any(test, feature = "testing"))]
#[derive(Clone, Copy, Debug, Default)]
pub struct AllowAll;

#[cfg(any(test, feature = "testing"))]
impl Authorizer for AllowAll {
    fn authorize(
        &self,
        _metadata: &MetadataMap,
        actor: Option<&proto::Actor>,
        _operation: Operation<'_>,
    ) -> Result<Principal, Status> {
        Ok(Principal {
            actor: actor.map(|actor| format!("{}:{}", actor.domain, actor.id)),
        })
    }
}

/// Called after every change made through the service.
pub type OnChange<S> = Arc<dyn Fn(Arc<Change<S>>) + Send + Sync>;

/// The `botconf.v1.Settings` service over a panel and its settings store.
pub struct SettingsService<S: Schema> {
    panel: Arc<Panel<S>>,
    authorizer: Arc<dyn Authorizer>,
    names: Option<Arc<dyn Names>>,
    on_change: Option<OnChange<S>>,
}

impl<S: Schema> SettingsService<S> {
    /// Serves `panel` and its settings store, letting `authorizer` decide on
    /// every request. There is no default authorizer.
    pub fn new(panel: Arc<Panel<S>>, authorizer: impl Authorizer) -> Self {
        Self {
            panel,
            authorizer: Arc::new(authorizer),
            names: None,
            on_change: None,
        }
    }

    /// Where the panel gets names for references (e.g. chat titles), and the
    /// suggestions it offers when adding them. Without it, the panel shows
    /// ids.
    #[must_use]
    pub fn names(mut self, names: impl Names + 'static) -> Self {
        self.names = Some(Arc::new(names));
        self
    }

    /// Called after every change made through the service (by a setter, a
    /// reload, the panel or a text command), e.g. to refresh what depends on
    /// the settings. Changes made elsewhere are not reported here; use
    /// [`botconf::SettingsStore::subscribe`] for those.
    #[must_use]
    pub fn on_change(mut self, hook: impl Fn(Arc<Change<S>>) + Send + Sync + 'static) -> Self {
        self.on_change = Some(Arc::new(hook));
        self
    }

    /// The tonic service, to add to a server.
    pub fn into_server(self) -> proto::settings_server::SettingsServer<Self> {
        proto::settings_server::SettingsServer::new(self)
    }

    fn changed(&self, change: Option<Change<S>>) {
        if let (Some(hook), Some(change)) = (&self.on_change, change) {
            hook(Arc::new(change));
        }
    }
}
