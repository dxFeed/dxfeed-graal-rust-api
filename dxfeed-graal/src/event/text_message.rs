//! Text message events.

use dxfeed_graal_sys as sys;

use super::format::{Quoted, Time};

time_sequence_values! {
    time_sequence
    event_type! {
        head {
            /// A text message with a time, which a feed does not keep (`TextMessage`).
            TextMessage, TextMessageRef, sys::dxfg_text_message_t, sys::DXFG_EVENT_TEXT_MESSAGE, []
        }
        fields {
            /// The time and the sequence of the message, packed as [`time`](Self::time) and
            /// [`sequence`](Self::sequence) read them.
            #[doc(alias = "getTimeSequence")]
            time_sequence: i64 [time_sequence] = 0,
            /// The text; `None` if none.
            #[doc(alias = "getText")]
            text: string [text] = None,
        }
        derived {}
    }
}

impl TextMessage {
    /// A `TextMessage` for `symbol` with `time`, in milliseconds since the Unix epoch, and `text`, as Java's
    /// `new TextMessage(symbol, time, text)`; Java's constructor without a time gives time 0.
    pub fn with_text(symbol: impl AsRef<str>, time: i64, text: impl Into<String>) -> Self {
        let mut message = TextMessage {
            text: Some(text.into()),
            ..TextMessage::new(symbol)
        };
        message.set_time(time);
        message
    }
}

/// The text of Java's `TextMessage.toString`, e.g. `TextMessage{AAPL, eventTime=0, time=0, sequence=0, text='null'}`.
impl std::fmt::Display for TextMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "TextMessage{{{}, eventTime={}, time={}, sequence={}, text={}}}",
            self.symbol,
            Time(self.event_time),
            Time(self.time()),
            self.sequence(),
            Quoted(self.text.as_deref()),
        )
    }
}
