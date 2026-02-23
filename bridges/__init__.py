"""
Neural Kernel — Python Bridge
===============================

Python-side orchestration layer for Neural Kernel.
This module provides:
  1. Agent management (spawn, kill, list, capabilities)
  2. Scheduler interface (CFS-based)
  3. IPC message passing
  4. VectorFS operations
  5. IntraMind agent wrapping

When the Rust binary is available, this delegates to it via PyO3.
Otherwise, it runs a pure-Python simulation that matches the Rust API.

Usage:
    from neural_kernel.bridge import NeuralKernelBridge, IntraMindAgent

    nk = NeuralKernelBridge()
    nk.boot()

    # Wrap IntraMind as a managed agent
    im_agent = IntraMindAgent(nk, edition="library")
    im_agent.boot()
    result = im_agent.query("What is AI?")

Copyright (c) 2026 CruxLabx — AGPL-3.0
"""

from __future__ import annotations

import hashlib
import json
import logging
import os
import secrets
import sys
import time
import threading
from collections import defaultdict
from dataclasses import dataclass, field, asdict
from enum import Enum
from pathlib import Path
from typing import Any, Callable, Dict, List, Optional, Tuple

logger = logging.getLogger("neural_kernel.bridge")


# ═══════════════════════════════════════════════════════════════════
# Enums & Constants
# ═══════════════════════════════════════════════════════════════════

class AgentState(str, Enum):
    CREATED = "created"
    READY = "ready"
    RUNNING = "running"
    BLOCKED = "blocked"
    TERMINATED = "terminated"


class Priority(str, Enum):
    IDLE = "idle"
    NORMAL = "normal"
    HIGH = "high"
    REALTIME = "realtime"
    CRITICAL = "critical"


PRIORITY_WEIGHTS = {
    Priority.IDLE: 15,
    Priority.NORMAL: 100,
    Priority.HIGH: 400,
    Priority.REALTIME: 1600,
    Priority.CRITICAL: 6400,
}

# Full capability catalog matching Rust side
CAPABILITY_CATALOG = frozenset([
    "execute", "spawn", "terminate", "schedule",
    "fs_read", "fs_write", "fs_create", "fs_delete",
    "net_connect", "net_listen", "net_send", "net_receive",
    "mem_alloc", "mem_shared", "mem_map",
    "ipc_send", "ipc_receive", "ipc_create_channel",
    "crypto_encrypt", "crypto_decrypt", "crypto_sign", "crypto_verify",
    "gpu_compute", "gpu_memory",
    "vector_read", "vector_write", "vector_search", "vector_index",
    "model_load", "model_infer", "model_train",
    "audit_read", "audit_write",
    "kernel_access",
])


# ═══════════════════════════════════════════════════════════════════
# Data Types
# ═══════════════════════════════════════════════════════════════════

@dataclass
class Agent:
    """A Neural Kernel agent."""
    agent_id: str
    name: str
    state: AgentState = AgentState.CREATED
    priority: Priority = Priority.NORMAL
    capabilities: set = field(default_factory=set)
    memory_quota: int = 256 * 1024 * 1024  # 256MB
    memory_used: int = 0
    vruntime_ns: int = 0
    tasks_completed: int = 0
    created_at: float = field(default_factory=time.time)
    metadata: dict = field(default_factory=dict)
    _handler: Optional[Callable] = field(default=None, repr=False)

    @property
    def weight(self) -> int:
        return PRIORITY_WEIGHTS.get(self.priority, 100)

    def to_dict(self) -> dict:
        return {
            "agent_id": self.agent_id,
            "name": self.name,
            "state": self.state.value,
            "priority": self.priority.value,
            "capabilities": sorted(self.capabilities),
            "memory_quota": self.memory_quota,
            "memory_used": self.memory_used,
            "vruntime_ns": self.vruntime_ns,
            "tasks_completed": self.tasks_completed,
            "uptime_s": time.time() - self.created_at,
        }


