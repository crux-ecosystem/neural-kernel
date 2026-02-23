//! Sandbox — Agent isolation via capability enforcement
//!
//! Each agent runs in a restricted sandbox:
//! - Capability-based access control (checked at syscall boundary)
//! - Resource quota enforcement (memory, CPU, FDs)
//! - Syscall filtering (only allowed syscalls pass through)
//! - Namespace isolation (each agent has its own view)
//!
//! On Linux, this could be backed by seccomp-BPF + namespaces.
//! On other platforms, we use a software enforcement layer.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::agent::AgentId;
use crate::capability::{Capability, CapabilitySet};
use crate::syscall::Syscall;

/// Sandbox policy — what an agent is allowed to do
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxPolicy {
    /// Allowed syscalls (if empty, use capability-based filtering)
    pub allowed_syscalls: HashSet<u8>,

    /// Maximum memory (bytes)
    pub max_memory: usize,

    /// Maximum CPU time per quantum
    pub max_cpu_time: Duration,

    /// Maximum open file descriptors
    pub max_fds: usize,

    /// Maximum network connections
    pub max_connections: usize,

    /// Allowed filesystem paths (prefixes)
    pub allowed_paths: Vec<String>,

    /// Allowed network destinations (CIDR or host:port)
    pub allowed_network: Vec<String>,

    /// Whether the agent can spawn children
    pub can_spawn: bool,

    /// Whether the agent can access GPU
    pub can_gpu: bool,

    /// Whether the agent can access raw crypto keys
    pub can_access_keys: bool,
}

impl SandboxPolicy {
    /// Restrictive policy (minimal permissions)
    pub fn restrictive() -> Self {
        Self {
            allowed_syscalls: HashSet::new(),
            max_memory: 64 * 1024 * 1024, // 64 MB
            max_cpu_time: Duration::from_millis(50),
            max_fds: 16,
            max_connections: 4,
            allowed_paths: vec![],
            allowed_network: vec![],
            can_spawn: false,
            can_gpu: false,
            can_access_keys: false,
        }
    }

    /// Standard policy (reasonable defaults)
    pub fn standard() -> Self {
        Self {
            allowed_syscalls: HashSet::new(),
            max_memory: 256 * 1024 * 1024, // 256 MB
            max_cpu_time: Duration::from_millis(100),
            max_fds: 64,
            max_connections: 32,
            allowed_paths: vec!["/vectorfs".into()],
            allowed_network: vec!["127.0.0.1:*".into()],
            can_spawn: true,
            can_gpu: false,
            can_access_keys: false,
        }
    }

    /// Privileged policy (for kernel agents)
    pub fn privileged() -> Self {
        Self {
            allowed_syscalls: HashSet::new(),
            max_memory: 4 * 1024 * 1024 * 1024, // 4 GB
            max_cpu_time: Duration::from_secs(10),
            max_fds: 1024,
            max_connections: 1024,
            allowed_paths: vec!["/".into()],
            allowed_network: vec!["*".into()],
            can_spawn: true,
            can_gpu: true,
            can_access_keys: true,
        }
    }
}

/// Sandbox violation record
#[derive(Debug, Clone, Serialize)]
pub struct Violation {
    /// Agent that violated
    pub agent_id: AgentId,
    /// What was violated
    pub violation_type: ViolationType,
    /// Human-readable description
    pub description: String,
    /// When it happened (skipped in serialization — Instant is not serializable)
    #[serde(skip)]
    pub timestamp: Instant,
}

/// Types of sandbox violations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ViolationType {
    /// Attempted unauthorized syscall
    UnauthorizedSyscall(u8),
    /// Memory quota exceeded
    MemoryQuotaExceeded { current: usize, limit: usize },
    /// CPU quota exceeded
    CpuQuotaExceeded { used: Duration, limit: Duration },
    /// File descriptor limit reached
    FdLimitReached,
    /// Unauthorized file access
    UnauthorizedPath(String),
    /// Unauthorized network access
    UnauthorizedNetwork(String),
    /// Capability escalation attempt
    CapabilityEscalation(String),
}

/// A sandbox instance for one agent
pub struct Sandbox {
    /// The agent this sandbox protects
    pub agent_id: AgentId,
    /// Policy
    pub policy: SandboxPolicy,
    /// Current resource usage
    pub memory_used: usize,
    pub cpu_used: Duration,
    pub fd_count: usize,
    pub connection_count: usize,
    /// Violation log
    pub violations: Vec<Violation>,
    /// Is the sandbox active?
    pub active: bool,
}

impl Sandbox {
    /// Create a new sandbox for an agent
    pub fn new(agent_id: AgentId, policy: SandboxPolicy) -> Self {
        Self {
            agent_id,
            policy,
            memory_used: 0,
            cpu_used: Duration::ZERO,
            fd_count: 0,
            connection_count: 0,
            violations: Vec::new(),
            active: true,
        }
    }

    /// Check if a syscall is allowed
    pub fn check_syscall(
        &mut self,
        syscall: Syscall,
        agent_caps: &CapabilitySet,
    ) -> Result<(), SandboxViolation> {
        if !self.active {
            return Err(SandboxViolation::SandboxInactive);
        }

        // Check capability
        let required = syscall.required_capability();
        if !agent_caps.has(required) {
            self.record_violation(ViolationType::UnauthorizedSyscall(syscall as u8));
            return Err(SandboxViolation::MissingCapability(required));
        }

        // Check syscall whitelist (if configured)
        if !self.policy.allowed_syscalls.is_empty()
            && !self.policy.allowed_syscalls.contains(&(syscall as u8))
        {
            self.record_violation(ViolationType::UnauthorizedSyscall(syscall as u8));
            return Err(SandboxViolation::SyscallBlocked(syscall as u8));
        }

        Ok(())
    }

