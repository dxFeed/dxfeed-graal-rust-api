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
        version: i32 [version] = 0,
        /// The attachment, as UTF-8 text; `None` if none.
        attachment: attachment [attachment] = None,
    }
    derived {}
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