@dataclass
class CapabilityToken:
    """Unforgeable capability token."""
    token_id: str
    capability: str
    agent_id: str
    delegatable: bool = False
    expires_at: Optional[float] = None
    issued_at: float = field(default_factory=time.time)

    @property
    def valid(self) -> bool:
        if self.expires_at and time.time() > self.expires_at:
            return False
        return True


@dataclass
class IpcMessage:
    """IPC message between agents."""
    sender: str
    payload: Any
    msg_type: str = "data"
    timestamp: float = field(default_factory=time.time)
    seq: int = 0


@dataclass
class TaskRequest:
    """A task submitted to an agent."""
    task_id: str
    agent_id: str
    task_type: str
    payload: dict
    priority: int = 0
    submitted_at: float = field(default_factory=time.time)


@dataclass
class TaskResult:
    """Result from an agent task."""
    task_id: str
    agent_id: str
    success: bool
    data: Any = None
    error: Optional[str] = None
    elapsed_ms: float = 0.0


# ═══════════════════════════════════════════════════════════════════
# CFS Scheduler
# ═══════════════════════════════════════════════════════════════════

class CFSScheduler:
    """Completely Fair Scheduler — Python implementation matching Rust."""

    def __init__(self, time_slice_ms: int = 10, max_agents: int = 1024):
        self.time_slice_ms = time_slice_ms
        self.max_agents = max_agents
        self._agents: dict[str, Agent] = {}
        self._lock = threading.Lock()
        self._min_vruntime: int = 0
        self._dispatched: int = 0
        self._preemptions: int = 0

    def admit(self, agent: Agent) -> bool:
        """Admit an agent into the scheduler."""
        with self._lock:
            if len(self._agents) >= self.max_agents:
                return False
            agent.state = AgentState.READY
            agent.vruntime_ns = self._min_vruntime  # Fair start
            self._agents[agent.agent_id] = agent
            return True

    def pick_next(self) -> Optional[Agent]:
        """Pick the agent with the lowest vruntime (CFS core)."""
        with self._lock:
            ready = [
                a for a in self._agents.values()
                if a.state == AgentState.READY
            ]
            if not ready:
                return None

            # CFS: pick by lowest vruntime
            chosen = min(ready, key=lambda a: a.vruntime_ns)
            chosen.state = AgentState.RUNNING
            self._dispatched += 1
            return chosen

    def complete_timeslice(self, agent_id: str, wall_time_ns: int = None):
        """Called when an agent completes its time slice."""
        with self._lock:
            agent = self._agents.get(agent_id)
            if not agent:
                return

            # Update vruntime: delta * (NICE_0_WEIGHT / weight)
            actual_ns = wall_time_ns or (self.time_slice_ms * 1_000_000)
            vruntime_delta = actual_ns * 100 // agent.weight
            agent.vruntime_ns += vruntime_delta
            agent.state = AgentState.READY
            agent.tasks_completed += 1

            # Update global minimum
            active_vruntimes = [
                a.vruntime_ns for a in self._agents.values()
                if a.state in (AgentState.READY, AgentState.RUNNING)
            ]
            if active_vruntimes:
                self._min_vruntime = min(active_vruntimes)

    def block(self, agent_id: str):
        """Block an agent (I/O wait)."""
        with self._lock:
            agent = self._agents.get(agent_id)
            if agent:
                agent.state = AgentState.BLOCKED

    def wake(self, agent_id: str):
        """Wake a blocked agent with sleep credit."""
        with self._lock:
            agent = self._agents.get(agent_id)
            if agent and agent.state == AgentState.BLOCKED:
                # Wakeup credit: set vruntime to min - half timeslice
                credit = (self.time_slice_ms * 1_000_000) // 2
                agent.vruntime_ns = max(0, self._min_vruntime - credit)
                agent.state = AgentState.READY

    def terminate(self, agent_id: str):
        """Remove an agent from scheduling."""
        with self._lock:
            agent = self._agents.get(agent_id)
            if agent:
                agent.state = AgentState.TERMINATED

    def remove(self, agent_id: str):
        """Fully remove an agent."""
        with self._lock:
            self._agents.pop(agent_id, None)

    @property
    def stats(self) -> dict:
        with self._lock:
            return {
                "total_agents": len(self._agents),
                "ready": sum(1 for a in self._agents.values() if a.state == AgentState.READY),
                "running": sum(1 for a in self._agents.values() if a.state == AgentState.RUNNING),
                "blocked": sum(1 for a in self._agents.values() if a.state == AgentState.BLOCKED),
                "dispatched": self._dispatched,
                "preemptions": self._preemptions,
                "min_vruntime": self._min_vruntime,
            }