    /// Check if a memory allocation is within quota
    pub fn check_memory(&mut self, additional_bytes: usize) -> Result<(), SandboxViolation> {
        let new_total = self.memory_used + additional_bytes;
        if new_total > self.policy.max_memory {
            self.record_violation(ViolationType::MemoryQuotaExceeded {
                current: new_total,
                limit: self.policy.max_memory,
            });
            return Err(SandboxViolation::MemoryExceeded {
                requested: additional_bytes,
                limit: self.policy.max_memory,
                current: self.memory_used,
            });
        }
        self.memory_used = new_total;
        Ok(())
    }

    /// Check if CPU time is within quota
    pub fn check_cpu(&mut self, additional: Duration) -> Result<(), SandboxViolation> {
        let new_total = self.cpu_used + additional;
        if new_total > self.policy.max_cpu_time {
            self.record_violation(ViolationType::CpuQuotaExceeded {
                used: new_total,
                limit: self.policy.max_cpu_time,
            });
            return Err(SandboxViolation::CpuExceeded {
                used: new_total,
                limit: self.policy.max_cpu_time,
            });
        }
        self.cpu_used = new_total;
        Ok(())
    }

    /// Check if a filesystem path is allowed
    pub fn check_path(&mut self, path: &str) -> Result<(), SandboxViolation> {
        let allowed = self
            .policy
            .allowed_paths
            .iter()
            .any(|prefix| path.starts_with(prefix));

        if !allowed {
            self.record_violation(ViolationType::UnauthorizedPath(path.to_string()));
            return Err(SandboxViolation::PathBlocked(path.to_string()));
        }
        Ok(())
    }

    /// Record a violation
    fn record_violation(&mut self, vtype: ViolationType) {
        self.violations.push(Violation {
            agent_id: self.agent_id,
            violation_type: vtype,
            description: String::new(),
            timestamp: Instant::now(),
        });

        // Auto-deactivate after too many violations
        if self.violations.len() > 100 {
            self.active = false;
            tracing::warn!(
                ?self.agent_id,
                "Sandbox deactivated after 100+ violations"
            );
        }
    }

    /// Get violation count
    pub fn violation_count(&self) -> usize {
        self.violations.len()
    }

    /// Reset resource tracking (called at start of each quantum)
    pub fn reset_quantum(&mut self) {
        self.cpu_used = Duration::ZERO;
    }
}

/// Sandbox violation errors
#[derive(Debug, thiserror::Error)]
pub enum SandboxViolation {
    #[error("Sandbox is inactive (agent terminated)")]
    SandboxInactive,

    #[error("Missing capability: {0:?}")]
    MissingCapability(Capability),

    #[error("Syscall {0:#x} is blocked by policy")]
    SyscallBlocked(u8),

    #[error("Memory exceeded: requested {requested}, limit {limit}, current {current}")]
    MemoryExceeded {
        requested: usize,
        limit: usize,
        current: usize,
    },

    #[error("CPU time exceeded: used {used:?}, limit {limit:?}")]
    CpuExceeded { used: Duration, limit: Duration },

    #[error("Path blocked: {0}")]
    PathBlocked(String),

    #[error("Network blocked: {0}")]
    NetworkBlocked(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_restrictive_sandbox() {
        let agent = AgentId::new();
        let mut sandbox = Sandbox::new(agent, SandboxPolicy::restrictive());
        let caps = CapabilitySet::empty();

        // Should block everything with empty capabilities
        assert!(sandbox.check_syscall(Syscall::Spawn, &caps).is_err());
    }

    #[test]
    fn test_standard_sandbox_allows_basic_ops() {
        let agent = AgentId::new();
        let mut sandbox = Sandbox::new(agent, SandboxPolicy::standard());
        let caps = CapabilitySet::user_default();

        assert!(sandbox.check_syscall(Syscall::GetId, &caps).is_ok());
        assert!(sandbox.check_syscall(Syscall::FsRead, &caps).is_ok());
    }

    #[test]
    fn test_memory_quota() {
        let agent = AgentId::new();
        let mut sandbox = Sandbox::new(agent, SandboxPolicy::restrictive());

        // 64 MB quota
        sandbox.check_memory(32 * 1024 * 1024).unwrap(); // 32 MB OK
        sandbox.check_memory(32 * 1024 * 1024).unwrap(); // 64 MB total OK
        assert!(sandbox.check_memory(1).is_err()); // Over quota
    }

    #[test]
    fn test_path_checking() {
        let agent = AgentId::new();
        let mut sandbox = Sandbox::new(agent, SandboxPolicy::standard());

        assert!(sandbox.check_path("/vectorfs/data.vec").is_ok());
        assert!(sandbox.check_path("/etc/passwd").is_err());
    }

    #[test]
    fn test_violation_tracking() {
        let agent = AgentId::new();
        let mut sandbox = Sandbox::new(agent, SandboxPolicy::restrictive());
        let caps = CapabilitySet::empty();

        let _ = sandbox.check_syscall(Syscall::Spawn, &caps);
        let _ = sandbox.check_syscall(Syscall::NetConnect, &caps);
        assert_eq!(sandbox.violation_count(), 2);
    }
}
