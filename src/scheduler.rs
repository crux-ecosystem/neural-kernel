//! Scheduler — CFS-inspired preemptive agent scheduler
//!
//! Neural Kernel uses a Completely Fair Scheduler (CFS) variant
//! adapted for AI agents:
//!
//! - **vruntime tracking** — Each agent tracks "virtual runtime"
//!   normalized by priority weight. Higher-priority agents accumulate
//!   vruntime slower, getting proportionally more CPU time.
//!
//! - **Red-Black tree ready queue** — O(log n) pick-next using
//!   minimum vruntime. (Simplified to BTreeMap in this version.)
//!
//! - **Time slicing** — Configurable quantum (default 10ms).
//!   Agents are preempted when their quantum expires.
//!
//! - **Priority boost** — Agents waking from I/O get a small
//!   vruntime credit to improve interactive responsiveness.
//!
//! - **Load balancing** — Work-stealing across worker threads
//!   for multi-core utilization.
//!
//! Scheduling cycle:
//! ```text
//! pick_next() → dispatch(agent) → [agent runs] → preempt/yield → enqueue()
//! ```

use std::collections::{BTreeMap, HashMap};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::agent::{Agent, AgentId, AgentState, BlockReason};

/// Scheduler configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchedulerConfig {
    /// Maximum concurrent agents
    pub max_agents: usize,

    /// Time quantum per scheduling round (default: 10ms)
    pub quantum: Duration,

    /// Number of worker threads
    pub num_workers: usize,

    /// Minimum granularity — don't preempt below this (default: 1ms)
    pub min_granularity: Duration,

    /// vruntime credit for I/O wakeup (reduces starvation)
    pub wakeup_credit_ns: u64,

    /// How often to rebalance across workers (default: 100ms)
    pub rebalance_interval: Duration,

    /// Target scheduling latency (all agents run within this window)
    pub target_latency: Duration,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            max_agents: 1024,
            quantum: Duration::from_millis(10),
            num_workers: num_cpus(),
            min_granularity: Duration::from_millis(1),
            wakeup_credit_ns: 1_000_000, // 1ms credit
            rebalance_interval: Duration::from_millis(100),
            target_latency: Duration::from_millis(100),
        }
    }
}

/// Get number of CPU cores
fn num_cpus() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
}

/// Scheduler statistics
#[derive(Debug, Default, Clone, Serialize)]
pub struct SchedulerStats {
    /// Total scheduling decisions made
    pub total_schedules: u64,
    /// Total context switches
    pub context_switches: u64,
    /// Total preemptions
    pub preemptions: u64,
    /// Total voluntary yields
    pub yields: u64,
    /// Number of agents currently ready
    pub ready_count: usize,
    /// Number of agents currently blocked
    pub blocked_count: usize,
    /// Number of agents currently running
    pub running_count: usize,
    /// Scheduler uptime
    pub uptime: Duration,
}

/// The main scheduler
pub struct Scheduler {
    /// Configuration
    pub config: SchedulerConfig,

    /// All agents by ID
    agents: HashMap<AgentId, Agent>,

    /// Ready queue (sorted by vruntime — lowest first gets picked)
    /// Key: (vruntime, agent_id) for stable ordering
    ready_queue: BTreeMap<(u64, AgentId), AgentId>,

    /// Blocked agents (waiting for events)
    blocked: HashMap<AgentId, BlockReason>,

    /// Currently running agents (one per worker)
    running: HashMap<usize, AgentId>, // worker_id → agent_id

    /// Minimum vruntime across all agents (for new agent initialization)
    min_vruntime: u64,

    /// Statistics
    stats: SchedulerStats,

    /// Start time
    start_time: Instant,
}

impl Scheduler {
    /// Create a new scheduler
    pub fn new(config: SchedulerConfig) -> Self {
        Self {
            config,
            agents: HashMap::new(),
            ready_queue: BTreeMap::new(),
            blocked: HashMap::new(),
            running: HashMap::new(),
            min_vruntime: 0,
            stats: SchedulerStats::default(),
            start_time: Instant::now(),
        }
    }

