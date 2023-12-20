use std::{
    convert::{AsRef, From, Into},
    fmt,
    str,
};

use net_packet::{NetPacketGuard, take_small_packet};

use super::frame::{CloseFrame, Frame};
use crate::adapters::ws::error::{CapacityError, Error, Result};

mod string_collect {
    use utf8::DecodeError;

    use crate::adapters::ws::error::{Error, Result};

    #[derive(Debug)]
    pub struct StringCollector {
        data: String,
        incomplete: Option<utf8::Incomplete>,
    }

    impl StringCollector {
        pub fn new() -> Self {
            StringCollector { data: String::new(), incomplete: None }
        }

        pub fn len(&self) -> usize {
            self.data
                .len()
                .saturating_add(self.incomplete.map(|i| i.buffer_len as usize).unwrap_or(0))
        }

        pub fn extend<T: AsRef<[u8]>>(&mut self, tail: T) -> Result<()> {
            let mut input: &[u8] = tail.as_ref();

            if let Some(mut incomplete) = self.incomplete.take() {
                if let Some((result, rest)) = incomplete.try_complete(input) {
                    input = rest;
                    if let Ok(text) = result {
                        self.data.push_str(text);
                    } else {
                        return Err(Error::Utf8);
                    }
                } else {
                    input = &[];
                    self.incomplete = Some(incomplete);
                }
            }

            if !input.is_empty() {
                match utf8::decode(input) {
                    Ok(text) => {
                        self.data.push_str(text);
                        Ok(())
                    }
                    Err(DecodeError::Incomplete { valid_prefix, incomplete_suffix }) => {
                        self.data.push_str(valid_prefix);
                        self.incomplete = Some(incomplete_suffix);
                        Ok(())
                    }
                    Err(DecodeError::Invalid { valid_prefix, .. }) => {
                        self.data.push_str(valid_prefix);
                        Err(Error::Utf8)
                    }
                }
            } else {
                Ok(())
            }
        }

        pub fn into_string(self) -> Result<String> {
            if self.incomplete.is_some() {
                Err(Error::Utf8)
            } else {
                Ok(self.data)
            }
        }
    }
}

use self::string_collect::StringCollector;

/// A struct representing the incomplete message.
#[derive(Debug)]
pub struct IncompleteMessage {
    collector: IncompleteMessageCollector,
}

enum IncompleteMessageCollector {
    Text(StringCollector),
    Binary(NetPacketGuard),
}

impl IncompleteMessage {
    /// Create new.
    pub fn new(message_type: IncompleteMessageType) -> Self {
        IncompleteMessage {
            collector: match message_type {
                IncompleteMessageType::Binary => {
                    //
                    let buffer = take_small_packet();
                    IncompleteMessageCollector::Binary(buffer)
                },
                IncompleteMessageType::Text => {
                    IncompleteMessageCollector::Text(StringCollector::new())
                }
            },
        }
    }

    /// Get the current filled size of the buffer.
    pub fn len(&self) -> usize {
        match self.collector {
            IncompleteMessageCollector::Text(ref t) => t.len(),
            IncompleteMessageCollector::Binary(ref b) => b.buffer_raw_len(),
        }
    }

    /// Add more data to an existing message.
    pub fn extend<T: AsRef<[u8]>>(&mut self, tail: T, size_limit: Option<usize>) -> Result<()> {
        // Always have a max size. This ensures an error in case of concatenating two buffers
        // of more than `usize::max_value()` bytes in total.
        let max_size = size_limit.unwrap_or_else(usize::max_value);
        let my_size = self.len();
        let portion_size = tail.as_ref().len();
        // Be careful about integer overflows here.
        if my_size > max_size || portion_size > max_size - my_size {
            return Err(Error::Capacity(CapacityError::MessageTooLong {
                size: my_size + portion_size,
                max_size,
            }));
        }

        match self.collector {
            IncompleteMessageCollector::Binary(ref mut buffer) => {
                buffer.append_slice(tail.as_ref());
                Ok(())
            }
            IncompleteMessageCollector::Text(ref mut t) => t.extend(tail),
        }
    }

    /// Convert an incomplete message into a complete one.
    pub fn complete(self) -> Result<Message> {
        match self.collector {
            IncompleteMessageCollector::Binary(v) => Ok(Message::Binary(v)),
            IncompleteMessageCollector::Text(t) => {
                let text = t.into_string()?;
                Ok(Message::Text(text))
            }
        }
    }
}

impl fmt::Debug for IncompleteMessageCollector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let string = match self {
            IncompleteMessageCollector::Binary(buffer) => {
                format!("Binary:{}", buffer.buffer_raw_len())
            }
            other => {
                format!("{other:?}")
            }
        };
        write!(f, "IncompleteMessageCollector::{string}")
    }
}

