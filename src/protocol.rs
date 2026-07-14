//! Definition of the Lidi protocol used to transfer data over UDP
//!
//! The Lidi protocol is rather simple: since the communications are unidirectional, it is defined
//! by the blocks structure. There are 5 block types:
//! - `BlockType::Heartbeat` lets know the receiver that transfer can happen,
//! - `BlockType::Start` informs the receiver that the sent data chunk represents the beginning of
//!   a new transfer,
//! - `BlockType::Data` is used to send a data chunk that is not the beginning nor the ending of
//!   a transfer,
//! - `BlockType::Abort` informs the receiver that the current transfer has been aborted on the
//!   sender side,
//! - `BlockType::End` informs the receiver that the current transfer is completed (i.e. all
//!   data have been sent).
//!
//! A block is stored in a `Vec` of `u8`s, with the following representation:
//!
//! ```text
//!
//! <-- 4 bytes -> <-- 1 byte --> <-- 4 bytes -->
//! --------------+--------------+---------------+--------------------------------------
//! |             |              |               |                                     |
//! |  client_id  |  block_type  |  data_length  |  payload = data + optional padding  |
//! |             |              |               |                                     |
//! --------------+--------------+---------------+--------------------------------------
//!  <----------- SERIALIZE_OVERHEAD -----------> <----------- block_length ----------->
//!
//! ```
//!
//! 4-bytes values are encoded in little-endian byte order.
//!
//! In `Heartbeat` blocks, `client_id` is unused and should be set to 0 by the constructor
//! caller. Also no data payload should be provided by the constructor caller in case the block
//! is of type `Heartbeat`, `Abort` or `End`. Then the `data_length` will be set to 0 by the
//! block constructor and the data chunk will be fully padded with zeros.

use std::{fmt, io, num, sync};

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    InvalidBlockType(Option<u8>),
    InvalidBlockSize { actual: usize, expected: usize },
    InvalidPayloadLength { actual: u32, max: usize },
    Other(String),
}