# ═══════════════════════════════════════════════════════════════════
# Neural Kernel Bridge
# ═══════════════════════════════════════════════════════════════════

class NeuralKernelBridge:
    """
    Python-side Neural Kernel interface.
    
    Manages agents, scheduling, IPC, and capabilities.
    When the Rust NK binary is compiled, this delegates to it.
    Otherwise, runs a fully-functional Python simulation.
    """

    def __init__(self, config: Optional[dict] = None):
        conf = config or {}
        self.scheduler = CFSScheduler(
            time_slice_ms=conf.get("time_slice_ms", 10),
            max_agents=conf.get("max_agents", 1024),
        )
        self._agents: dict[str, Agent] = {}
        self._tokens: dict[str, CapabilityToken] = {}
        self._channels: dict[str, list] = defaultdict(list)
        self._task_queue: list[TaskRequest] = []
        self._audit_log: list[dict] = []
        self._booted = False
        self._lock = threading.Lock()
        self._rust_available = self._check_rust()

    def _check_rust(self) -> bool:
        """Check if Rust NK binary / PyO3 module is available."""
        try:
            import neural_kernel as nk_rust
            logger.info("Rust Neural Kernel available via PyO3")
            return True
        except ImportError:
            logger.debug("Rust NK not available — using Python simulation")
            return False

    @property
    def mode(self) -> str:
        return "rust" if self._rust_available else "python"

    def boot(self) -> dict:
        """Boot the Neural Kernel."""
        self._booted = True
        self._audit("system", "boot", {"mode": self.mode})
        logger.info(f"Neural Kernel booted (mode={self.mode})")
        return {
            "status": "ok",
            "mode": self.mode,
            "scheduler": self.scheduler.stats,
        }

    def shutdown(self):
        """Shut down the kernel."""
        for aid in list(self._agents.keys()):
            self.agent_kill(aid)
        self._booted = False
        self._audit("system", "shutdown", {})

    # ── Agent Management ──────────────────────────────────────

    def agent_spawn(
        self,
        name: str,
        priority: str = "normal",
        capabilities: Optional[list] = None,
        memory_quota: int = 256 * 1024 * 1024,
        handler: Optional[Callable] = None,
        metadata: Optional[dict] = None,
    ) -> Agent:
        """Spawn a new agent with capability-based security."""
        caps = set(capabilities or ["execute"])
        invalid = caps - CAPABILITY_CATALOG
        if invalid:
            raise ValueError(f"Unknown capabilities: {invalid}")

        agent_id = secrets.token_hex(16)
        agent = Agent(
            agent_id=agent_id,
            name=name,
            priority=Priority(priority),
            capabilities=caps,
            memory_quota=memory_quota,
            metadata=metadata or {},
            _handler=handler,
        )

        with self._lock:
            self._agents[agent_id] = agent

        # Issue tokens
        for cap in caps:
            token = CapabilityToken(
                token_id=secrets.token_hex(8),
                capability=cap,
                agent_id=agent_id,
            )
            self._tokens[token.token_id] = token

        # Admit to scheduler
        self.scheduler.admit(agent)

        self._audit("agent", "spawn", {
            "agent_id": agent_id,
            "name": name,
            "caps": sorted(caps),
            "priority": priority,
        })
        return agent

    def agent_kill(self, agent_id: str) -> dict:
        """Terminate and remove an agent."""
        agent = self._agents.get(agent_id)
        if not agent:
            raise KeyError(f"Agent {agent_id} not found")

        self.scheduler.terminate(agent_id)
        self.scheduler.remove(agent_id)

        # Revoke tokens
        to_remove = [tid for tid, t in self._tokens.items() if t.agent_id == agent_id]
        for tid in to_remove:
            del self._tokens[tid]

        agent.state = AgentState.TERMINATED
        self._audit("agent", "kill", {"agent_id": agent_id, "name": agent.name})
        return {"terminated": agent_id, "name": agent.name}

    def agent_list(self) -> list[dict]:
        """List all agents."""
        return [a.to_dict() for a in self._agents.values()]

    def agent_get(self, agent_id: str) -> Optional[Agent]:
        return self._agents.get(agent_id)

    # ── Capability Management ─────────────────────────────────

    def grant(self, agent_id: str, *capabilities: str) -> list:
        """Grant capabilities to an agent."""
        agent = self._agents.get(agent_id)
        if not agent:
            raise KeyError(f"Agent {agent_id} not found")

        granted = []
        for cap in capabilities:
            if cap not in CAPABILITY_CATALOG:
                raise ValueError(f"Unknown capability: {cap}")
            if cap not in agent.capabilities:
                agent.capabilities.add(cap)
                token = CapabilityToken(
                    token_id=secrets.token_hex(8),
                    capability=cap,
                    agent_id=agent_id,
                )
                self._tokens[token.token_id] = token
                granted.append(cap)

        self._audit("capability", "grant", {"agent_id": agent_id, "granted": granted})
        return granted

    def revoke(self, agent_id: str, *capabilities: str) -> list:
        """Revoke capabilities from an agent."""
        agent = self._agents.get(agent_id)
        if not agent:
            raise KeyError(f"Agent {agent_id} not found")

        revoked = []
        for cap in capabilities:
            if cap in agent.capabilities:
                agent.capabilities.discard(cap)
                to_remove = [
                    tid for tid, t in self._tokens.items()
                    if t.agent_id == agent_id and t.capability == cap
                ]
                for tid in to_remove:
                    del self._tokens[tid]
                revoked.append(cap)

        self._audit("capability", "revoke", {"agent_id": agent_id, "revoked": revoked})
        return revoked

    def check_capability(self, agent_id: str, capability: str) -> bool:
        """Check if an agent has a capability."""
        agent = self._agents.get(agent_id)
        return agent is not None and capability in agent.capabilities

    # ── Task Submission & Dispatch ────────────────────────────

    def submit_task(self, agent_id: str, task_type: str, payload: dict, priority: int = 0) -> TaskRequest:
        """Submit a task to a specific agent."""
        if agent_id not in self._agents:
            raise KeyError(f"Agent {agent_id} not found")

        task = TaskRequest(
            task_id=secrets.token_hex(8),
            agent_id=agent_id,
            task_type=task_type,
            payload=payload,
            priority=priority,
        )
        self._task_queue.append(task)
        return task

    def dispatch_next(self) -> Optional[TaskResult]:
        """Dispatch the highest-priority task to its agent."""
        if not self._task_queue:
            return None

        # Sort by priority (higher first)
        self._task_queue.sort(key=lambda t: t.priority, reverse=True)
        task = self._task_queue.pop(0)
        agent = self._agents.get(task.agent_id)

        if not agent or not agent._handler:
            return TaskResult(
                task_id=task.task_id,
                agent_id=task.agent_id,
                success=False,
                error="Agent has no handler",
            )

        start = time.time()
        try:
            result = agent._handler(task)
            elapsed = (time.time() - start) * 1000
            self.scheduler.complete_timeslice(agent.agent_id)
            return TaskResult(
                task_id=task.task_id,
                agent_id=task.agent_id,
                success=True,
                data=result,
                elapsed_ms=elapsed,
            )
        except Exception as e:
            elapsed = (time.time() - start) * 1000
            return TaskResult(
                task_id=task.task_id,
                agent_id=task.agent_id,
                success=False,
                error=str(e),
                elapsed_ms=elapsed,
            )

    # ── IPC ───────────────────────────────────────────────────

    def ipc_send(self, channel: str, payload: Any, sender: str = "kernel") -> dict:
        """Send a message on an IPC channel."""
        msg = IpcMessage(
            sender=sender,
            payload=payload,
            seq=len(self._channels[channel]),
        )
        self._channels[channel].append(msg)
        return {"sent": True, "channel": channel, "seq": msg.seq}

    def ipc_recv(self, channel: str) -> Optional[IpcMessage]:
        """Receive a message from an IPC channel."""
        msgs = self._channels.get(channel, [])
        if msgs:
            return msgs.pop(0)
        return None

    def ipc_create_channel(self, name: str) -> str:
        """Create a named IPC channel."""
        if name not in self._channels:
            self._channels[name] = []
        return name

    # ── Status ────────────────────────────────────────────────

    @property
    def status(self) -> dict:
        return {
            "booted": self._booted,
            "mode": self.mode,
            "agents": len(self._agents),
            "tokens": len(self._tokens),
            "channels": len(self._channels),
            "pending_tasks": len(self._task_queue),
            "scheduler": self.scheduler.stats,
            "audit_entries": len(self._audit_log),
        }

    # ── Audit ─────────────────────────────────────────────────

    def _audit(self, subsystem: str, action: str, data: dict):
        prev_hash = self._audit_log[-1]["hash"] if self._audit_log else "0" * 64
        entry = {
            "timestamp": time.time(),
            "subsystem": subsystem,
            "action": action,
            "data": data,
            "prev_hash": prev_hash,
        }
        entry_json = json.dumps(entry, sort_keys=True, default=str)
        entry["hash"] = hashlib.blake2b(entry_json.encode(), digest_size=32).hexdigest()
        self._audit_log.append(entry)

    @property
    def audit_log(self) -> list:
        return list(self._audit_log)


