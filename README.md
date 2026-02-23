<div align="center">

# Neural Kernel

### AI Microkernel — Capability-based agent scheduler with VectorFS

*The orchestration layer of the Crux Sovereign AI Stack*

[![License: AGPL-3.0](https://img.shields.io/badge/License-AGPL%20v3-blue.svg)](https://www.gnu.org/licenses/agpl-3.0)
[![Rust](https://img.shields.io/badge/rust-1.75%2B-orange.svg)](https://www.rust-lang.org/)
[![Part of Crux Ecosystem](https://img.shields.io/badge/crux-ecosystem-orange)](https://github.com/crux-ecosystem)

</div>

---

## What is Neural Kernel?

Neural Kernel is an **AI microkernel** — a minimal, capability-secured runtime for scheduling and isolating AI agents. Think of it as an operating system kernel, but designed specifically for AI workloads.

### Core Subsystems

| Module | Description |
|--------|-------------|
| **Scheduler** | CFS-inspired preemptive scheduler with vruntime tracking, priority weights, and per-agent time slicing |
| **Capability System** | 30+ fine-grained capabilities (Execute, FsRead, CryptoEncrypt, GpuCompute, ...) with unforgeable delegatable tokens |
| **Syscall Interface** | 40 kernel syscalls (spawn, alloc, ipc_send, fs_search, crypto_encrypt, agent_kill, ...) |
| **Memory Manager** | Arena-based allocation with vector-aware memory regions and per-agent quotas |
| **IPC** | Zero-copy message passing via SPSC ring buffers in shared memory |
| **Sandbox** | Capability enforcement + resource quota checking at every syscall boundary |
| **VectorFS** | FUSE filesystem where files are vectors, directories are clusters, and search is the primary operation |
| **Python Bindings** | Full PyO3 integration for using Neural Kernel from Python |

## Architecture

```
┌─────────────────────────────────┐
│         User Space              │
│  ┌─────┐ ┌─────┐ ┌─────────┐  │
│  │Agent│ │Agent│ │De-RAG   │  │
│  │  A  │ │  B  │ │  Node   │  │
│  └──┬──┘ └──┬──┘ └────┬────┘  │
│     │       │          │       │
│  ───┴───────┴──────────┴───    │
│        Syscall Interface       │
│     (40 syscalls, cap-gated)   │
├────────────────────────────────┤
│         Kernel Space           │
│  ┌──────────┐ ┌───────────┐   │
│  │Scheduler │ │ Capability│   │
│  │(CFS, O(  │ │   Table   │   │
│  │log n))   │ │ (30 caps) │   │
│  └──────────┘ └───────────┘   │
│  ┌──────────┐ ┌───────────┐   │
│  │ Memory   │ │ VectorFS  │   │
│  │ Manager  │ │ (FUSE)    │   │
│  │ (arenas) │ │           │   │
│  └──────────┘ └───────────┘   │
│  ┌──────────┐ ┌───────────┐   │
│  │   IPC    │ │  Sandbox  │   │
│  │ (SPSC)   │ │ (seccomp) │   │
│  └──────────┘ └───────────┘   │
└────────────────────────────────┘
```

## Quick Start

### Build from source

```bash
# Clone
git clone https://github.com/crux-ecosystem/neural-kernel.git
cd neural-kernel

# Build (release)
cargo build --release

# Run tests
cargo test

# Install CLI
cargo install --path .

# Start kernel
nk start
```

### Python bindings

```bash
# Build Python wheel
pip install maturin
maturin build --features python --release

# Install
pip install target/wheels/neural_kernel-*.whl
```

```python
import neural_kernel as nk

# Scheduler
sched = nk.Scheduler(max_agents=1024, quantum_ms=10)

# Create an agent
agent = nk.Agent("research-bot", priority="high")
agent.grant_capability("fs_read")
agent.grant_capability("fs_search")
agent.grant_capability("crypto_encrypt")
print(agent.capabilities())

# Admit to scheduler
sched.admit(agent)
print(f"Ready: {sched.ready_count}, Total: {sched.total_agents}")

# VectorFS
vfs = nk.VectorStore(dim=768)
vfs.create_cluster("documents")
vfs.add("paper.vec", vector=[0.1] * 768, cluster_id=1)
results = vfs.search(query=[0.1] * 768, top_k=5)
print(results)
```

## Capability System

Agents can only perform operations they have explicit capabilities for:

```rust
use neural_kernel::capability::*;

let caps = CapabilitySetBuilder::new()
    .compute()           // Execute, Spawn, MemAlloc, MemRead, MemWrite
    .storage()           // FsRead, FsWrite, FsCreate, FsSearch
    .crypto()            // Encrypt, Decrypt, Sign, Verify
    .build();

// Check at runtime
caps.require(Capability::FsSearch)?;   // ✓ OK
caps.require(Capability::GpuCompute)?; // ✗ Error: Missing capability
```

## Syscall Interface

All agent↔kernel communication flows through numbered syscalls:

| Range | Category | Syscalls |
|-------|----------|----------|
| 0x00-0x0F | Process | spawn, exit, yield, sleep, get_id, fork |
| 0x10-0x1F | Memory | alloc, free, mmap, munmap, mem_info |
| 0x20-0x2F | IPC | send, recv, try_recv, create_channel, close_channel |
| 0x30-0x3F | VectorFS | open, close, read, write, search, create_cluster, delete |
| 0x40-0x4F | Network | connect, listen, send, recv, close |
| 0x50-0x5F | Crypto | encrypt, decrypt, sign, verify, keygen, hash |
| 0x60-0x6F | Admin | inspect, kill, cap_delegate, cap_revoke |

## VectorFS

A FUSE filesystem where vectors are first-class citizens:

```bash
# Mount
nk fs mount /tmp/vectorfs

# Operations
echo "[0.1, 0.2, 0.3]" > /tmp/vectorfs/docs/embedding.vec
cat /tmp/vectorfs/docs/embedding.vec  # JSON output

# Search is the primary operation
ls /tmp/vectorfs/docs/  # Shows vectors ranked by cluster similarity
```

## Testing

```bash
# All tests
cargo test

# With output
cargo test -- --nocapture

# Specific module
cargo test scheduler::tests
cargo test capability::tests
cargo test vectorfs::tests

# Benchmarks
cargo bench
```

## Project Structure

```
Neural_Kernel/
├── Cargo.toml              # Dependencies and features
├── src/
│   ├── lib.rs              # Crate root, module declarations
│   ├── main.rs             # CLI entry point (nk binary)
│   ├── agent.rs            # Agent definition, state machine, priority
│   ├── capability.rs       # 30+ capabilities, unforgeable tokens, builder
│   ├── scheduler.rs        # CFS scheduler, vruntime, time slicing
│   ├── syscall.rs          # 40 syscalls, dispatch table, cap checking
│   ├── memory.rs           # Arena allocator, vector regions, quotas
│   ├── ipc.rs              # SPSC ring buffers, channels, message types
│   ├── sandbox.rs          # Policy enforcement, violation tracking
│   ├── vectorfs.rs         # Vector files, clusters, similarity search
│   └── python.rs           # PyO3 bindings (optional)
└── README.md
```

## Roadmap

- [x] **v0.1.0** — Core kernel (scheduler, capabilities, syscalls, memory, IPC, sandbox, VectorFS)
- [ ] **v0.2.0** — Real FUSE mount, seccomp-BPF sandbox, work-stealing scheduler
- [ ] **v0.3.0** — De-RAG integration (encrypted VectorFS, P2P agent migration)
- [ ] **v0.4.0** — GPU scheduling (CUDA/Vulkan compute capabilities)
- [ ] **v0.5.0** — WASM agent runtime (sandboxed WebAssembly execution)
- [ ] **v1.0.0** — Production kernel, formal verification of capability system

## Part of the Crux Ecosystem

| Project | Role | Language | Status |
|---------|------|----------|--------|
| [MOL](https://github.com/crux-ecosystem/mol-lang) | Sovereign compute language | Python | v1.1.0 |
| [De-RAG](https://github.com/crux-ecosystem/de-rag) | Decentralized encrypted data | Python | v0.1.0 |
| **Neural Kernel** | AI microkernel & orchestration | Rust | v0.1.0 |
| [IntraMind](https://github.com/crux-ecosystem/IntraMind) | Offline-first RAG engine | Python | v1.1.0 |

## License

AGPL-3.0 — See [LICENSE](LICENSE) for details.

**Copyright (c) 2026 CruxLabx**
