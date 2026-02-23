//! Capability System — Fine-grained permission control for agents
//!
//! Inspired by capability-based security (seL4, Capsicum, WASI):
//! - Each agent holds a set of capability tokens
//! - Capabilities are unforgeable and can be delegated
//! - The kernel validates capabilities on every syscall
//! - Capabilities can be attenuated (reduced) but never amplified
//!
//! Capability hierarchy:
//! ```text
//!   Root (kernel only)
//!     ├── Compute    — execute code, spawn children
//!     ├── Memory     — allocate, map, share
//!     ├── Storage    — read/write VectorFS
//!     ├── Network    — TCP/UDP, P2P
//!     ├── IPC        — send/receive messages
//!     ├── Crypto     — encrypt, decrypt, sign
//!     ├── GPU        — tensor operations
//!     └── Admin      — manage other agents
//! ```

use std::collections::HashSet;
use std::fmt;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::agent::AgentId;

/// The set of capability types in Neural Kernel
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Capability {
    // ── Compute ──
    /// Execute code
    Execute,
    /// Spawn child agents
    Spawn,
    /// Fork (copy) self
    Fork,

    // ── Memory ──
    /// Allocate memory
    MemAlloc,
    /// Map shared memory regions
    MemMap,
    /// Read from memory-mapped regions
    MemRead,
    /// Write to memory-mapped regions
    MemWrite,

    // ── Storage (VectorFS) ──
    /// Read files/vectors from VectorFS
    FsRead,
    /// Write files/vectors to VectorFS
    FsWrite,
    /// Create directories/clusters
    FsCreate,
    /// Delete files/vectors
    FsDelete,
    /// Search VectorFS (similarity queries)
    FsSearch,

    // ── Network ──
    /// Open outbound connections
    NetConnect,
    /// Listen for inbound connections
    NetListen,
    /// Send data
    NetSend,
    /// Receive data
    NetRecv,
    /// P2P peer operations
    NetPeer,

    // ── IPC ──
    /// Send IPC messages to other agents
    IpcSend,
    /// Receive IPC messages
    IpcRecv,
    /// Create IPC channels
    IpcCreate,

    // ── Crypto ──
    /// Encrypt data
    CryptoEncrypt,
    /// Decrypt data
    CryptoDecrypt,
    /// Sign data
    CryptoSign,
    /// Verify signatures
    CryptoVerify,
    /// Access key material
    CryptoKeyAccess,

    // ── GPU / Tensor ──
    /// Submit GPU compute tasks
    GpuCompute,
    /// Allocate GPU memory
    GpuAlloc,

    // ── Admin ──
    /// Inspect other agents
    AgentInspect,
    /// Kill other agents
    AgentKill,
    /// Modify capabilities of children
    CapDelegate,
    /// Access kernel internals
    KernelAccess,
}

impl fmt::Display for Capability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

/// A capability token — unforgeable, delegatable permission
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityToken {
    /// The capability this token grants
    pub capability: Capability,

    /// Who issued this token
    pub issuer: AgentId,

    /// Who holds this token
    pub holder: AgentId,

    /// Optional expiry
    pub expires_at: Option<Instant>,

    /// Whether this token can be further delegated
    pub delegatable: bool,

    /// Optional restrictions (e.g., path prefix for FsRead)
    pub restrictions: Vec<String>,

    /// Unique token ID (for revocation)
    pub token_id: u64,
}

impl CapabilityToken {
    /// Check if this token has expired
    pub fn is_expired(&self) -> bool {
        self.expires_at.map_or(false, |exp| Instant::now() > exp)
    }

    /// Check if this token is valid (not expired)
    pub fn is_valid(&self) -> bool {
        !self.is_expired()
    }

    /// Create a delegated (possibly attenuated) copy
    pub fn delegate(
        &self,
        to: AgentId,
        restrictions: Vec<String>,
    ) -> Result<CapabilityToken, CapabilityError> {
        if !self.delegatable {
            return Err(CapabilityError::NotDelegatable(self.capability));
        }
        if self.is_expired() {
            return Err(CapabilityError::Expired(self.capability));
        }

        // Merge restrictions (attenuate only — cannot widen)
        let mut merged = self.restrictions.clone();
        merged.extend(restrictions);

        Ok(CapabilityToken {
            capability: self.capability,
            issuer: self.holder, // current holder becomes issuer
            holder: to,
            expires_at: self.expires_at, // inherit expiry
            delegatable: false,          // delegated tokens are not re-delegatable by default
            restrictions: merged,
            token_id: rand::random::<u64>(),
        })
    }
}

/// A set of capabilities held by an agent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilitySet {
    capabilities: HashSet<Capability>,
}

impl CapabilitySet {
    /// Empty capability set (no permissions)
    pub fn empty() -> Self {
        Self {
            capabilities: HashSet::new(),
        }
    }

    /// Minimal default set for a new user agent
    pub fn user_default() -> Self {
        let mut caps = HashSet::new();
        caps.insert(Capability::Execute);
        caps.insert(Capability::MemAlloc);
        caps.insert(Capability::MemRead);
        caps.insert(Capability::MemWrite);
        caps.insert(Capability::FsRead);
        caps.insert(Capability::FsSearch);
        caps.insert(Capability::IpcSend);
        caps.insert(Capability::IpcRecv);
        Self { capabilities: caps }
    }

