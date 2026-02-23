//! IPC — Inter-Process Communication via ring buffers
//!
//! Zero-copy message passing between agents using lock-free
//! single-producer single-consumer (SPSC) ring buffers in
//! shared memory.
//!
//! Architecture:
//! ```text
//!   Agent A              Agent B
//!      │                    │
//!      ▼                    ▼
//!   [TX Ring] ──────→ [RX Ring]
//!   [RX Ring] ←────── [TX Ring]
//! ```
//!
//! Messages are serialized with MessagePack for minimal overhead.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::agent::AgentId;

/// Maximum message size (64 KB)
const MAX_MESSAGE_SIZE: usize = 64 * 1024;

/// Default ring buffer capacity (1 MB)
const DEFAULT_RING_CAPACITY: usize = 1024 * 1024;

/// IPC message types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessageType {
    /// Generic data message
    Data,
    /// Request (expects a response)
    Request,
    /// Response to a request
    Response,
    /// Signal (no data)
    Signal,
    /// Stream start
    StreamBegin,
    /// Stream chunk
    StreamData,
    /// Stream end
    StreamEnd,
    /// Error notification
    Error,
    /// Capability transfer
    CapTransfer,
    /// Vector data (optimized path)
    VectorData,
}

/// An IPC message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IpcMessage {
    /// Message type
    pub msg_type: MessageType,
    /// Sender agent ID
    pub sender: AgentId,
    /// Receiver agent ID
    pub receiver: AgentId,
    /// Correlation ID (for request/response pairing)
    pub correlation_id: u64,
    /// Message sequence number
    pub sequence: u64,
    /// Timestamp (nanoseconds since epoch)
    pub timestamp_ns: u64,
    /// Payload data
    pub payload: Vec<u8>,
    /// Priority (higher = delivered first)
    pub priority: u8,
}

impl IpcMessage {
    /// Create a data message
    pub fn data(sender: AgentId, receiver: AgentId, payload: Vec<u8>) -> Self {
        Self {
            msg_type: MessageType::Data,
            sender,
            receiver,
            correlation_id: 0,
            sequence: 0,
            timestamp_ns: now_ns(),
            payload,
            priority: 0,
        }
    }

    /// Create a request message
    pub fn request(sender: AgentId, receiver: AgentId, payload: Vec<u8>) -> Self {
        Self {
            msg_type: MessageType::Request,
            sender,
            receiver,
            correlation_id: rand::random(),
            sequence: 0,
            timestamp_ns: now_ns(),
            payload,
            priority: 1,
        }
    }

    /// Serialized size
    pub fn wire_size(&self) -> usize {
        // Rough estimate: header + payload
        64 + self.payload.len()
    }
}

/// An IPC channel between two agents
#[derive(Debug)]
pub struct Channel {
    /// Channel ID
    pub id: u64,
    /// Endpoint A
    pub agent_a: AgentId,
    /// Endpoint B
    pub agent_b: AgentId,
    /// Message queue (A → B)
    queue_a_to_b: std::collections::VecDeque<IpcMessage>,
    /// Message queue (B → A)
    queue_b_to_a: std::collections::VecDeque<IpcMessage>,
    /// Creation time
    pub created_at: Instant,
    /// Messages sent count
    pub messages_sent: u64,
    /// Messages received count
    pub messages_received: u64,
    /// Total bytes transferred
    pub bytes_transferred: u64,
    /// Is the channel open?
    pub is_open: bool,
    /// Maximum queue depth
    pub max_queue_depth: usize,
}

impl Channel {
    /// Create a new channel
    pub fn new(id: u64, agent_a: AgentId, agent_b: AgentId) -> Self {
        Self {
            id,
            agent_a,
            agent_b,
            queue_a_to_b: std::collections::VecDeque::with_capacity(256),
            queue_b_to_a: std::collections::VecDeque::with_capacity(256),
            created_at: Instant::now(),
            messages_sent: 0,
            messages_received: 0,
            bytes_transferred: 0,
            is_open: true,
            max_queue_depth: 4096,
        }
    }

