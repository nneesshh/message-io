//! WebSocket handshake machine.

use log::*;
use net_packet::{take_small_packet, NetPacketGuard};
use std::io::{Read, Write};

use crate::adapters::ws::error::{Error, Result};
use crate::adapters::ws::util::NonBlockingResult;

/// A generic handshake state machine.
#[derive(Debug)]
pub struct HandshakeMachine<Stream> {
    stream: Stream,
    state: HandshakeState,
}

impl<Stream> HandshakeMachine<Stream> {
    /// Start reading data from the peer.
    pub fn start_read(stream: Stream) -> Self {
        let buffer = take_small_packet();
        Self { stream, state: HandshakeState::Reading(buffer, AttackCheck::new()) }
    }
    /// Start writing data to the peer.
    pub fn start_write<D: Into<Vec<u8>>>(stream: Stream, data: D) -> Self {
        let mut buffer = take_small_packet();
        let slice: Vec<u8> = data.into();
        buffer.append_slice(slice.as_slice());
        HandshakeMachine { stream, state: HandshakeState::Writing(buffer) }
    }
    /// Returns a shared reference to the inner stream.
    pub fn get_ref(&self) -> &Stream {
        &self.stream
    }
    /// Returns a mutable reference to the inner stream.
    pub fn get_mut(&mut self) -> &mut Stream {
        &mut self.stream
    }
}

impl<Stream: Read + Write> HandshakeMachine<Stream> {
    /// Perform a single handshake round.
    pub fn single_round<Obj: TryParse>(mut self) -> Result<RoundResult<Obj, Stream>> {
        trace!("Doing handshake round.");
        match self.state {
            HandshakeState::Reading(mut buf, mut attack_check) => {
                let count = buf.buffer_raw_len();
                if 0 == count {
                    //
                    Ok(RoundResult::WouldBlock(HandshakeMachine {
                        state: HandshakeState::Reading(buf, attack_check),
                        ..self
                    }))
                } else {
                    attack_check.check_incoming_packet_size(count)?;

                    // TODO: this is slow for big headers with too many small packets.
                    // The parser has to be reworked in order to work on streams instead
                    // of buffers.
                    Ok(if let Some((size, obj)) = Obj::try_parse(buf.peek())? {
                        buf.advance(size);
                        RoundResult::StageFinished(StageResult::DoneReading {
                            result: obj,
                            stream: self.stream,
                            tail: buf,
                        })
                    } else {
                        RoundResult::Incomplete(HandshakeMachine {
                            state: HandshakeState::Reading(buf, attack_check),
                            ..self
                        })
                    })
                }
            }
            HandshakeState::Writing(mut buf) => {
                assert!(buf.has_remaining());
                if let Some(size) = self.stream.write(buf.peek()).no_block()? {
                    assert!(size > 0);
                    buf.advance(size);
                    Ok(if buf.has_remaining() {
                        RoundResult::Incomplete(HandshakeMachine {
                            state: HandshakeState::Writing(buf),
                            ..self
                        })
                    } else {
                        RoundResult::StageFinished(StageResult::DoneWriting(self.stream))
                    })
                } else {
                    Ok(RoundResult::WouldBlock(HandshakeMachine {
                        state: HandshakeState::Writing(buf),
                        ..self
                    }))
                }
            }
        }
    }
}

/// The result of the round.
#[derive(Debug)]
pub enum RoundResult<Obj, Stream> {
    /// Round not done, I/O would block.
    WouldBlock(HandshakeMachine<Stream>),
    /// Round done, state unchanged.
    Incomplete(HandshakeMachine<Stream>),
    /// Stage complete.
    StageFinished(StageResult<Obj, Stream>),
}

/// The result of the stage.
pub enum StageResult<Obj, Stream> {
    /// Reading round finished.
    #[allow(missing_docs)]
    DoneReading { result: Obj, stream: Stream, tail: NetPacketGuard },
    /// Writing round finished.
    DoneWriting(Stream),
}

impl<Obj, Stream> std::fmt::Debug for StageResult<Obj, Stream> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let string = match self {
            Self::DoneReading { result: _, stream: _, tail } => {
                format!("DoneReading::(tail={})", tail.buffer_raw_len())
            }
            Self::DoneWriting(_) => {
                format!("DoneWriting")
            }
        };
        write!(f, "StageResult::{string}")
    }
}

/// The parseable object.
pub trait TryParse: Sized {
    /// Return Ok(None) if incomplete, Err on syntax error.
    fn try_parse(data: &[u8]) -> Result<Option<(usize, Self)>>;
}

/// The handshake state.
enum HandshakeState {
    /// Reading data from the peer.
    Reading(NetPacketGuard, AttackCheck),
    /// Sending data to the peer.
    Writing(NetPacketGuard),
}

impl std::fmt::Debug for HandshakeState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let string = match self {
            Self::Reading(buffer, _) => {
                format!("Reading::({})", buffer.buffer_raw_len())
            }
            Self::Writing(buffer) => {
                format!("Writing::({})", buffer.buffer_raw_len())
            }
        };
        write!(f, "HandshakeState::{string}")
    }
}

/// Attack mitigation. Contains counters needed to prevent DoS attacks
/// and reject valid but useless headers.
#[derive(Debug)]
pub(crate) struct AttackCheck {
    /// Number of HTTP header successful reads (TCP packets).
    number_of_packets: usize,
    /// Total number of bytes in HTTP header.
    number_of_bytes: usize,
}

impl AttackCheck {
    /// Initialize attack checking for incoming buffer.
    fn new() -> Self {
        Self { number_of_packets: 0, number_of_bytes: 0 }
    }

    /// Check the size of an incoming packet. To be called immediately after `read()`
    /// passing its returned bytes count as `size`.
    fn check_incoming_packet_size(&mut self, size: usize) -> Result<()> {
        self.number_of_packets += 1;
        self.number_of_bytes += size;

        // TODO: these values are hardcoded. Instead of making them configurable,
        // rework the way HTTP header is parsed to remove this check at all.
        const MAX_BYTES: usize = 65536;
        const MAX_PACKETS: usize = 512;
        const MIN_PACKET_SIZE: usize = 128;
        const MIN_PACKET_CHECK_THRESHOLD: usize = 64;

        if self.number_of_bytes > MAX_BYTES {
            return Err(Error::AttackAttempt);
        }

        if self.number_of_packets > MAX_PACKETS {
            return Err(Error::AttackAttempt);
        }

        if self.number_of_packets > MIN_PACKET_CHECK_THRESHOLD {
            if self.number_of_packets * MIN_PACKET_SIZE > self.number_of_bytes {
                return Err(Error::AttackAttempt);
            }
        }

        Ok(())
    }
}