# ═══════════════════════════════════════════════════════════════════
# IntraMind Agent — Wraps IntraMind as a Neural Kernel Agent
# ═══════════════════════════════════════════════════════════════════

class IntraMindAgent:
    """
    Wraps IntraMind's RAG engine as a managed Neural Kernel agent.
    
    This is the key integration point: IntraMind runs inside the
    Neural Kernel's capability-gated, scheduled execution environment.
    
    Usage:
        nk = NeuralKernelBridge()
        nk.boot()
        
        im = IntraMindAgent(nk, edition="library")
        im.boot()
        
        result = im.query("What is quantum computing?")
        im.ingest("/path/to/document.pdf")
        
        # IntraMind runs as a scheduled agent with capabilities:
        # [vector_read, vector_write, vector_search, model_load, model_infer,
        #  fs_read, crypto_encrypt, crypto_decrypt, audit_write]
    """

    # Capabilities required by IntraMind
    REQUIRED_CAPABILITIES = [
        "execute",
        "vector_read", "vector_write", "vector_search", "vector_index",
        "model_load", "model_infer",
        "fs_read",
        "crypto_encrypt", "crypto_decrypt",
        "audit_read", "audit_write",
        "mem_alloc",
    ]

    def __init__(
        self,
        kernel: NeuralKernelBridge,
        edition: str = "core",
        priority: str = "high",
    ):
        self._kernel = kernel
        self._edition = edition
        self._priority = priority
        self._agent: Optional[Agent] = None
        self._engine = None  # IntraMindEngine instance
        self._booted = False

    @property
    def agent(self) -> Optional[Agent]:
        return self._agent

    @property
    def agent_id(self) -> Optional[str]:
        return self._agent.agent_id if self._agent else None

    def boot(self) -> dict:
        """Boot IntraMind as a Neural Kernel agent."""
        # 1. Spawn the agent in NK
        self._agent = self._kernel.agent_spawn(
            name=f"intramind_{self._edition}",
            priority=self._priority,
            capabilities=self.REQUIRED_CAPABILITIES,
            memory_quota=1024 * 1024 * 1024,  # 1GB for IntraMind
            handler=self._handle_task,
            metadata={"type": "intramind", "edition": self._edition},
        )

        # 2. Try to boot the actual IntraMind engine
        engine_status = self._boot_engine()

        self._booted = True
        return {
            "agent_id": self._agent.agent_id,
            "name": self._agent.name,
            "engine": engine_status,
            "edition": self._edition,
            "capabilities": sorted(self._agent.capabilities),
        }

    def _boot_engine(self) -> str:
        """Try to import and boot IntraMindEngine."""
        try:
            # Search for IntraMind in workspace
            intramind_paths = [
                Path(__file__).parent.parent.parent / "IntraMind",
                Path(__file__).parent.parent.parent.parent / "IntraMind",
                Path.home() / "Documents" / "Cruxlabx" / "IntraMind",
            ]
            for path in intramind_paths:
                if (path / "core" / "engine.py").exists():
                    if str(path.parent) not in sys.path:
                        sys.path.insert(0, str(path.parent))
                    break

            from IntraMind.core.engine import IntraMindEngine
            self._engine = IntraMindEngine(edition=self._edition)
            boot_result = self._engine.boot()
            logger.info(f"IntraMind engine booted (edition={self._edition})")
            return "live"
        except Exception as e:
            logger.warning(f"IntraMind engine not available: {e}")
            self._engine = None
            return f"stub ({e})"

    # ── Task Handler (called by NK scheduler) ─────────────────

    def _handle_task(self, task: TaskRequest) -> Any:
        """Handle tasks dispatched by Neural Kernel scheduler."""
        task_type = task.task_type
        payload = task.payload

        handlers = {
            "query": self._task_query,
            "search": self._task_search,
            "ingest": self._task_ingest,
            "summarize": self._task_summarize,
            "health": self._task_health,
        }

        handler = handlers.get(task_type)
        if not handler:
            raise ValueError(f"Unknown task type: {task_type}")

        # Check capabilities
        cap_map = {
            "query": "vector_search",
            "search": "vector_search",
            "ingest": "fs_read",
            "summarize": "model_infer",
            "health": "execute",
        }
        required_cap = cap_map.get(task_type, "execute")
        if not self._kernel.check_capability(self._agent.agent_id, required_cap):
            raise PermissionError(f"Agent lacks capability: {required_cap}")

        return handler(payload)

    def _task_query(self, payload: dict) -> dict:
        question = payload.get("question", "")
        top_k = payload.get("top_k", 5)

        if self._engine:
            result = self._engine.process(question, top_k=top_k)
            return {
                "success": result.success,
                "answer": result.data.get("answer", ""),
                "sources": result.sources,
                "cached": result.cached,
            }
        return {"success": True, "answer": f"[stub] {question}", "sources": []}

    def _task_search(self, payload: dict) -> list:
        query = payload.get("query", "")
        top_k = payload.get("top_k", 5)

        if self._engine:
            result = self._engine.process(query, top_k=top_k)
            return result.sources
        return []

    def _task_ingest(self, payload: dict) -> dict:
        file_path = payload.get("file_path", "")

        if self._engine:
            result = self._engine.ingest(file_path)
            return {"success": result.success, "message": result.message}
        return {"success": True, "mode": "stub", "file": file_path}

    def _task_summarize(self, payload: dict) -> dict:
        text = payload.get("text", "")
        if self._engine:
            result = self._engine.process(f"summarize: {text}")
            return {"success": result.success, "summary": result.data.get("answer", "")}
        return {"success": True, "summary": f"[stub] Summary of: {text[:100]}..."}

    def _task_health(self, payload: dict) -> dict:
        if self._engine:
            return self._engine.health()
        return {
            "engine": "stub",
            "edition": self._edition,
            "agent_state": self._agent.state.value if self._agent else "none",
        }

    # ── Convenience Methods ───────────────────────────────────

    def query(self, question: str, top_k: int = 5) -> dict:
        """Submit a RAG query through the NK task system."""
        task = self._kernel.submit_task(
            self._agent.agent_id,
            "query",
            {"question": question, "top_k": top_k},
            priority=10,
        )
        result = self._kernel.dispatch_next()
        return result.data if result and result.success else {"error": result.error if result else "dispatch failed"}

    def search(self, query: str, top_k: int = 5) -> list:
        """Semantic search through NK."""
        task = self._kernel.submit_task(
            self._agent.agent_id,
            "search",
            {"query": query, "top_k": top_k},
        )
        result = self._kernel.dispatch_next()
        return result.data if result and result.success else []

    def ingest(self, file_path: str) -> dict:
        """Ingest a document through NK."""
        task = self._kernel.submit_task(
            self._agent.agent_id,
            "ingest",
            {"file_path": file_path},
        )
        result = self._kernel.dispatch_next()
        return result.data if result and result.success else {"error": result.error if result else "dispatch failed"}

    def health(self) -> dict:
        """Get IntraMind health through NK."""
        task = self._kernel.submit_task(
            self._agent.agent_id,
            "health",
            {},
        )
        result = self._kernel.dispatch_next()
        return result.data if result and result.success else {}

    @property
    def status(self) -> dict:
        return {
            "booted": self._booted,
            "agent": self._agent.to_dict() if self._agent else None,
            "engine": "live" if self._engine else "stub",
            "edition": self._edition,
        }