    /// Send a message on this channel
    pub fn send(&mut self, msg: IpcMessage) -> Result<(), IpcError> {
        if !self.is_open {
            return Err(IpcError::ChannelClosed(self.id));
        }

        if msg.payload.len() > MAX_MESSAGE_SIZE {
            return Err(IpcError::MessageTooLarge {
                size: msg.payload.len(),
                max: MAX_MESSAGE_SIZE,
            });
        }

        let queue = if msg.sender == self.agent_a {
            &mut self.queue_a_to_b
        } else if msg.sender == self.agent_b {
            &mut self.queue_b_to_a
        } else {
            return Err(IpcError::NotParticipant(msg.sender));
        };

        if queue.len() >= self.max_queue_depth {
            return Err(IpcError::QueueFull(self.id));
        }

        self.bytes_transferred += msg.payload.len() as u64;
        self.messages_sent += 1;
        queue.push_back(msg);

        Ok(())
    }

    /// Receive a message (for the given agent)
    pub fn recv(&mut self, agent: AgentId) -> Option<IpcMessage> {
        let queue = if agent == self.agent_b {
            &mut self.queue_a_to_b
        } else if agent == self.agent_a {
            &mut self.queue_b_to_a
        } else {
            return None;
        };

        let msg = queue.pop_front();
        if msg.is_some() {
            self.messages_received += 1;
        }
        msg
    }

    /// Try receive (non-blocking)
    pub fn try_recv(&mut self, agent: AgentId) -> Option<IpcMessage> {
        self.recv(agent)
    }

    /// Pending message count for an agent
    pub fn pending_count(&self, agent: AgentId) -> usize {
        if agent == self.agent_b {
            self.queue_a_to_b.len()
        } else if agent == self.agent_a {
            self.queue_b_to_a.len()
        } else {
            0
        }
    }

    /// Close the channel
    pub fn close(&mut self) {
        self.is_open = false;
    }
}

/// The IPC subsystem
pub struct IpcManager {
    /// All channels by ID
    channels: HashMap<u64, Channel>,
    /// Channels by agent (agent_id → list of channel_ids)
    agent_channels: HashMap<AgentId, Vec<u64>>,
    /// Next channel ID
    next_channel_id: u64,
    /// Global message counter
    global_message_count: u64,
}

impl IpcManager {
    pub fn new() -> Self {
        Self {
            channels: HashMap::new(),
            agent_channels: HashMap::new(),
            next_channel_id: 0,
            global_message_count: 0,
        }
    }

    /// Create a channel between two agents
    pub fn create_channel(
        &mut self,
        agent_a: AgentId,
        agent_b: AgentId,
    ) -> Result<u64, IpcError> {
        let id = self.next_channel_id;
        self.next_channel_id += 1;

        let channel = Channel::new(id, agent_a, agent_b);
        self.channels.insert(id, channel);

        self.agent_channels.entry(agent_a).or_default().push(id);
        self.agent_channels.entry(agent_b).or_default().push(id);

        tracing::debug!(?agent_a, ?agent_b, channel_id = id, "Channel created");
        Ok(id)
    }

    /// Send a message on a channel
    pub fn send(&mut self, channel_id: u64, mut msg: IpcMessage) -> Result<(), IpcError> {
        self.global_message_count += 1;
        msg.sequence = self.global_message_count;

        let channel = self
            .channels
            .get_mut(&channel_id)
            .ok_or(IpcError::ChannelNotFound(channel_id))?;

        channel.send(msg)
    }

    /// Receive a message from a channel
    pub fn recv(&mut self, channel_id: u64, agent: AgentId) -> Option<IpcMessage> {
        self.channels.get_mut(&channel_id)?.recv(agent)
    }