    /// Admit a new agent into the scheduler
    pub fn admit(&mut self, mut agent: Agent) -> Result<AgentId, SchedulerError> {
        if self.agents.len() >= self.config.max_agents {
            return Err(SchedulerError::MaxAgentsReached(self.config.max_agents));
        }

        let id = agent.id;

        // Initialize vruntime to current minimum (so it doesn't starve others)
        agent.vruntime = self.min_vruntime;

        // Calculate time slice based on priority and number of agents
        agent.time_slice_remaining = self.calculate_time_slice(&agent);

        // Transition to Ready
        agent
            .transition(AgentState::Ready)
            .map_err(|e| SchedulerError::StateTransition(e.to_string()))?;

        // Add to ready queue
        self.ready_queue.insert((agent.vruntime, id), id);
        self.agents.insert(id, agent);

        tracing::debug!(?id, "Agent admitted to scheduler");
        Ok(id)
    }

    /// Pick the next agent to run (lowest vruntime wins)
    pub fn pick_next(&mut self) -> Option<AgentId> {
        // Pop from front of BTreeMap (lowest vruntime)
        let key = *self.ready_queue.keys().next()?;
        let (_key, id) = self.ready_queue.remove_entry(&key)?;

        self.stats.total_schedules += 1;
        Some(id)
    }

    /// Dispatch an agent to a worker thread
    pub fn dispatch(&mut self, agent_id: AgentId, worker_id: usize) -> Result<(), SchedulerError> {
        let agent = self
            .agents
            .get_mut(&agent_id)
            .ok_or(SchedulerError::AgentNotFound(agent_id))?;

        agent
            .transition(AgentState::Running)
            .map_err(|e| SchedulerError::StateTransition(e.to_string()))?;

        self.running.insert(worker_id, agent_id);
        self.stats.context_switches += 1;

        Ok(())
    }

    /// Preempt a running agent (quantum expired)
    pub fn preempt(&mut self, agent_id: AgentId, elapsed: Duration) -> Result<(), SchedulerError> {
        let agent = self
            .agents
            .get_mut(&agent_id)
            .ok_or(SchedulerError::AgentNotFound(agent_id))?;

        // Charge CPU time
        agent.charge_time(elapsed);

        // Transition back to Ready
        agent
            .transition(AgentState::Ready)
            .map_err(|e| SchedulerError::StateTransition(e.to_string()))?;

        // Recalculate time slice — extract weight first to avoid borrow conflict
        let agent_weight = agent.weight();
        let vruntime = agent.vruntime;
        // End mutable borrow of agent
        let _ = agent;
        let time_slice = self.calculate_time_slice_by_weight(agent_weight);
        if let Some(agent) = self.agents.get_mut(&agent_id) {
            agent.time_slice_remaining = time_slice;
        }

        // Re-enqueue with updated vruntime
        self.ready_queue.insert((vruntime, agent_id), agent_id);

        // Update min_vruntime
        if let Some((&(min_vrt, _), _)) = self.ready_queue.iter().next() {
            self.min_vruntime = min_vrt;
        }

        // Remove from running
        self.running.retain(|_, id| *id != agent_id);
        self.stats.preemptions += 1;

        Ok(())
    }

    /// Block an agent (waiting for I/O, IPC, etc.)
    pub fn block(
        &mut self,
        agent_id: AgentId,
        reason: BlockReason,
        elapsed: Duration,
    ) -> Result<(), SchedulerError> {
        let agent = self
            .agents
            .get_mut(&agent_id)
            .ok_or(SchedulerError::AgentNotFound(agent_id))?;

        agent.charge_time(elapsed);
        agent
            .transition(AgentState::Blocked(reason))
            .map_err(|e| SchedulerError::StateTransition(e.to_string()))?;

        self.blocked.insert(agent_id, reason);
        self.running.retain(|_, id| *id != agent_id);

        Ok(())
    }

