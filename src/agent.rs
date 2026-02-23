//! Agent — The fundamental execution unit of Neural Kernel
//!
//! An Agent is an isolated process with:
//! - Unique identity (Ed25519 keypair)
//! - Capability set (what it's allowed to do)
//! - Priority level (scheduling weight)
//! - Memory quota (enforced by the memory manager)
//! - State machine lifecycle (Created → Ready → Running → Blocked → Terminated)

use std::collections::HashMap;
use std::fmt;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

use crate::capability::CapabilitySet;

/// Unique agent identifier — 128-bit random ID
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AgentId([u8; 16]);

impl AgentId {
    /// Generate a new random AgentId
    pub fn new() -> Self {
        let mut bytes = [0u8; 16];
        rand::Rng::fill(&mut rand::thread_rng(), &mut bytes);
        Self(bytes)
    }

    /// Create from raw bytes
    pub fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    /// Get the raw bytes
    pub fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }

    /// Short hex string (first 8 chars)
    pub fn short(&self) -> String {
        hex::encode(&self.0[..4])
    }
}

impl fmt::Debug for AgentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Agent({})", hex::encode(&self.0[..4]))
    }
}

impl fmt::Display for AgentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", hex::encode(&self.0))
    }
}

impl Default for AgentId {
    fn default() -> Self {
        Self::new()
    }
}

/// Agent lifecycle state machine
///
/// ```text
/// Created → Ready → Running ──→ Blocked → Ready
///                       │                     │
///                       └──→ Terminated ←─────┘
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentState {
    /// Agent has been created but not yet scheduled
    Created,
    /// Agent is ready to run (in the scheduler's ready queue)
    Ready,
    /// Agent is currently executing on a worker thread
    Running,
    /// Agent is waiting (I/O, IPC, timer, etc.)
    Blocked(BlockReason),
    /// Agent has terminated (exit code or error)
    Terminated(i32),
}

/// Why an agent is blocked
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BlockReason {
    /// Waiting for IPC message
    IpcWait,
    /// Waiting for I/O completion
    IoWait,
    /// Sleeping (timer)
    Sleep,
    /// Waiting for memory allocation
    MemoryWait,
    /// Waiting for capability grant
    CapabilityWait,
    /// Waiting for VectorFS operation
    VectorFsWait,
}

/// Agent priority levels (higher = more CPU time)
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Priority {
    /// Background tasks (lowest priority)
    Idle = 0,
    /// Normal agent priority
    Normal = 10,
    /// Elevated priority (e.g., user-facing queries)
    High = 20,
    /// Realtime priority (kernel agents only)
    Realtime = 30,
    /// Critical kernel operations
    Critical = 40,
}

impl Default for Priority {
    fn default() -> Self {
        Priority::Normal
    }
}

/// Resource quotas for an agent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceQuota {
    /// Maximum memory in bytes
    pub max_memory_bytes: usize,
    /// Maximum CPU time per scheduling quantum
    pub max_cpu_time: Duration,
    /// Maximum number of open file descriptors
    pub max_fds: usize,
    /// Maximum number of IPC channels
    pub max_ipc_channels: usize,
    /// Maximum vector storage (dimensions × count)
    pub max_vector_storage: usize,
    /// Network bandwidth limit (bytes/sec, 0 = unlimited)
    pub max_bandwidth: usize,
}

impl Default for ResourceQuota {
    fn default() -> Self {
        Self {
            max_memory_bytes: 256 * 1024 * 1024, // 256 MB
            max_cpu_time: Duration::from_millis(100),
            max_fds: 64,
            max_ipc_channels: 16,
            max_vector_storage: 1_000_000,
            max_bandwidth: 0, // unlimited
        }
    }
}

/// Agent execution statistics
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentStats {
    /// Total CPU time consumed
    pub total_cpu_time: Duration,
    /// Number of times scheduled
    pub schedule_count: u64,
    /// Number of syscalls made
    pub syscall_count: u64,
    /// Bytes allocated
    pub memory_allocated: usize,
    /// Peak memory usage
    pub peak_memory: usize,
    /// Number of IPC messages sent
    pub ipc_messages_sent: u64,
    /// Number of IPC messages received
    pub ipc_messages_received: u64,
    /// Number of vector operations
    pub vector_ops: u64,
}

/// An Agent — the primary execution unit
#[derive(Debug)]
pub struct Agent {
    /// Unique identifier
    pub id: AgentId,

    /// Human-readable name
    pub name: String,

    /// Current state
    pub state: AgentState,

    /// Scheduling priority
    pub priority: Priority,

    /// Capability set (permissions)
    pub capabilities: CapabilitySet,

    /// Resource quotas
    pub quota: ResourceQuota,

    /// Execution statistics
    pub stats: AgentStats,

    /// Creation timestamp
    pub created_at: Instant,

    /// Last state change timestamp
    pub last_state_change: Instant,

    /// Parent agent (if spawned by another agent)
    pub parent: Option<AgentId>,

    /// Child agents (spawned by this agent)
    pub children: Vec<AgentId>,

    /// Agent-local key-value metadata
    pub metadata: HashMap<String, String>,

    /// Virtual runtime (for CFS scheduling: tracks "fair" time)
    pub vruntime: u64,

    /// Time slice remaining in current quantum
    pub time_slice_remaining: Duration,
}