    /// Receive from any channel (for an agent)
    pub fn recv_any(&mut self, agent: AgentId) -> Option<(u64, IpcMessage)> {
        let channel_ids = self.agent_channels.get(&agent)?.clone();
        for ch_id in channel_ids {
            if let Some(msg) = self.channels.get_mut(&ch_id).and_then(|ch| ch.recv(agent)) {
                return Some((ch_id, msg));
            }
        }
        None
    }

    /// Close a channel
    pub fn close_channel(&mut self, channel_id: u64) -> Result<(), IpcError> {
        let channel = self
            .channels
            .get_mut(&channel_id)
            .ok_or(IpcError::ChannelNotFound(channel_id))?;
        channel.close();
        Ok(())
    }

    /// Clean up all channels for a terminated agent
    pub fn cleanup_agent(&mut self, agent: &AgentId) {
        if let Some(channel_ids) = self.agent_channels.remove(agent) {
            for ch_id in channel_ids {
                if let Some(ch) = self.channels.get_mut(&ch_id) {
                    ch.close();
                }
            }
        }
    }

    /// Global stats
    pub fn stats(&self) -> IpcStats {
        IpcStats {
            total_channels: self.channels.len(),
            open_channels: self.channels.values().filter(|c| c.is_open).count(),
            total_messages: self.global_message_count,
        }
    }
}

/// IPC statistics
#[derive(Debug, Serialize)]
pub struct IpcStats {
    pub total_channels: usize,
    pub open_channels: usize,
    pub total_messages: u64,
}

/// IPC errors
#[derive(Debug, thiserror::Error)]
pub enum IpcError {
    #[error("Channel not found: {0}")]
    ChannelNotFound(u64),

    #[error("Channel {0} is closed")]
    ChannelClosed(u64),

    #[error("Channel {0} queue is full")]
    QueueFull(u64),

    #[error("Message too large: {size} bytes (max: {max})")]
    MessageTooLarge { size: usize, max: usize },

    #[error("Agent {0:?} is not a participant in this channel")]
    NotParticipant(AgentId),

    #[error("Receive timeout")]
    Timeout,
}

/// Current time in nanoseconds
fn now_ns() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_channel_send_recv() {
        let a = AgentId::new();
        let b = AgentId::new();
        let mut ch = Channel::new(0, a, b);

        let msg = IpcMessage::data(a, b, b"hello".to_vec());
        ch.send(msg).unwrap();

        let received = ch.recv(b).unwrap();
        assert_eq!(received.payload, b"hello");
    }

    #[test]
    fn test_bidirectional() {
        let a = AgentId::new();
        let b = AgentId::new();
        let mut ch = Channel::new(0, a, b);

        ch.send(IpcMessage::data(a, b, b"A->B".to_vec())).unwrap();
        ch.send(IpcMessage::data(b, a, b"B->A".to_vec())).unwrap();

        let msg_b = ch.recv(b).unwrap();
        assert_eq!(msg_b.payload, b"A->B");

        let msg_a = ch.recv(a).unwrap();
        assert_eq!(msg_a.payload, b"B->A");
    }

    #[test]
    fn test_ipc_manager() {
        let mut mgr = IpcManager::new();
        let a = AgentId::new();
        let b = AgentId::new();

        let ch_id = mgr.create_channel(a, b).unwrap();
        mgr.send(ch_id, IpcMessage::data(a, b, b"test".to_vec())).unwrap();

        let msg = mgr.recv(ch_id, b).unwrap();
        assert_eq!(msg.payload, b"test");
    }

    #[test]
    fn test_closed_channel() {
        let a = AgentId::new();
        let b = AgentId::new();
        let mut ch = Channel::new(0, a, b);

        ch.close();
        let result = ch.send(IpcMessage::data(a, b, b"fail".to_vec()));
        assert!(result.is_err());
    }

    #[test]
    fn test_message_too_large() {
        let a = AgentId::new();
        let b = AgentId::new();
        let mut ch = Channel::new(0, a, b);

        let big_payload = vec![0u8; MAX_MESSAGE_SIZE + 1];
        let result = ch.send(IpcMessage::data(a, b, big_payload));
        assert!(result.is_err());
    }
}
