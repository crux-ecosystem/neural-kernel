//! Syscall Table — The kernel-agent interface
//!
//! All agent ↔ kernel communication happens through syscalls.
//! Each syscall checks capabilities before execution.
//!
//! Syscall numbers:
//! ```text
//!   0x00-0x0F  Process management (spawn, exit, yield, sleep)
//!   0x10-0x1F  Memory operations (alloc, free, mmap, munmap)
//!   0x20-0x2F  IPC (send, recv, create_channel, close_channel)
//!   0x30-0x3F  VectorFS (open, close, read, write, search, cluster)
//!   0x40-0x4F  Network (connect, listen, send, recv)
//!   0x50-0x5F  Crypto (encrypt, decrypt, sign, verify, keygen)
//!   0x60-0x6F  Agent management (inspect, kill, delegate_cap)
//!   0xF0-0xFF  Kernel internal (reserved)
//! ```


use serde::{Deserialize, Serialize};

use crate::capability::Capability;

/// Syscall numbers
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum Syscall {
    // ── Process Management (0x00-0x0F) ──
    /// Spawn a child agent
    Spawn = 0x00,
    /// Exit current agent
    Exit = 0x01,
    /// Yield remaining time slice
    Yield = 0x02,
    /// Sleep for a duration
    Sleep = 0x03,
    /// Get own agent ID
    GetId = 0x04,
    /// Get own capabilities
    GetCaps = 0x05,
    /// Fork (clone) self
    Fork = 0x06,

    // ── Memory (0x10-0x1F) ──
    /// Allocate memory
    Alloc = 0x10,
    /// Free memory
    Free = 0x11,
    /// Map a shared memory region
    Mmap = 0x12,
    /// Unmap a shared memory region
    Munmap = 0x13,
    /// Query memory usage
    MemInfo = 0x14,

    // ── IPC (0x20-0x2F) ──
    /// Send a message to another agent
    IpcSend = 0x20,
    /// Receive a message (blocking)
    IpcRecv = 0x21,
    /// Receive a message (non-blocking)
    IpcTryRecv = 0x22,
    /// Create an IPC channel
    IpcCreateChannel = 0x23,
    /// Close an IPC channel
    IpcCloseChannel = 0x24,

    // ── VectorFS (0x30-0x3F) ──
    /// Open a file/vector
    FsOpen = 0x30,
    /// Close a file handle
    FsClose = 0x31,
    /// Read data
    FsRead = 0x32,
    /// Write data
    FsWrite = 0x33,
    /// Similarity search
    FsSearch = 0x34,
    /// Create a cluster (directory)
    FsCreateCluster = 0x35,
    /// Delete a file/vector
    FsDelete = 0x36,
    /// List directory entries
    FsList = 0x37,
    /// Get file metadata
    FsStat = 0x38,

    // ── Network (0x40-0x4F) ──
    /// Open a connection
    NetConnect = 0x40,
    /// Start listening
    NetListen = 0x41,
    /// Send data over network
    NetSend = 0x42,
    /// Receive data from network
    NetRecv = 0x43,
    /// Close connection
    NetClose = 0x44,

    // ── Crypto (0x50-0x5F) ──
    /// Encrypt data
    CryptoEncrypt = 0x50,
    /// Decrypt data
    CryptoDecrypt = 0x51,
    /// Sign data
    CryptoSign = 0x52,
    /// Verify signature
    CryptoVerify = 0x53,
    /// Generate key pair
    CryptoKeygen = 0x54,
    /// Hash data (BLAKE3)
    CryptoHash = 0x55,

    // ── Agent Management (0x60-0x6F) ──
    /// Inspect another agent's status
    AgentInspect = 0x60,
    /// Kill another agent
    AgentKill = 0x61,
    /// Delegate a capability to a child
    CapDelegate = 0x62,
    /// Revoke a delegated capability
    CapRevoke = 0x63,
}