impl fmt::Display for Error {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> Result<(), std::fmt::Error> {
        match self {
            Self::Io(e) => write!(fmt, "I/O error: {e}"),
            Self::InvalidBlockType(b) => write!(fmt, "invalid block type: {b:?}"),
            Self::InvalidBlockSize { actual, expected } => {
                write!(fmt, "invalid block size: {actual} != {expected}")
            }
            Self::InvalidPayloadLength { actual, max } => {
                write!(fmt, "invalid payload length: {actual} > {max}")
            }
            Self::Other(e) => write!(fmt, "{e}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::InvalidBlockType(_)
            | Self::InvalidBlockSize { .. }
            | Self::InvalidPayloadLength { .. }
            | Self::Other(_) => None,
        }
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<num::TryFromIntError> for Error {
    fn from(e: num::TryFromIntError) -> Self {
        Self::Other(e.to_string())
    }
}

const PACKET_HEADER_SIZE: u16 = 20 + 8;
const RAPTORQ_ALIGNMENT: u16 = 8;
const RAPTORQ_HEADER_SIZE: u16 = 4;

pub struct RaptorQ {
    max_packet_size: u16,
    symbol_count: u16,
    transfer_length: u32,
    plan: raptorq::SourceBlockEncodingPlan,
    config: raptorq::ObjectTransmissionInformation,
    nb_repair_packets: u16,
    min_nb_repair_packets: u16,
}

impl RaptorQ {
    /// # Errors
    ///
    /// Will return `Err` if `symbol_count`
    ///   or
    /// `nb_repair_packets` parsing fails
    pub fn new(
        mtu: u16,
        block_size: u32,
        repair_percentage: u32,
        min_repair_percentage: u32,
    ) -> Result<Self, Error> {
        let mut max_packet_size = mtu - PACKET_HEADER_SIZE - RAPTORQ_HEADER_SIZE;
        max_packet_size -= max_packet_size % RAPTORQ_ALIGNMENT;

        let symbol_count = u16::try_from(block_size / u32::from(max_packet_size))
            .map_err(|e| Error::Other(format!("symbol_count: {e}")))?;

        let transfer_length = u32::from(max_packet_size) * u32::from(symbol_count);

        log::debug!("generating source encoding plan...");
        let plan = raptorq::SourceBlockEncodingPlan::generate(symbol_count);
        log::debug!("source encoding plan generated");

        let config = raptorq::ObjectTransmissionInformation::with_defaults(
            u64::from(transfer_length),
            max_packet_size,
        );

        let mut nb_repair_packets = u16::try_from(
            ((transfer_length / 100) * repair_percentage) / u32::from(max_packet_size),
        )
        .map_err(|e| Error::Other(format!("nb_repair_packets: {e}")))?;

        let min_nb_repair_packets = u16::try_from(
            ((transfer_length / 100) * min_repair_percentage) / u32::from(max_packet_size),
        )
        .map_err(|e| Error::Other(format!("min_nb_repair_packets: {e}")))?;

        if nb_repair_packets < min_nb_repair_packets {
            nb_repair_packets = min_nb_repair_packets;
        }

        Ok(Self {
            max_packet_size,
            symbol_count,
            transfer_length,
            plan,
            config,
            nb_repair_packets,
            min_nb_repair_packets,
        })
    }

    #[must_use]
    pub const fn block_size(&self) -> u32 {
        self.transfer_length
    }

    #[must_use]
    pub const fn min_nb_packets(&self) -> u16 {
        // we require to have at least min_nb_repair_packets packets
        // in addition to normal packets to improve integrity of
        // RaptorQ decoding process
        self.symbol_count + self.min_nb_repair_packets
    }

    #[must_use]
    pub fn nb_packets(&self) -> u32 {
        u32::from(self.symbol_count) + u32::from(self.nb_repair_packets)
    }

    #[must_use]
    pub fn encode(&self, block_id: u8, data: &[u8]) -> Vec<raptorq::EncodingPacket> {
        let encoder = raptorq::SourceBlockEncoder::with_encoding_plan(
            block_id,
            &self.config,
            data,
            &self.plan,
        );
        let mut packets = encoder.source_packets();
        if 0 < self.nb_repair_packets {
            packets.extend(encoder.repair_packets(
                u32::from(self.config.symbol_size()),
                u32::from(self.nb_repair_packets),
            ));
        }
        packets
    }

    #[must_use]
    pub fn decode(&self, block_id: u8, packets: Vec<raptorq::EncodingPacket>) -> Option<Vec<u8>> {
        let mut decoder = raptorq::SourceBlockDecoder::new(
            block_id,
            &self.config,
            u64::from(self.transfer_length),
        );
        decoder.decode(packets)
    }
}

impl fmt::Display for RaptorQ {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(
            fmt,
            "RaptorQ max_packet_size == {} transfer_length = {} symbol_count|nb_packets == {} nb_repair_packets == {} min_nb_repair_packets == {}",
            self.max_packet_size,
            self.transfer_length,
            self.symbol_count,
            self.nb_repair_packets,
            self.min_nb_repair_packets
        )
    }
}

pub(crate) enum BlockType {
    Heartbeat,
    Start,
    Data,
    Abort,
    End,
}

impl BlockType {
    const fn serialized(self) -> u8 {
        match self {
            Self::Heartbeat => ID_HEARTBEAT,
            Self::Start => ID_START,
            Self::Data => ID_DATA,
            Self::Abort => ID_ABORT,
            Self::End => ID_END,
        }
    }
}

impl fmt::Display for BlockType {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        match self {
            Self::Heartbeat => write!(fmt, "Heartbeat"),
            Self::Start => write!(fmt, "Start"),
            Self::Data => write!(fmt, "Data"),
            Self::Abort => write!(fmt, "Abort"),
            Self::End => write!(fmt, "End"),
        }
    }
}

const ID_HEARTBEAT: u8 = 0x00;
const ID_START: u8 = 0x01;
const ID_DATA: u8 = 0x02;
const ID_ABORT: u8 = 0x03;
const ID_END: u8 = 0x04;

pub type ClientId = u32;

static CLIENT_ID_COUNTER: sync::atomic::AtomicU32 = sync::atomic::AtomicU32::new(0);

pub(crate) fn new_client_id() -> ClientId {
    CLIENT_ID_COUNTER.fetch_add(1, sync::atomic::Ordering::Relaxed)
}

pub(crate) struct Block(Vec<u8>);

const SERIALIZE_OVERHEAD: usize = 4 + 1 + 4;

impl Block {
    /// Block constructor, craft a block according to the representation introduced in
    /// [`crate::protocol`].
    ///
    /// Some (unchecked) constraints on arguments must be respected:
    /// - if `block` is `BlockType::Heartbeat`, `BlockType::Abort` or `BlockType::End`
    ///   then no data should be provided,
    /// - if `block` is `BlockType::Heartbeat` then `client_id` should be equal to 0,
    /// - if there is some `data`, its length must be lower than `Messsage::max_data_len()`.
    pub(crate) fn new(
        block: BlockType,
        raptorq: &RaptorQ,
        client_id: ClientId,
        data: Option<&[u8]>,
    ) -> Result<Self, Error> {
        match data {
            None => {
                let mut content = vec![
                    0u8;
                    usize::try_from(raptorq.transfer_length).map_err(|e| {
                        Error::Other(format!("transfer_length: {e}"))
                    })?
                ];
                let bytes = client_id.to_le_bytes();
                content[0] = bytes[0];
                content[1] = bytes[1];
                content[2] = bytes[2];
                content[3] = bytes[3];
                content[4] = block.serialized();
                Ok(Self(content))
            }
            Some(data) => {
                let max_data_len = Self::max_data_len(raptorq);
                if data.len() > max_data_len {
                    return Err(Error::InvalidPayloadLength {
                        actual: u32::try_from(data.len())
                            .map_err(|e| Error::Other(format!("data.len(): {e}")))?,
                        max: max_data_len,
                    });
                }

                let mut content = Vec::with_capacity(
                    usize::try_from(raptorq.transfer_length)
                        .map_err(|e| Error::Other(format!("transfer_length: {e}")))?,
                );
                content.extend_from_slice(&client_id.to_le_bytes());
                content.push(block.serialized());
                content.extend_from_slice(&u32::to_le_bytes(
                    u32::try_from(data.len())
                        .map_err(|e| Error::Other(format!("data.len(): {e}")))?,
                ));
                content.extend_from_slice(data);
                if content.len() < content.capacity() {
                    content.resize(content.capacity(), 0);
                }
                Ok(Self(content))
            }
        }
    }

