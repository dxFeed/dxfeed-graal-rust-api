//! Text configuration events.

use dxfeed_graal_sys as sys;

use super::format::{Quoted, Time};

time_sequence_values! {
    time_sequence
    event_type! {
        head {
            /// A configuration as text with a time and a version, the last of which a feed keeps
            /// (`TextConfiguration`).
            TextConfiguration, TextConfigurationRef, sys::dxfg_text_configuration_t, sys::DXFG_EVENT_TEXT_CONFIGURATION,
            []
        }
        fields {
            /// The time and the sequence of the configuration, packed as [`time`](Self::time) and
            /// [`sequence`](Self::sequence) read them.
            time_sequence: i64 [time_sequence] = 0,
            /// The version of the configuration.
            version: i32 [version] = 0,
            /// The text; `None` if none.
            text: string [text] = None,
        }
        derived {}
    }
}

/// The text of Java's `TextConfiguration.toString`, e.g. `TextConfiguration{AAPL, eventTime=0, time=0, ...}`.
impl std::fmt::Display for TextConfiguration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "TextConfiguration{{{}, eventTime={}, time={}, sequence={}, version={}, text={}}}",
            self.symbol,
            Time(self.event_time),
            Time(self.time()),
            self.sequence(),
            self.version,
            Quoted(self.text.as_deref()),
        )
    }
}
