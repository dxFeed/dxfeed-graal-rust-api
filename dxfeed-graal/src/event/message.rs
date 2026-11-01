//! Message events.

use dxfeed_graal_sys as sys;

use super::format::Time;

event_type! {
    head {
        /// A message with an attachment, which a feed does not keep (`Message`).
        Message, MessageRef, sys::dxfg_message_t, sys::DXFG_EVENT_MESSAGE, []
    }
    fields {
        /// The attachment, as UTF-8 text; `None` if none.
        #[doc(alias = "getAttachment")]
        attachment: attachment [attachment] = None,
    }
    derived {}
}

impl Message {
    /// A `Message` for `symbol` with `attachment`, as Java's `new Message(symbol, attachment)`.
    pub fn with_attachment(symbol: impl AsRef<str>, attachment: impl Into<String>) -> Self {
        Message {
            attachment: Some(attachment.into()),
            ..Message::new(symbol)
        }
    }
}

/// The text of Java's `Message.toString`, e.g. `Message{AAPL, eventTime=0, attachment=null}`.
impl std::fmt::Display for Message {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Message{{{}, eventTime={}, attachment={}}}",
            self.symbol,
            Time(self.event_time),
            self.attachment.as_deref().unwrap_or("null"),
        )
    }
}