impl Agent {
    /// Create a new agent with default settings
    pub fn new(name: impl Into<String>) -> Self {
        let now = Instant::now();
        Self {
            id: AgentId::new(),
            name: name.into(),
            state: AgentState::Created,
            priority: Priority::Normal,
            capabilities: CapabilitySet::empty(),
            quota: ResourceQuota::default(),
            stats: AgentStats::default(),
            created_at: now,
            last_state_change: now,
            parent: None,
            children: Vec::new(),
            metadata: HashMap::new(),
            vruntime: 0,
            time_slice_remaining: Duration::from_millis(10),
        }
    }

    /// Create an agent with specific priority and capabilities
    pub fn with_config(
        name: impl Into<String>,
        priority: Priority,
        capabilities: CapabilitySet,
        quota: ResourceQuota,
    ) -> Self {
        let mut agent = Self::new(name);
        agent.priority = priority;
        agent.capabilities = capabilities;
        agent.quota = quota;
        agent
    }

    /// Transition to a new state
    pub fn transition(&mut self, new_state: AgentState) -> Result<(), AgentStateError> {
        use AgentState::*;

        let valid = match (&self.state, &new_state) {
            (Created, Ready) => true,
            (Ready, Running) => true,
            (Running, Ready) => true,         // preempted
            (Running, Blocked(_)) => true,    // blocked
            (Running, Terminated(_)) => true, // exited
            (Blocked(_), Ready) => true,      // unblocked
            (Blocked(_), Terminated(_)) => true, // killed while blocked
            _ => false,
        };

        if !valid {
            return Err(AgentStateError::InvalidTransition {
                from: self.state,
                to: new_state,
            });
        }

        self.state = new_state;
        self.last_state_change = Instant::now();
        Ok(())
    }

    /// Check if agent is alive (not terminated)
    pub fn is_alive(&self) -> bool {
        !matches!(self.state, AgentState::Terminated(_))
    }

    /// Check if agent is runnable
    pub fn is_runnable(&self) -> bool {
        matches!(self.state, AgentState::Ready)
    }

    /// Calculate the effective priority weight (used by scheduler)
    pub fn weight(&self) -> u64 {
        // CFS-inspired: weight = 1024 / (1 + vruntime_delta)
        // Higher priority agents get larger timeslices
        let base = match self.priority {
            Priority::Idle => 1,
            Priority::Normal => 10,
            Priority::High => 100,
            Priority::Realtime => 1000,
            Priority::Critical => 10000,
        };
        base
    }

    /// Update virtual runtime after execution
    pub fn charge_time(&mut self, elapsed: Duration) {
        let weight = self.weight().max(1);
        // vruntime grows slower for high-priority agents
        self.vruntime += (elapsed.as_nanos() as u64) / weight;
        self.stats.total_cpu_time += elapsed;
        self.stats.schedule_count += 1;
    }

    /// Get a summary for display
    pub fn summary(&self) -> AgentSummary {
        AgentSummary {
            id: self.id,
            name: self.name.clone(),
            state: self.state,
            priority: self.priority,
            cpu_time_ms: self.stats.total_cpu_time.as_millis() as u64,
            memory_bytes: self.stats.memory_allocated,
            syscalls: self.stats.syscall_count,
            uptime: self.created_at.elapsed(),
        }
    }
}

/// Compact agent display info
#[derive(Debug, Serialize)]
pub struct AgentSummary {
    pub id: AgentId,
    pub name: String,
    pub state: AgentState,
    pub priority: Priority,
    pub cpu_time_ms: u64,
    pub memory_bytes: usize,
    pub syscalls: u64,
    pub uptime: Duration,
}

/// Errors from agent state transitions
#[derive(Debug, thiserror::Error)]
pub enum AgentStateError {
    #[error("Invalid state transition: {from:?} → {to:?}")]
    InvalidTransition {
        from: AgentState,
        to: AgentState,
    },
}

/// hex encoding helper (inline to avoid extra dep)
mod hex {
    pub fn encode(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{:02x}", b)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_creation() {
        let agent = Agent::new("test-agent");
        assert_eq!(agent.state, AgentState::Created);
        assert_eq!(agent.priority, Priority::Normal);
        assert!(agent.is_alive());
        assert!(!agent.is_runnable());
    }

    #[test]
    fn test_valid_transitions() {
        let mut agent = Agent::new("test");
        assert!(agent.transition(AgentState::Ready).is_ok());
        assert!(agent.transition(AgentState::Running).is_ok());
        assert!(agent.transition(AgentState::Terminated(0)).is_ok());
    }

    #[test]
    fn test_invalid_transition() {
        let mut agent = Agent::new("test");
        // Cannot go directly from Created to Running
        assert!(agent.transition(AgentState::Running).is_err());
    }

    #[test]
    fn test_charge_time() {
        let mut agent = Agent::new("test");
        let old_vruntime = agent.vruntime;
        agent.charge_time(Duration::from_millis(10));
        assert!(agent.vruntime > old_vruntime);
        assert_eq!(agent.stats.schedule_count, 1);
    }

    #[test]
    fn test_priority_weights() {
        let idle = Agent::with_config("idle", Priority::Idle, CapabilitySet::empty(), ResourceQuota::default());
        let critical = Agent::with_config("crit", Priority::Critical, CapabilitySet::empty(), ResourceQuota::default());
        assert!(critical.weight() > idle.weight());
    }

    #[test]
    fn test_agent_id_uniqueness() {
        let id1 = AgentId::new();
        let id2 = AgentId::new();
        assert_ne!(id1, id2);
    }
}