    /// Wake up a blocked agent
    pub fn wake(&mut self, agent_id: AgentId) -> Result<(), SchedulerError> {
        let agent = self
            .agents
            .get_mut(&agent_id)
            .ok_or(SchedulerError::AgentNotFound(agent_id))?;

        agent
            .transition(AgentState::Ready)
            .map_err(|e| SchedulerError::StateTransition(e.to_string()))?;

        // Give wakeup credit (prevent starvation of I/O-bound agents)
        if agent.vruntime > self.config.wakeup_credit_ns {
            agent.vruntime = agent
                .vruntime
                .saturating_sub(self.config.wakeup_credit_ns);
        }

        // Ensure vruntime doesn't go below global minimum
        agent.vruntime = agent.vruntime.max(self.min_vruntime);

        // Extract weight before dropping mutable borrow
        let agent_weight = agent.weight();
        let vruntime = agent.vruntime;
        // End mutable borrow of agent
        let _ = agent;
        let time_slice = self.calculate_time_slice_by_weight(agent_weight);
        if let Some(agent) = self.agents.get_mut(&agent_id) {
            agent.time_slice_remaining = time_slice;
        }

        self.ready_queue.insert((vruntime, agent_id), agent_id);
        self.blocked.remove(&agent_id);

        Ok(())
    }

    /// Terminate an agent
    pub fn terminate(&mut self, agent_id: AgentId, exit_code: i32) -> Result<(), SchedulerError> {
        let agent = self
            .agents
            .get_mut(&agent_id)
            .ok_or(SchedulerError::AgentNotFound(agent_id))?;

        agent
            .transition(AgentState::Terminated(exit_code))
            .map_err(|e| SchedulerError::StateTransition(e.to_string()))?;

        // Clean up from all queues
        self.ready_queue.retain(|_, id| *id != agent_id);
        self.blocked.remove(&agent_id);
        self.running.retain(|_, id| *id != agent_id);

        tracing::info!(?agent_id, exit_code, "Agent terminated");
        Ok(())
    }

    /// Remove a terminated agent entirely
    pub fn reap(&mut self, agent_id: AgentId) -> Option<Agent> {
        if let Some(agent) = self.agents.get(&agent_id) {
            if matches!(agent.state, AgentState::Terminated(_)) {
                return self.agents.remove(&agent_id);
            }
        }
        None
    }

    /// Get agent by ID
    pub fn get_agent(&self, id: &AgentId) -> Option<&Agent> {
        self.agents.get(id)
    }

    /// Get mutable agent
    pub fn get_agent_mut(&mut self, id: &AgentId) -> Option<&mut Agent> {
        self.agents.get_mut(id)
    }

    /// List all agents
    pub fn list_agents(&self) -> Vec<&Agent> {
        self.agents.values().collect()
    }

    /// Get scheduler statistics
    pub fn stats(&self) -> SchedulerStats {
        SchedulerStats {
            total_schedules: self.stats.total_schedules,
            context_switches: self.stats.context_switches,
            preemptions: self.stats.preemptions,
            yields: self.stats.yields,
            ready_count: self.ready_queue.len(),
            blocked_count: self.blocked.len(),
            running_count: self.running.len(),
            uptime: self.start_time.elapsed(),
        }
    }

    /// Calculate time slice for an agent
    /// Calculate time slice given an agent weight (avoids borrow conflicts)
    fn calculate_time_slice_by_weight(&self, agent_weight: u64) -> Duration {
        let total_weight: u64 = self
            .agents
            .values()
            .filter(|a| a.is_alive())
            .map(|a| a.weight())
            .sum();

        if total_weight == 0 {
            return self.config.quantum;
        }

        let slice_ns = self.config.target_latency.as_nanos() as u64 * agent_weight / total_weight;
        let slice = Duration::from_nanos(slice_ns);
        slice.max(self.config.min_granularity).min(self.config.quantum)
    }

    fn calculate_time_slice(&self, agent: &Agent) -> Duration {
        let total_weight: u64 = self
            .agents
            .values()
            .filter(|a| a.is_alive())
            .map(|a| a.weight())
            .sum();

        if total_weight == 0 {
            return self.config.quantum;
        }

        // Proportional time slice based on weight
        let agent_weight = agent.weight();
        let slice_ns = self.config.target_latency.as_nanos() as u64 * agent_weight / total_weight;

        let slice = Duration::from_nanos(slice_ns);

        // Clamp between min_granularity and quantum
        slice.max(self.config.min_granularity).min(self.config.quantum)
    }