    /// Full capability set (only for kernel agents)
    pub fn kernel() -> Self {
        Self {
            capabilities: [
                Capability::Execute,
                Capability::Spawn,
                Capability::Fork,
                Capability::MemAlloc,
                Capability::MemMap,
                Capability::MemRead,
                Capability::MemWrite,
                Capability::FsRead,
                Capability::FsWrite,
                Capability::FsCreate,
                Capability::FsDelete,
                Capability::FsSearch,
                Capability::NetConnect,
                Capability::NetListen,
                Capability::NetSend,
                Capability::NetRecv,
                Capability::NetPeer,
                Capability::IpcSend,
                Capability::IpcRecv,
                Capability::IpcCreate,
                Capability::CryptoEncrypt,
                Capability::CryptoDecrypt,
                Capability::CryptoSign,
                Capability::CryptoVerify,
                Capability::CryptoKeyAccess,
                Capability::GpuCompute,
                Capability::GpuAlloc,
                Capability::AgentInspect,
                Capability::AgentKill,
                Capability::CapDelegate,
                Capability::KernelAccess,
            ]
            .into_iter()
            .collect(),
        }
    }

    /// Check if this set contains a capability
    pub fn has(&self, cap: Capability) -> bool {
        self.capabilities.contains(&cap)
    }

    /// Require a capability (returns error if missing)
    pub fn require(&self, cap: Capability) -> Result<(), CapabilityError> {
        if self.has(cap) {
            Ok(())
        } else {
            Err(CapabilityError::Missing(cap))
        }
    }

    /// Grant a capability
    pub fn grant(&mut self, cap: Capability) {
        self.capabilities.insert(cap);
    }

    /// Revoke a capability
    pub fn revoke(&mut self, cap: Capability) {
        self.capabilities.remove(&cap);
    }

    /// Number of capabilities
    pub fn count(&self) -> usize {
        self.capabilities.len()
    }

    /// Check if this is a subset of another set (for delegation)
    pub fn is_subset_of(&self, other: &CapabilitySet) -> bool {
        self.capabilities.is_subset(&other.capabilities)
    }

    /// Intersection with another set
    pub fn intersect(&self, other: &CapabilitySet) -> CapabilitySet {
        CapabilitySet {
            capabilities: self
                .capabilities
                .intersection(&other.capabilities)
                .copied()
                .collect(),
        }
    }

    /// List all capabilities
    pub fn list(&self) -> Vec<Capability> {
        self.capabilities.iter().copied().collect()
    }
}

/// Builder for CapabilitySet
pub struct CapabilitySetBuilder {
    caps: HashSet<Capability>,
}

impl CapabilitySetBuilder {
    pub fn new() -> Self {
        Self {
            caps: HashSet::new(),
        }
    }

    pub fn add(mut self, cap: Capability) -> Self {
        self.caps.insert(cap);
        self
    }

    pub fn compute(self) -> Self {
        self.add(Capability::Execute)
            .add(Capability::Spawn)
            .add(Capability::MemAlloc)
            .add(Capability::MemRead)
            .add(Capability::MemWrite)
    }

    pub fn storage(self) -> Self {
        self.add(Capability::FsRead)
            .add(Capability::FsWrite)
            .add(Capability::FsCreate)
            .add(Capability::FsSearch)
    }

    pub fn network(self) -> Self {
        self.add(Capability::NetConnect)
            .add(Capability::NetSend)
            .add(Capability::NetRecv)
    }

    pub fn crypto(self) -> Self {
        self.add(Capability::CryptoEncrypt)
            .add(Capability::CryptoDecrypt)
            .add(Capability::CryptoSign)
            .add(Capability::CryptoVerify)
    }

    pub fn build(self) -> CapabilitySet {
        CapabilitySet {
            capabilities: self.caps,
        }
    }
}

/// Capability-related errors
#[derive(Debug, thiserror::Error)]
pub enum CapabilityError {
    #[error("Missing capability: {0}")]
    Missing(Capability),

    #[error("Capability {0} is not delegatable")]
    NotDelegatable(Capability),

    #[error("Capability {0} has expired")]
    Expired(Capability),

    #[error("Capability {0} revoked")]
    Revoked(Capability),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_set() {
        let caps = CapabilitySet::empty();
        assert_eq!(caps.count(), 0);
        assert!(!caps.has(Capability::Execute));
    }

    #[test]
    fn test_user_default() {
        let caps = CapabilitySet::user_default();
        assert!(caps.has(Capability::Execute));
        assert!(caps.has(Capability::FsRead));
        assert!(!caps.has(Capability::KernelAccess));
    }

    #[test]
    fn test_kernel_has_all() {
        let caps = CapabilitySet::kernel();
        assert!(caps.has(Capability::KernelAccess));
        assert!(caps.has(Capability::Execute));
        assert!(caps.has(Capability::CryptoEncrypt));
    }

    #[test]
    fn test_require() {
        let caps = CapabilitySet::user_default();
        assert!(caps.require(Capability::Execute).is_ok());
        assert!(caps.require(Capability::KernelAccess).is_err());
    }

    #[test]
    fn test_builder() {
        let caps = CapabilitySetBuilder::new()
            .compute()
            .storage()
            .build();

        assert!(caps.has(Capability::Execute));
        assert!(caps.has(Capability::FsRead));
        assert!(!caps.has(Capability::NetConnect));
    }

    #[test]
    fn test_subset() {
        let user = CapabilitySet::user_default();
        let kernel = CapabilitySet::kernel();
        assert!(user.is_subset_of(&kernel));
        assert!(!kernel.is_subset_of(&user));
    }
}
