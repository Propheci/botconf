//! Where the base configuration (the layers below the stored overrides) comes
//! from.

use figment::Figment;

/// Why the base sources could not be read.
pub type BaseError = Box<dyn std::error::Error + Send + Sync>;

/// Where the base configuration comes from: the config file and the
/// environment, typically. The [`SettingsStore`](crate::SettingsStore) asks
/// for it when it loads and again on every
/// [`reload`](crate::SettingsStore::reload).
///
/// - A [`Figment`] is a **fixed** base: figment reads its providers when they
///   are merged, so it is a snapshot of the sources as they were then, and a
///   reload sees the same values.
/// - A closure returning a `Figment` is **re-read** on every reload, so edits
///   to the config file apply, e.g. `move || AppConfig::figment(&path)`.
pub trait BaseSource: Send + Sync + 'static {
    /// Builds the base, reading the sources again for a closure.
    fn figment(&self) -> Result<Figment, BaseError>;
}

/// A fixed base: every call returns the same values.
impl BaseSource for Figment {
    fn figment(&self) -> Result<Figment, BaseError> {
        Ok(self.clone())
    }
}

/// A base rebuilt on every call, e.g. `move || AppConfig::figment(&path)`.
impl<F, E> BaseSource for F
where
    F: Fn() -> Result<Figment, E> + Send + Sync + 'static,
    E: Into<BaseError>,
{
    fn figment(&self) -> Result<Figment, BaseError> {
        self().map_err(Into::into)
    }
}