    /// Number of ready agents
    pub fn ready_count(&self) -> usize {
        self.ready_queue.len()
    }

    /// Number of total agents
    pub fn total_agents(&self) -> usize {
        self.agents.len()
    }
}

/// Scheduler errors
#[derive(Debug, thiserror::Error)]
pub enum SchedulerError {
    #[error("Maximum agents reached ({0})")]
    MaxAgentsReached(usize),

    #[error("Agent not found: {0:?}")]
    AgentNotFound(AgentId),

    #[error("State transition error: {0}")]
    StateTransition(String),

    #[error("Worker {0} is busy")]
    WorkerBusy(usize),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::Priority;
    use crate::capability::CapabilitySet;

    #[test]
    fn test_admit_and_pick() {
        let mut sched = Scheduler::new(SchedulerConfig::default());
        let agent = Agent::new("test-agent");
        let id = sched.admit(agent).unwrap();

        let next = sched.pick_next();
        assert_eq!(next, Some(id));
    }

    #[test]
    fn test_priority_ordering() {
        let mut sched = Scheduler::new(SchedulerConfig::default());

        // Low priority agent — gets high vruntime quickly
        let mut low = Agent::new("low");
        low.priority = Priority::Idle;
        let low_id = low.id;

        // High priority agent — low vruntime
        let mut high = Agent::new("high");
        high.priority = Priority::Critical;
        let high_id = high.id;

        let low_id = sched.admit(low).unwrap();
        let high_id = sched.admit(high).unwrap();

        // Manually set vruntimes to simulate scheduling history
        // (admit resets vruntime to min_vruntime)
        if let Some(agent) = sched.agents.get_mut(&low_id) {
            agent.vruntime = 1000;
        }
        if let Some(agent) = sched.agents.get_mut(&high_id) {
            agent.vruntime = 100;
        }

        // Re-insert into ready queue with correct vruntimes
        sched.ready_queue.clear();
        sched.ready_queue.insert((1000, low_id), low_id);
        sched.ready_queue.insert((100, high_id), high_id);

        // High-priority agent should be picked first (lower vruntime)
        let next = sched.pick_next().unwrap();
        assert_eq!(next, high_id);
    }

    #[test]
    fn test_preempt_and_reschedule() {
        let mut sched = Scheduler::new(SchedulerConfig::default());
        let agent = Agent::new("preempt-test");
        let id = sched.admit(agent).unwrap();

        let next = sched.pick_next().unwrap();
        sched.dispatch(next, 0).unwrap();

        // Preempt after 5ms
        sched.preempt(id, Duration::from_millis(5)).unwrap();

        // Should be back in ready queue
        assert_eq!(sched.ready_count(), 1);
    }

    #[test]
    fn test_block_and_wake() {
        let mut sched = Scheduler::new(SchedulerConfig::default());
        let agent = Agent::new("block-test");
        let id = sched.admit(agent).unwrap();

        sched.pick_next();
        sched.dispatch(id, 0).unwrap();

        // Block on I/O
        sched.block(id, BlockReason::IoWait, Duration::from_millis(2)).unwrap();
        assert_eq!(sched.ready_count(), 0);

        // Wake up
        sched.wake(id).unwrap();
        assert_eq!(sched.ready_count(), 1);
    }

    #[test]
    fn test_terminate_and_reap() {
        let mut sched = Scheduler::new(SchedulerConfig::default());
        let agent = Agent::new("term-test");
        let id = sched.admit(agent).unwrap();

        sched.pick_next();
        sched.dispatch(id, 0).unwrap();
        sched.terminate(id, 0).unwrap();

        let reaped = sched.reap(id);
        assert!(reaped.is_some());
        assert_eq!(sched.total_agents(), 0);
    }

    #[test]
    fn test_max_agents() {
        let config = SchedulerConfig {
            max_agents: 2,
            ..Default::default()
        };
        let mut sched = Scheduler::new(config);

        sched.admit(Agent::new("a1")).unwrap();
        sched.admit(Agent::new("a2")).unwrap();

        let result = sched.admit(Agent::new("a3"));
        assert!(result.is_err());
    }
}
