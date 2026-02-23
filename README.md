<div align="center">

# Neural Kernel

### AI Microkernel — Capability-Based Agent Scheduler with VectorFS

**An operating system kernel, but for AI agents.**

[![CruxLabx](https://img.shields.io/badge/Built%20by-CruxLabx-blueviolet?style=for-the-badge)](https://github.com/crux-ecosystem)
[![Status](https://img.shields.io/badge/Status-Active%20Development-brightgreen?style=for-the-badge)]()
[![Rust](https://img.shields.io/badge/Rust-1.75+-orange?style=for-the-badge&logo=rust&logoColor=white)]()
[![Python](https://img.shields.io/badge/Python-Bindings-blue?style=for-the-badge&logo=python&logoColor=white)]()

[Architecture](#architecture) • [Subsystems](#core-subsystems) • [Capabilities](#capability-system) • [Syscalls](#syscall-interface) • [VectorFS](#vectorfs) • [Ecosystem](#part-of-crux-sovereign-ai-stack)

</div>

---

## The Problem

AI agents today run in chaos. No isolation. No scheduling. No access control. An LLM agent can read any file, call any API, consume unlimited memory, and crash the entire system. There is no kernel for AI.

**Neural Kernel is that kernel.**

## What is Neural Kernel?

Neural Kernel is an **AI microkernel** — a minimal, capability-secured runtime that schedules, isolates, and governs AI agents. Every agent operation flows through a syscall interface with unforgeable capability tokens. No capability, no access. No exceptions.

It is written in **Rust** for zero-cost abstractions and memory safety, with **Python bindings** (PyO3) for ML/AI ecosystem integration.

### Design Principles

| Principle | Implementation |
|-----------|---------------|
| **Least Privilege** | Agents start with zero capabilities. Every permission is explicitly granted. |
| **Preemptive Fair Scheduling** | CFS-inspired scheduler with vruntime tracking. No agent starves. |
| **Zero-Copy IPC** | SPSC ring buffers in shared memory. No kernel-mediated copies. |
| **Vectors as First-Class Citizens** | VectorFS: a filesystem where files are embedding vectors and search is the primary operation. |
| **Formal Capability Model** | 30+ fine-grained capabilities with cryptographic delegation chains. |

---

## Architecture

```
┌──────────────────────────────────────────────────────┐
│                    User Space                         │
│                                                       │
│   ┌──────────┐  ┌──────────┐  ┌────────────────┐    │
│   │ RAG      │  │ Crypto   │  │ IntraMind      │    │
│   │ Agent    │  │ Agent    │  │ Agent          │    │
│   │ ·········│  │ ·········│  │ ················│    │
│   │ 7 caps   │  │ 5 caps   │  │ 12 caps        │    │
│   └────┬─────┘  └────┬─────┘  └───────┬────────┘    │
│        │              │                │              │
│   ─────┴──────────────┴────────────────┴──────────   │
│                  Syscall Boundary                     │
│              (40 numbered syscalls)                   │
│          Every call is capability-checked             │
├──────────────────────────────────────────────────────┤
│                   Kernel Space                        │
│                                                       │
│   ┌─────────────┐      ┌──────────────────┐          │
│   │  Scheduler  │      │ Capability Table │          │
│   │  CFS, O(1)  │      │ 30+ permissions  │          │
│   │  vruntime   │      │ Unforgeable      │          │
│   │  per-agent  │      │ Delegatable      │          │
│   └─────────────┘      └──────────────────┘          │
│                                                       │
│   ┌─────────────┐      ┌──────────────────┐          │
│   │   Memory    │      │    VectorFS      │          │
│   │   Manager   │      │ Files = Vectors  │          │
│   │ Arena-based │      │ Dirs = Clusters  │          │
│   │ Per-agent   │      │ Search = Primary │          │
│   │ quotas      │      │ FUSE-mountable   │          │
│   └─────────────┘      └──────────────────┘          │
│                                                       │
│   ┌─────────────┐      ┌──────────────────┐          │
│   │     IPC     │      │    Sandbox       │          │
│   │ SPSC ring   │      │ seccomp-BPF      │          │
│   │ Zero-copy   │      │ Policy engine    │          │
│   │ Typed msgs  │      │ Violation track  │          │
│   └─────────────┘      └──────────────────┘          │
└──────────────────────────────────────────────────────┘
```

---

## Core Subsystems

### Scheduler

A **CFS-inspired preemptive scheduler** that ensures fair CPU time distribution across all agents:

- **Virtual runtime (vruntime)** tracking per agent — agents that have used less time get priority
- **Priority weights**: `idle(0.25)` → `low(0.5)` → `normal(1.0)` → `high(2.0)` → `realtime(4.0)`
- **Configurable time slices** with per-agent quantum
- **O(log n)** pick-next using sorted run queue
- States: `Ready → Running → Blocked → Terminated`

### Memory Manager

- **Arena-based allocation** with vector-aware memory regions
- **Per-agent quotas** — no agent can OOM the system
- **Tagged regions**: Code, Stack, Heap, VectorData, Model, SharedMemory
- **Allocation tracking** with peak usage statistics

### IPC (Inter-Process Communication)

- **SPSC (Single-Producer, Single-Consumer) ring buffers** in shared memory
- **Zero-copy message passing** — no kernel-mediated data transfer
- **Typed messages**: Data, Signal, Capability, Error, Shutdown
- **Named channels** with sender/receiver agent binding

### Sandbox

- **Capability enforcement** at every syscall boundary
- **Resource quota checking** — memory, CPU time, file descriptors
- **seccomp-BPF** integration for system call filtering
- **Violation logging** with per-agent strike tracking

---

## Capability System

30+ fine-grained capabilities organized by domain:

| Domain | Capabilities |
|--------|------------|
| **Process** | Execute, Spawn, Kill, Yield, Sleep |
| **Memory** | MemAlloc, MemRead, MemWrite, MemExec |
| **Filesystem** | FsRead, FsWrite, FsCreate, FsDelete, FsSearch |
| **Network** | NetConnect, NetListen, NetSend, NetReceive |
| **Crypto** | CryptoEncrypt, CryptoDecrypt, CryptoSign, CryptoVerify |
| **Vector** | VectorRead, VectorWrite, VectorSearch, VectorIndex |
| **Model** | ModelLoad, ModelInfer, ModelTrain |
| **GPU** | GpuCompute, GpuMemAlloc |
| **IPC** | IpcSend, IpcReceive, IpcCreateChannel |
| **Audit** | AuditRead, AuditWrite |
| **Admin** | Admin (superuser) |

**Capability tokens** are:
- **Unforgeable** — cryptographically signed at creation
- **Delegatable** — an agent can grant a subset of its own capabilities
- **Revocable** — kernel can revoke any token at any time
- **Auditable** — every grant, revoke, and check is logged

```
Agent A (12 capabilities)
   │
   ├── delegates {FsRead, VectorSearch} ──▶ Agent B
   │
   └── delegates {CryptoEncrypt} ──▶ Agent C
                                        │
                                        └── cannot delegate further
                                            (no Admin capability)
```

---

## Syscall Interface

All agent↔kernel communication flows through **40 numbered syscalls**, every one capability-gated:

| Range | Category | Example Syscalls |
|-------|----------|-----------------|
| `0x00–0x0F` | **Process** | spawn, exit, yield, sleep, get_id, fork |
| `0x10–0x1F` | **Memory** | alloc, free, mmap, munmap, mem_info |
| `0x20–0x2F` | **IPC** | send, recv, try_recv, create_channel, close |
| `0x30–0x3F` | **VectorFS** | open, close, read, write, search, cluster |
| `0x40–0x4F` | **Network** | connect, listen, send, recv, close |
| `0x50–0x5F` | **Crypto** | encrypt, decrypt, sign, verify, keygen, hash |
| `0x60–0x6F` | **Admin** | inspect, kill, cap_delegate, cap_revoke |

**Every syscall**:
1. Checks the calling agent's capability token
2. Validates resource quotas
3. Executes the operation
4. Logs to audit trail
5. Returns result or error

---

## VectorFS

A FUSE-mountable filesystem where **vectors are first-class citizens**:

```
/vectorfs/
├── documents/           ← cluster
│   ├── paper_01.vec     ← 768-dim embedding
│   ├── paper_02.vec
│   └── paper_03.vec
├── code/                ← cluster
│   ├── module_a.vec
│   └── module_b.vec
└── .search              ← virtual file: write query, read results
```

- **Files are vectors** — stored as dense float arrays
- **Directories are clusters** — cosine similarity within clusters
- **Search is the primary operation** — `ls` returns vectors ranked by similarity
- **FUSE integration** — mount as a real filesystem, use standard tools
- **Metadata** — JSON metadata attached to each vector (source, timestamp, tags)

---

## Python Bindings

Full PyO3 integration for ML/AI ecosystem compatibility:

```python
import neural_kernel as nk

# Boot the kernel
kernel = nk.NeuralKernelBridge(max_agents=256, time_slice_ms=5)

# Spawn agents with capabilities
agent = kernel.spawn_agent("rag_worker", priority="high", capabilities=[
    "execute", "vector_read", "vector_search", "model_infer"
])

# CFS scheduling
next_agent = kernel.schedule()  # picks lowest vruntime

# IPC between agents
kernel.ipc_send("search_channel", {"query": "quantum computing"}, sender=agent_a)
msg = kernel.ipc_recv("search_channel")

# Capability-gated operations
kernel.grant_capability(agent_id, "crypto_encrypt")
kernel.revoke_capability(agent_id, "crypto_encrypt")
```

---

## MOL Language Integration

Neural Kernel agents can be orchestrated from the [MOL language](https://github.com/crux-ecosystem/mol-lang):

```mol
-- Spawn a fleet of 4 specialized agents
nk_init({max_agents: 256, time_slice_ms: 5})

let rag be nk_agent_spawn("rag_worker", "high", [
    "execute", "vector_read", "vector_search", "model_infer"
])

let crypto be nk_agent_spawn("crypto_engine", "realtime", [
    "execute", "crypto_encrypt", "crypto_decrypt"
])

-- IPC: RAG sends results to Crypto for encryption
nk_ipc_send("results_channel", {data: results}, rag.agent_id)
let msg be nk_ipc_recv("results_channel")

-- CFS picks the next agent to run
let next be nk_schedule()
show f"Scheduled: {next.scheduled}"
```

---

## Performance Characteristics

| Component | Complexity | Notes |
|-----------|-----------|-------|
| Scheduler pick-next | O(log n) | Red-black tree over vruntime |
| Capability check | O(1) | Bitfield operations |
| IPC send/recv | O(1) amortized | Lock-free SPSC ring buffer |
| VectorFS search (1M vectors) | O(log n) | HNSW approximate nearest neighbor |
| Agent spawn | O(1) | Pre-allocated arena slots |
| Syscall dispatch | O(1) | Indexed jump table |

---

## Part of Crux Sovereign AI Stack

Neural Kernel is **Phase 3** — the orchestration layer that governs all other components:

```
┌─────────────────────────────────────────────────────┐
│  MOL Language         — Sovereign compute            │
│  ─────────────────────────────────────────────────── │
│  De-RAG               — Encrypted decentralized data │
│  ─────────────────────────────────────────────────── │
│  Neural Kernel        — AI microkernel orchestration │  ◀── You are here
│  ─────────────────────────────────────────────────── │
│  IntraMind            — RAG engine (offline-first)   │
└─────────────────────────────────────────────────────┘
```

Neural Kernel wraps agents from every layer:
- **IntraMindAgent** — wraps the IntraMind RAG engine as a capability-gated agent (12 caps)
- **DeRAGAgent** — wraps De-RAG encryption/sharding nodes as agents (5 caps)
- **MOL scripts** — orchestrate agents with 38 ecosystem builtins

| Project | Role | Status |
|---------|------|--------|
| [MOL](https://github.com/crux-ecosystem/mol-lang) | Sovereign compute language | v1.1.0 |
| [De-RAG](https://github.com/crux-ecosystem/De-RAG-Showcase) | Encrypted decentralized RAG | v0.1.0 |
| **Neural Kernel** | AI microkernel & orchestration | v0.1.0 |
| [IntraMind](https://github.com/crux-ecosystem/IntraMind-Showcase) | Offline-first RAG engine | v1.1.0 |

---

## Roadmap

- [x] **v0.1.0** — Core kernel: CFS scheduler, 30+ capabilities, 40 syscalls, arena memory, SPSC IPC, sandbox, VectorFS, Python bridge
- [ ] **v0.2.0** — Real FUSE mount, seccomp-BPF sandbox, work-stealing scheduler
- [ ] **v0.3.0** — De-RAG integration (encrypted VectorFS, P2P agent migration)
- [ ] **v0.4.0** — GPU scheduling (CUDA/Vulkan compute capabilities)
- [ ] **v0.5.0** — WASM agent runtime (sandboxed WebAssembly execution)
- [ ] **v1.0.0** — Production kernel, formal verification of capability system

---

## Part of the Crux Ecosystem

| Project | Role | Language | Status |
|---------|------|----------|--------|
| [MOL](https://github.com/crux-ecosystem/mol-lang) | Sovereign compute language | Python | v1.1.0 |
| [De-RAG](https://github.com/crux-ecosystem/de-rag) | Decentralized encrypted data | Python | v0.1.0 |
| **Neural Kernel** | AI microkernel & orchestration | Rust | v0.1.0 |
| [IntraMind](https://github.com/crux-ecosystem/IntraMind-Showcase) | Offline-first RAG engine | Python | v1.1.0 |

## Build With Us

**CruxLabx** is building an operating system for AI agents — not another framework, not another wrapper, but a real kernel with scheduling, isolation, and capability-based security.

This is hard, low-level systems work. If you're the kind of engineer who reads kernel source code for fun, we're building something worth your time.

**We're looking for:**
- Systems programmers (Rust, kernel development, schedulers)
- Security engineers (capability systems, formal verification, sandboxing)
- ML infrastructure engineers (GPU scheduling, model serving)
- Distributed systems developers (agent migration, P2P consensus)

We believe AI agents deserve the same rigor as operating system processes — isolation, fair scheduling, and zero-trust security. If you agree, let's build it together.

**[github.com/crux-ecosystem](https://github.com/crux-ecosystem)**

---

<div align="center">

**Creator:** Mounesh Kodi

**Copyright © 2026 CruxLabx**

*An operating system for AI — built from first principles in Rust.*

</div>