impl Syscall {
    /// Get the required capability for this syscall
    pub fn required_capability(&self) -> Capability {
        match self {
            // Process
            Syscall::Spawn => Capability::Spawn,
            Syscall::Exit => Capability::Execute,
            Syscall::Yield => Capability::Execute,
            Syscall::Sleep => Capability::Execute,
            Syscall::GetId => Capability::Execute,
            Syscall::GetCaps => Capability::Execute,
            Syscall::Fork => Capability::Fork,

            // Memory
            Syscall::Alloc => Capability::MemAlloc,
            Syscall::Free => Capability::MemAlloc,
            Syscall::Mmap => Capability::MemMap,
            Syscall::Munmap => Capability::MemMap,
            Syscall::MemInfo => Capability::MemRead,

            // IPC
            Syscall::IpcSend => Capability::IpcSend,
            Syscall::IpcRecv => Capability::IpcRecv,
            Syscall::IpcTryRecv => Capability::IpcRecv,
            Syscall::IpcCreateChannel => Capability::IpcCreate,
            Syscall::IpcCloseChannel => Capability::IpcCreate,

            // VectorFS
            Syscall::FsOpen => Capability::FsRead,
            Syscall::FsClose => Capability::FsRead,
            Syscall::FsRead => Capability::FsRead,
            Syscall::FsWrite => Capability::FsWrite,
            Syscall::FsSearch => Capability::FsSearch,
            Syscall::FsCreateCluster => Capability::FsCreate,
            Syscall::FsDelete => Capability::FsDelete,
            Syscall::FsList => Capability::FsRead,
            Syscall::FsStat => Capability::FsRead,

            // Network
            Syscall::NetConnect => Capability::NetConnect,
            Syscall::NetListen => Capability::NetListen,
            Syscall::NetSend => Capability::NetSend,
            Syscall::NetRecv => Capability::NetRecv,
            Syscall::NetClose => Capability::NetConnect,

            // Crypto
            Syscall::CryptoEncrypt => Capability::CryptoEncrypt,
            Syscall::CryptoDecrypt => Capability::CryptoDecrypt,
            Syscall::CryptoSign => Capability::CryptoSign,
            Syscall::CryptoVerify => Capability::CryptoVerify,
            Syscall::CryptoKeygen => Capability::CryptoKeyAccess,
            Syscall::CryptoHash => Capability::Execute,

            // Agent management
            Syscall::AgentInspect => Capability::AgentInspect,
            Syscall::AgentKill => Capability::AgentKill,
            Syscall::CapDelegate => Capability::CapDelegate,
            Syscall::CapRevoke => Capability::CapDelegate,
        }
    }

    /// Human-readable syscall name
    pub fn name(&self) -> &'static str {
        match self {
            Syscall::Spawn => "spawn",
            Syscall::Exit => "exit",
            Syscall::Yield => "yield",
            Syscall::Sleep => "sleep",
            Syscall::GetId => "get_id",
            Syscall::GetCaps => "get_caps",
            Syscall::Fork => "fork",
            Syscall::Alloc => "alloc",
            Syscall::Free => "free",
            Syscall::Mmap => "mmap",
            Syscall::Munmap => "munmap",
            Syscall::MemInfo => "mem_info",
            Syscall::IpcSend => "ipc_send",
            Syscall::IpcRecv => "ipc_recv",
            Syscall::IpcTryRecv => "ipc_try_recv",
            Syscall::IpcCreateChannel => "ipc_create_channel",
            Syscall::IpcCloseChannel => "ipc_close_channel",
            Syscall::FsOpen => "fs_open",
            Syscall::FsClose => "fs_close",
            Syscall::FsRead => "fs_read",
            Syscall::FsWrite => "fs_write",
            Syscall::FsSearch => "fs_search",
            Syscall::FsCreateCluster => "fs_create_cluster",
            Syscall::FsDelete => "fs_delete",
            Syscall::FsList => "fs_list",
            Syscall::FsStat => "fs_stat",
            Syscall::NetConnect => "net_connect",
            Syscall::NetListen => "net_listen",
            Syscall::NetSend => "net_send",
            Syscall::NetRecv => "net_recv",
            Syscall::NetClose => "net_close",
            Syscall::CryptoEncrypt => "crypto_encrypt",
            Syscall::CryptoDecrypt => "crypto_decrypt",
            Syscall::CryptoSign => "crypto_sign",
            Syscall::CryptoVerify => "crypto_verify",
            Syscall::CryptoKeygen => "crypto_keygen",
            Syscall::CryptoHash => "crypto_hash",
            Syscall::AgentInspect => "agent_inspect",
            Syscall::AgentKill => "agent_kill",
            Syscall::CapDelegate => "cap_delegate",
            Syscall::CapRevoke => "cap_revoke",
        }
    }
}

/// Syscall arguments (passed from agent to kernel)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyscallArgs {
    /// The syscall number
    pub syscall: Syscall,
    /// Up to 6 register-sized arguments
    pub args: [u64; 6],
    /// Optional data buffer
    pub data: Option<Vec<u8>>,
}

/// Syscall result (returned from kernel to agent)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SyscallResult {
    /// Success with return value
    Ok(u64),
    /// Success with data
    OkData(Vec<u8>),
    /// Error
    Err(SyscallError),
}

