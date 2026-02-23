//! # Neural Kernel
//!
//! **AI Microkernel — Capability-based agent scheduler with VectorFS**
//!
//! Neural Kernel is the orchestration layer of the Crux Sovereign AI Stack.
//! It provides:
//!
//! - **Agent Scheduler** — Preemptive, priority-based scheduling of AI agents
//! - **Capability System** — Fine-grained capability tokens (read, write, net, gpu, exec)
//! - **Syscall Interface** — 32 kernel syscalls for agent-to-kernel communication
//! - **Memory Manager** — Arena-based allocation with memory-mapped vector regions
//! - **VectorFS** — FUSE filesystem where files are vectors and directories are clusters
//! - **IPC** — Zero-copy message passing between agents via shared memory rings
//! - **Sandbox** — Seccomp-BPF + capability isolation per agent
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────────────────────────┐
//! │         User Space              │
//! │  ┌─────┐ ┌─────┐ ┌─────────┐  │
//! │  │Agent│ │Agent│ │MOL REPL │  │
//! │  │  A  │ │  B  │ │         │  │
//! │  └──┬──┘ └──┬──┘ └────┬────┘  │
//! │     │       │          │       │
//! │  ───┴───────┴──────────┴───    │
//! │        Syscall Interface       │
//! ├────────────────────────────────┤
//! │         Kernel Space           │
//! │  ┌──────────┐ ┌───────────┐   │
//! │  │Scheduler │ │ Cap Table │   │
//! │  │(CFS+AI)  │ │ (RBAC)    │   │
//! │  └──────────┘ └───────────┘   │
//! │  ┌──────────┐ ┌───────────┐   │
//! │  │ Memory   │ │ VectorFS  │   │
//! │  │ Manager  │ │ (FUSE)    │   │
//! │  └──────────┘ └───────────┘   │
//! │  ┌──────────┐ ┌───────────┐   │
//! │  │   IPC    │ │  Sandbox  │   │
//! │  │ (rings)  │ │ (seccomp) │   │
//! │  └──────────┘ └───────────┘   │
//! └────────────────────────────────┘
//! ```
//!
//! Copyright (c) 2026 CruxLabx — AGPL-3.0

pub mod agent;
pub mod capability;
pub mod ipc;
pub mod memory;
pub mod sandbox;
pub mod scheduler;
pub mod syscall;
pub mod vectorfs;

#[cfg(feature = "python")]
mod python;

// Re-exports
pub use agent::{Agent, AgentId, AgentState};
pub use capability::{Capability, CapabilitySet, CapabilityToken};
pub use scheduler::{Scheduler, SchedulerConfig};
pub use syscall::SyscallTable;