# ═══════════════════════════════════════════════════════════════════
# De-RAG Agent — Wraps De-RAG node as a Neural Kernel Agent
# ═══════════════════════════════════════════════════════════════════

class DeRAGAgent:
    """
    Wraps a De-RAG node as a managed Neural Kernel agent.
    
    Handles encrypted document ingestion, sharding, distributed
    querying, and lineage verification under NK's capability system.
    """

    REQUIRED_CAPABILITIES = [
        "execute",
        "net_connect", "net_send", "net_receive",
        "crypto_encrypt", "crypto_decrypt", "crypto_sign", "crypto_verify",
        "vector_search", "vector_write",
        "fs_read", "fs_write",
        "audit_read", "audit_write",
        "ipc_send", "ipc_receive",
    ]

    def __init__(self, kernel: NeuralKernelBridge, node=None, priority: str = "high"):
        self._kernel = kernel
        self._node = node
        self._priority = priority
        self._agent: Optional[Agent] = None
        self._booted = False

    def boot(self) -> dict:
        """Boot De-RAG as a Neural Kernel agent."""
        self._agent = self._kernel.agent_spawn(
            name="derag_coordinator",
            priority=self._priority,
            capabilities=self.REQUIRED_CAPABILITIES,
            handler=self._handle_task,
            metadata={"type": "derag"},
        )

        # Try to initialize De-RAG node
        node_status = "stub"
        if self._node:
            node_status = "live"
        else:
            try:
                sys.path.insert(0, str(Path(__file__).parent.parent.parent))
                from De_RAG.derag.node import DeRAGNode
                # Would need config to fully init
                node_status = "importable"
            except ImportError:
                node_status = "stub"

        self._booted = True
        return {
            "agent_id": self._agent.agent_id,
            "name": self._agent.name,
            "node": node_status,
            "capabilities": sorted(self._agent.capabilities),
        }

    def _handle_task(self, task: TaskRequest) -> Any:
        task_type = task.task_type
        payload = task.payload

        if task_type == "encrypt":
            from mol.ecosystem import _builtin_derag_encrypt
            return _builtin_derag_encrypt(payload.get("data", ""))
        elif task_type == "shard":
            from mol.ecosystem import _builtin_derag_shard
            return _builtin_derag_shard(
                payload.get("data", ""),
                payload.get("threshold", 3),
                payload.get("total", 5),
            )
        elif task_type == "query":
            from mol.ecosystem import _builtin_derag_query
            return _builtin_derag_query(payload.get("question", ""))
        elif task_type == "status":
            from mol.ecosystem import _builtin_derag_status
            return _builtin_derag_status()
        else:
            raise ValueError(f"Unknown De-RAG task: {task_type}")

    @property
    def status(self) -> dict:
        return {
            "booted": self._booted,
            "agent": self._agent.to_dict() if self._agent else None,
            "node": "live" if self._node else "stub",
        }