/// Syscall errors
#[derive(Debug, Clone, Serialize, Deserialize, thiserror::Error)]
pub enum SyscallError {
    #[error("Permission denied: missing capability")]
    PermissionDenied,
    #[error("Invalid argument at position {0}")]
    InvalidArg(usize),
    #[error("Resource not found")]
    NotFound,
    #[error("Resource already exists")]
    AlreadyExists,
    #[error("Out of memory")]
    OutOfMemory,
    #[error("Quota exceeded")]
    QuotaExceeded,
    #[error("Operation timed out")]
    Timeout,
    #[error("Resource busy")]
    Busy,
    #[error("Internal kernel error: {0}")]
    Internal(String),
}

/// The syscall dispatch table
pub struct SyscallTable {
    /// Call counter per syscall
    call_counts: std::collections::HashMap<u8, u64>,
}

impl SyscallTable {
    pub fn new() -> Self {
        Self {
            call_counts: std::collections::HashMap::new(),
        }
    }

    /// Dispatch a syscall (checks capabilities, then executes)
    pub fn dispatch(
        &mut self,
        caller: &crate::agent::Agent,
        args: SyscallArgs,
    ) -> SyscallResult {
        // 1. Check capability
        let required_cap = args.syscall.required_capability();
        if let Err(_) = caller.capabilities.require(required_cap) {
            tracing::warn!(
                agent = ?caller.id,
                syscall = args.syscall.name(),
                "Syscall denied: missing {:?}",
                required_cap
            );
            return SyscallResult::Err(SyscallError::PermissionDenied);
        }

        // 2. Track call count
        *self.call_counts.entry(args.syscall as u8).or_insert(0) += 1;

        // 3. Dispatch to handler
        match args.syscall {
            Syscall::GetId => {
                let bytes = caller.id.as_bytes();
                SyscallResult::OkData(bytes.to_vec())
            }
            Syscall::GetCaps => {
                let caps = caller.capabilities.list();
                let data = serde_json::to_vec(&caps).unwrap_or_default();
                SyscallResult::OkData(data)
            }
            Syscall::Yield => SyscallResult::Ok(0),
            Syscall::Exit => SyscallResult::Ok(args.args[0]),
            Syscall::Sleep => {
                // args[0] = milliseconds
                SyscallResult::Ok(args.args[0])
            }
            Syscall::CryptoHash => {
                if let Some(data) = &args.data {
                    let hash = blake3::hash(data);
                    SyscallResult::OkData(hash.as_bytes().to_vec())
                } else {
                    SyscallResult::Err(SyscallError::InvalidArg(0))
                }
            }
            // TODO: Implement remaining syscall handlers
            _ => {
                tracing::debug!(syscall = args.syscall.name(), "Syscall not yet implemented");
                SyscallResult::Err(SyscallError::Internal(
                    format!("Syscall {} not yet implemented", args.syscall.name()),
                ))
            }
        }
    }

    /// Get call statistics
    pub fn stats(&self) -> &std::collections::HashMap<u8, u64> {
        &self.call_counts
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::Agent;
    use crate::capability::CapabilitySet;

    #[test]
    fn test_syscall_capability_check() {
        let mut table = SyscallTable::new();
        let mut agent = Agent::new("test");
        agent.capabilities = CapabilitySet::empty(); // No capabilities

        let args = SyscallArgs {
            syscall: Syscall::Spawn,
            args: [0; 6],
            data: None,
        };

        match table.dispatch(&agent, args) {
            SyscallResult::Err(SyscallError::PermissionDenied) => {} // Expected
            other => panic!("Expected PermissionDenied, got {:?}", other),
        }
    }

    #[test]
    fn test_get_id_syscall() {
        let mut table = SyscallTable::new();
        let mut agent = Agent::new("test");
        agent.capabilities = CapabilitySet::user_default();

        let args = SyscallArgs {
            syscall: Syscall::GetId,
            args: [0; 6],
            data: None,
        };

        match table.dispatch(&agent, args) {
            SyscallResult::OkData(data) => assert_eq!(data.len(), 16),
            other => panic!("Expected OkData, got {:?}", other),
        }
    }

    #[test]
    fn test_crypto_hash_syscall() {
        let mut table = SyscallTable::new();
        let mut agent = Agent::new("test");
        agent.capabilities = CapabilitySet::user_default();

        let args = SyscallArgs {
            syscall: Syscall::CryptoHash,
            args: [0; 6],
            data: Some(b"hello world".to_vec()),
        };

        match table.dispatch(&agent, args) {
            SyscallResult::OkData(hash) => assert_eq!(hash.len(), 32),
            other => panic!("Expected OkData, got {:?}", other),
        }
    }
}