    pub(crate) fn validate(&self, raptorq: &RaptorQ) -> Result<(), Error> {
        let expected_len = usize::try_from(raptorq.transfer_length)
            .map_err(|e| Error::Other(format!("transfer_length: {e}")))?;
        if self.0.len() != expected_len {
            return Err(Error::InvalidBlockSize {
                actual: self.0.len(),
                expected: expected_len,
            });
        }
        if self.0.len() < SERIALIZE_OVERHEAD {
            return Err(Error::Other("block shorter than header".into()));
        }
        let payload_len = self.payload_len()?;
        let max_payload = u32::try_from(Self::max_data_len(raptorq))
            .map_err(|e| Error::Other(format!("max_data_len: {e}")))?;
        if payload_len > max_payload {
            return Err(Error::InvalidPayloadLength {
                actual: payload_len,
                max: Self::max_data_len(raptorq),
            });
        }
        Ok(())
    }

    pub(crate) fn client_id(&self) -> Result<ClientId, Error> {
        let bytes = self
            .0
            .get(0..4)
            .ok_or_else(|| Error::Other("block shorter than client_id".into()))?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    pub(crate) fn block_type(&self) -> Result<BlockType, Error> {
        match self.0.get(4) {
            Some(&ID_HEARTBEAT) => Ok(BlockType::Heartbeat),
            Some(&ID_START) => Ok(BlockType::Start),
            Some(&ID_DATA) => Ok(BlockType::Data),
            Some(&ID_ABORT) => Ok(BlockType::Abort),
            Some(&ID_END) => Ok(BlockType::End),
            b => Err(Error::InvalidBlockType(b.copied())),
        }
    }

    fn payload_len(&self) -> Result<u32, Error> {
        let data_len_bytes = self
            .0
            .get(5..9)
            .ok_or_else(|| Error::Other("block shorter than payload length".into()))?;
        Ok(u32::from_le_bytes([
            data_len_bytes[0],
            data_len_bytes[1],
            data_len_bytes[2],
            data_len_bytes[3],
        ]))
    }

    pub(crate) const fn deserialize(data: Vec<u8>) -> Self {
        Self(data)
    }

    pub const fn max_data_len(raptorq: &RaptorQ) -> usize {
        raptorq.transfer_length as usize - SERIALIZE_OVERHEAD
    }

    pub(crate) fn payload(&self) -> Result<&[u8], Error> {
        let len = usize::try_from(self.payload_len()?)
            .map_err(|e| Error::Other(format!("payload_len: {e}")))?;
        self.0
            .get(SERIALIZE_OVERHEAD..SERIALIZE_OVERHEAD.saturating_add(len))
            .ok_or_else(|| Error::Other("payload out of bounds".into()))
    }

    pub(crate) fn serialized(&self) -> &[u8] {
        &self.0
    }
}

impl fmt::Display for Block {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        let msg_type = match self.block_type() {
            Err(e) => format!("UNKNOWN {e}"),
            Ok(t) => t.to_string(),
        };
        write!(
            fmt,
            "client {:x} block = {} data = {} byte(s)",
            self.client_id().unwrap_or(0),
            msg_type,
            self.payload_len().unwrap_or(0)
        )
    }
}

#[cfg(test)]
mod repro {
    use super::{Block, Error, RaptorQ, ID_START};

