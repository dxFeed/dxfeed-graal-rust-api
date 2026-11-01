//! Configuration events.

use dxfeed_graal_sys as sys;

use super::format::Time;

event_type! {
    head {
        /// A configuration with a version and an attachment, the last of which a feed keeps (`Configuration`).
        Configuration, ConfigurationRef, sys::dxfg_configuration_t, sys::DXFG_EVENT_CONFIGURATION, []
    }
    fields {
        /// The version of the configuration.
        #[doc(alias = "getVersion")]
        version: i32 [version] = 0,
        /// The attachment, as UTF-8 text; `None` if none.
        #[doc(alias = "getAttachment")]
        attachment: attachment [attachment] = None,
    }
    derived {}
}

impl Configuration {
    /// A `Configuration` for `symbol` with `attachment` and `version`, as Java's
    /// `new Configuration(symbol, attachment, version)`; Java's constructor without a version gives version 0.
    pub fn with_attachment(symbol: impl AsRef<str>, attachment: impl Into<String>, version: i32) -> Self {
        Configuration {
            attachment: Some(attachment.into()),
            version,
            ..Configuration::new(symbol)
        }
    }
}

/// The text of Java's `Configuration.toString`, e.g. `Configuration{AAPL, eventTime=0, version=0, attachment=null}`.
impl std::fmt::Display for Configuration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Configuration{{{}, eventTime={}, version={}, attachment={}}}",
            self.symbol,
            Time(self.event_time),
            self.version,
            self.attachment.as_deref().unwrap_or("null"),
        )
    }
}