/// The type of incomplete message.
pub enum IncompleteMessageType {
    Text,
    Binary,
}

/// An enum representing the various forms of a WebSocket message.
pub enum Message {
    /// A text WebSocket message
    Text(String),
    /// A binary WebSocket message
    Binary(NetPacketGuard),
    /// A ping message with the specified payload
    ///
    /// The payload here must have a length less than 125 bytes
    Ping(NetPacketGuard),
    /// A pong message with the specified payload
    ///
    /// The payload here must have a length less than 125 bytes
    Pong(NetPacketGuard),
    /// A close message with the optional close frame.
    Close(Option<CloseFrame>),
    /// Raw frame. Note, that you're not going to get this value while reading the message.
    Frame(Frame),
}

impl Message {
    /// Create a new text WebSocket message from a stringable.
    pub fn text<S>(string: S) -> Message
    where
        S: Into<String>,
    {
        Message::Text(string.into())
    }

    /// Create a new binary WebSocket message by converting to `Vec<u8>`.
    pub fn binary<B>(bin: B) -> Message
    where
        B: Into<Vec<u8>>,
    {
        let mut buffer = take_small_packet();
        let v = bin.into();
        buffer.append_slice(v.as_slice());
        Message::Binary(buffer)
    }

    /// Indicates whether a message is a text message.
    pub fn is_text(&self) -> bool {
        matches!(*self, Message::Text(_))
    }

    /// Indicates whether a message is a binary message.
    pub fn is_binary(&self) -> bool {
        matches!(*self, Message::Binary(_))
    }

    /// Indicates whether a message is a ping message.
    pub fn is_ping(&self) -> bool {
        matches!(*self, Message::Ping(_))
    }

    /// Indicates whether a message is a pong message.
    pub fn is_pong(&self) -> bool {
        matches!(*self, Message::Pong(_))
    }

    /// Indicates whether a message is a close message.
    pub fn is_close(&self) -> bool {
        matches!(*self, Message::Close(_))
    }

    /// Get the length of the WebSocket message.
    pub fn len(&self) -> usize {
        match *self {
            Message::Text(ref string) => string.len(),
            Message::Binary(ref buffer) | Message::Ping(ref buffer) | Message::Pong(ref buffer) => {
                buffer.buffer_raw_len()
            }
            Message::Close(ref data) => data.as_ref().map(|d| d.reason.buffer_raw_len()).unwrap_or(0),
            Message::Frame(ref frame) => frame.len(),
        }
    }

    /// Returns true if the WebSocket message has no content.
    /// For example, if the other side of the connection sent an empty string.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl From<String> for Message {
    fn from(string: String) -> Self {
        Message::text(string)
    }
}

impl<'s> From<&'s str> for Message {
    fn from(string: &'s str) -> Self {
        Message::text(string)
    }
}

impl<'b> From<&'b [u8]> for Message {
    fn from(data: &'b [u8]) -> Self {
        Message::binary(data)
    }
}

impl From<Vec<u8>> for Message {
    fn from(data: Vec<u8>) -> Self {
        Message::binary(data)
    }
}

impl fmt::Debug for Message {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let string = match self {
            Message::Binary(buffer) => {
                format!("Binary:{}", buffer.buffer_raw_len())
            }
            Message::Ping(buffer) => {
                format!("Ping:{}", buffer.buffer_raw_len())
            }
            Message::Pong(buffer) => {
                format!("Pong:{}", buffer.buffer_raw_len())
            }
            other => {
                format!("{other:?}")
            }
        };
        write!(f, "Message::{string}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display() {
        let t = Message::text("test".to_owned());
        assert_eq!(t.to_string(), "test".to_owned());

        let bin = Message::binary(vec![0, 1, 3, 4, 241]);
        assert_eq!(bin.to_string(), "Binary Data<length=5>".to_owned());
    }

    #[test]
    fn binary_convert() {
        let bin = [6u8, 7, 8, 9, 10, 241];
        let msg = Message::from(&bin[..]);
        assert!(msg.is_binary());
        assert!(msg.into_text().is_err());
    }

    #[test]
    fn binary_convert_vec() {
        let bin = vec![6u8, 7, 8, 9, 10, 241];
        let msg = Message::from(bin);
        assert!(msg.is_binary());
        assert!(msg.into_text().is_err());
    }

    #[test]
    fn binary_convert_into_vec() {
        let bin = vec![6u8, 7, 8, 9, 10, 241];
        let bin_copy = bin.clone();
        let msg = Message::from(bin);
        let serialized: Vec<u8> = msg.into();
        assert_eq!(bin_copy, serialized);
    }

    #[test]
    fn text_convert() {
        let s = "kiwotsukete";
        let msg = Message::from(s);
        assert!(msg.is_text());
    }
}