    fn test_raptorq() -> RaptorQ {
        match RaptorQ::new(1500, 65_536, 10, 5) {
            Ok(raptorq) => raptorq,
            Err(error) => panic!("raptorq config: {error}"),
        }
    }

    fn craft_block_with_claimed_payload_len(raptorq: &RaptorQ, claimed_len: u32) -> Block {
        let mut bytes = vec![0u8; raptorq.transfer_length as usize];
        bytes[4] = ID_START;
        bytes[5..9].copy_from_slice(&claimed_len.to_le_bytes());
        Block::deserialize(bytes)
    }

    /// Unpatched `payload()` indexes past the block buffer when `data_length` is forged.
    #[test]
    fn repro_oversized_payload_claim_panics_without_bounds_check() {
        let raptorq = test_raptorq();
        let claimed = u32::try_from(Block::max_data_len(&raptorq) + 1024).expect("claimed len");
        let block_len = raptorq.transfer_length as usize;
        let end = 9usize.saturating_add(claimed as usize);
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = &vec![0u8; block_len][9..end];
        }));
        assert!(panic.is_err(), "forged payload length must panic on unpatched slice");
    }

    #[test]
    fn repro_oversized_payload_claim_returns_error() {
        let raptorq = test_raptorq();
        let claimed = u32::try_from(Block::max_data_len(&raptorq) + 1024).expect("claimed len");
        let block = craft_block_with_claimed_payload_len(&raptorq, claimed);
        match block.payload() {
            Err(Error::Other(message)) if message.contains("payload out of bounds") => {}
            Err(Error::InvalidPayloadLength { .. }) => {}
            Ok(_) => panic!("expected payload extraction to fail"),
            Err(other) => panic!("unexpected payload error: {other}"),
        }
    }

    #[test]
    fn repro_validate_rejects_oversized_payload_claim() {
        let raptorq = test_raptorq();
        let claimed = u32::try_from(Block::max_data_len(&raptorq) + 1024).expect("claimed len");
        let block = craft_block_with_claimed_payload_len(&raptorq, claimed);
        assert!(matches!(
            block.validate(&raptorq),
            Err(Error::InvalidPayloadLength { .. })
        ));
    }

    #[test]
    fn repro_validate_rejects_wrong_block_size() {
        let raptorq = test_raptorq();
        let block = Block::deserialize(vec![0u8; 32]);
        assert!(matches!(
            block.validate(&raptorq),
            Err(Error::InvalidBlockSize { .. })
        ));
    }
}
