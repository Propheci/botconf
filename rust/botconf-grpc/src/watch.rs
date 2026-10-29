//! `Watch`: the values under some key prefixes now, then every change to
//! them, from the store's published snapshots.

use std::{collections::BTreeSet, sync::Arc};

use botconf::{Catalog, Schema, SettingsStore, Snapshot, keys::is_below};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::Status;

use crate::{convert, proto};

/// Events waiting for a slow client; the store never waits for a watcher,
/// which skips to the latest snapshot when it falls behind.
const BUFFER: usize = 16;

/// Whether `key` is watched: at or below one of `prefixes`, or anything when
/// there are none.
fn watched(key: &str, prefixes: &[String]) -> bool {
    prefixes.is_empty()
        || prefixes
            .iter()
            .any(|prefix| key == prefix || is_below(key, prefix))
}

/// The watched keys of `catalog`, in order.
fn keys(catalog: &Catalog, prefixes: &[String]) -> BTreeSet<String> {
    catalog
        .live()
        .map(|(_, entry)| entry.key.clone())
        .filter(|key| watched(key, prefixes))
        .collect()
}

/// The event telling what changed between two snapshots, if anything did.
fn changes<S: Schema>(
    before: (&Snapshot<S>, &Arc<Catalog>, &BTreeSet<String>),
    after: (&Snapshot<S>, &Arc<Catalog>, &BTreeSet<String>),
) -> Option<proto::WatchEvent> {
    let (old, old_catalog, old_keys) = before;
    let (new, new_catalog, new_keys) = after;

    let catalog_changed = !Arc::ptr_eq(old_catalog, new_catalog) && old_keys != new_keys;
    let mut values: Vec<_> = new_keys
        .iter()
        .filter(|key| {
            !old_keys.contains(*key)
                || old.value(key) != new.value(key)
                || old.source(key) != new.source(key)
        })
        .map(|key| convert::current(new, key))
        .collect();
    // Keys that went away (their section was removed) have no value now.
    values.extend(old_keys.difference(new_keys).map(|key| proto::ValueEntry {
        key: key.clone(),
        source: proto::Source::Default.into(),
        ..Default::default()
    }));

    (catalog_changed || !values.is_empty()).then_some(proto::WatchEvent {
        values,
        initial: false,
        catalog_changed,
    })
}

/// Streams the watched values of `store`: first every one, then the changed
/// ones, until the client goes away.
pub(crate) fn stream<S: Schema>(
    store: Arc<SettingsStore<S>>,
    prefixes: Vec<String>,
) -> ReceiverStream<Result<proto::WatchEvent, Status>> {
    let (events, receiver) = mpsc::channel(BUFFER);
    // Subscribed before reading the first snapshot, so that no change falls
    // between the initial event and the first change event.
    let mut published = store.subscribe();
    let mut snapshot = Arc::clone(&published.borrow_and_update());
    let mut catalog = store.catalog();
    let mut watched_keys = keys(&catalog, &prefixes);

    tokio::spawn(async move {
        let initial = proto::WatchEvent {
            values: watched_keys
                .iter()
                .map(|key| convert::current(&snapshot, key))
                .collect(),
            initial: true,
            catalog_changed: false,
        };
        if events.send(Ok(initial)).await.is_err() {
            return;
        }

        loop {
            tokio::select! {
                () = events.closed() => return,
                changed = published.changed() => {
                    if changed.is_err() {
                        return;
                    }
                }
            }
            let next = Arc::clone(&published.borrow_and_update());
            let next_catalog = store.catalog();
            let next_keys = keys(&next_catalog, &prefixes);
            let event = changes(
                (&snapshot, &catalog, &watched_keys),
                (&next, &next_catalog, &next_keys),
            );
            (snapshot, catalog, watched_keys) = (next, next_catalog, next_keys);
            if let Some(event) = event
                && events.send(Ok(event)).await.is_err()
            {
                return;
            }
        }
    });

    ReceiverStream::new(receiver)
}

#[cfg(test)]
mod tests {
    use super::watched;

    #[test]
    fn prefixes_match_whole_segments() {
        let prefixes = ["connectors.tg".to_string()];
        assert!(watched("connectors.tg", &prefixes));
        assert!(watched("connectors.tg.rendering.edit_mode", &prefixes));
        assert!(!watched("connectors.tg2.limit", &prefixes));
        assert!(!watched("connectors", &prefixes));
        assert!(watched("anything", &[]));
    }
}
