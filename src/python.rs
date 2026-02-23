//! Python bindings via PyO3
//!
//! Exposes Neural Kernel functionality to Python:
//!
//! ```python
//! import neural_kernel as nk
//!
//! # Create a scheduler
//! sched = nk.Scheduler(max_agents=1024)
//!
//! # Spawn an agent
//! agent = nk.Agent("my-agent", priority="high")
//! agent.grant_capability("fs_read")
//! agent.grant_capability("fs_search")
//!
//! sched.admit(agent)
//!
//! # VectorFS
//! vfs = nk.VectorStore(dim=768)
//! vfs.add("doc1.vec", cluster="documents", vector=[0.1, 0.2, ...])
//! results = vfs.search(query_vector, top_k=10)
//! ```
//!
//! Build with: `maturin build --features python`
//! Install with: `pip install neural-kernel`

// This module is only compiled when the `python` feature is enabled
#[cfg(feature = "python")]
mod python_bindings {
    use pyo3::prelude::*;
    use pyo3::types::{PyDict, PyList};

    use crate::agent::{Agent, AgentId, AgentState, Priority};
    use crate::capability::{Capability, CapabilitySet, CapabilitySetBuilder};
    use crate::scheduler::{Scheduler as RustScheduler, SchedulerConfig};
    use crate::vectorfs::{VectorStore as RustVectorStore, SearchResult};

    /// Python-exposed Agent
    #[pyclass(name = "Agent")]
    struct PyAgent {
        inner: Agent,
    }

    #[pymethods]
    impl PyAgent {
        #[new]
        #[pyo3(signature = (name, priority="normal"))]
        fn new(name: &str, priority: &str) -> PyResult<Self> {
            let p = match priority {
                "idle" => Priority::Idle,
                "normal" => Priority::Normal,
                "high" => Priority::High,
                "realtime" => Priority::Realtime,
                "critical" => Priority::Critical,
                _ => {
                    return Err(pyo3::exceptions::PyValueError::new_err(
                        format!("Invalid priority: {priority}. Use: idle, normal, high, realtime, critical"),
                    ))
                }
            };

            let mut agent = Agent::new(name);
            agent.priority = p;
            agent.capabilities = CapabilitySet::user_default();

            Ok(PyAgent { inner: agent })
        }

        /// Get agent ID as hex string
        #[getter]
        fn id(&self) -> String {
            self.inner.id.to_string()
        }

        /// Get agent name
        #[getter]
        fn name(&self) -> &str {
            &self.inner.name
        }

        /// Get state as string
        #[getter]
        fn state(&self) -> String {
            format!("{:?}", self.inner.state)
        }

        /// Get priority as string
        #[getter]
        fn priority(&self) -> String {
            format!("{:?}", self.inner.priority)
        }

        /// Grant a capability by name
        fn grant_capability(&mut self, cap_name: &str) -> PyResult<()> {
            let cap = parse_capability(cap_name)?;
            self.inner.capabilities.grant(cap);
            Ok(())
        }

        /// Revoke a capability
        fn revoke_capability(&mut self, cap_name: &str) -> PyResult<()> {
            let cap = parse_capability(cap_name)?;
            self.inner.capabilities.revoke(cap);
            Ok(())
        }

        /// Check if agent has a capability
        fn has_capability(&self, cap_name: &str) -> PyResult<bool> {
            let cap = parse_capability(cap_name)?;
            Ok(self.inner.capabilities.has(cap))
        }

        /// List all capabilities
        fn capabilities(&self) -> Vec<String> {
            self.inner
                .capabilities
                .list()
                .into_iter()
                .map(|c| format!("{:?}", c))
                .collect()
        }

        /// Stats as dict
        fn stats(&self) -> PyResult<PyObject> {
            Python::with_gil(|py| {
                let dict = PyDict::new(py);
                dict.set_item("cpu_time_ms", self.inner.stats.total_cpu_time.as_millis())?;
                dict.set_item("schedule_count", self.inner.stats.schedule_count)?;
                dict.set_item("syscall_count", self.inner.stats.syscall_count)?;
                dict.set_item("memory_allocated", self.inner.stats.memory_allocated)?;
                dict.set_item("vruntime", self.inner.vruntime)?;
                Ok(dict.into())
            })
        }

        fn __repr__(&self) -> String {
            format!(
                "Agent(name='{}', id='{}', state={:?}, priority={:?})",
                self.inner.name,
                self.inner.id.short(),
                self.inner.state,
                self.inner.priority,
            )
        }
    }

    /// Python-exposed Scheduler
    #[pyclass(name = "Scheduler")]
    struct PyScheduler {
        inner: RustScheduler,
    }

    #[pymethods]
    impl PyScheduler {
        #[new]
        #[pyo3(signature = (max_agents=1024, quantum_ms=10))]
        fn new(max_agents: usize, quantum_ms: u64) -> Self {
            let config = SchedulerConfig {
                max_agents,
                quantum: std::time::Duration::from_millis(quantum_ms),
                ..Default::default()
            };
            PyScheduler {
                inner: RustScheduler::new(config),
            }
        }

        /// Admit an agent
        fn admit(&mut self, agent: &mut PyAgent) -> PyResult<String> {
            // Clone agent for scheduler (takes ownership)
            let a = Agent::with_config(
                agent.inner.name.clone(),
                agent.inner.priority,
                agent.inner.capabilities.clone(),
                agent.inner.quota.clone(),
            );
            let id = self
                .inner
                .admit(a)
                .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;
            Ok(id.to_string())
        }

        /// Pick next agent to run
        fn pick_next(&mut self) -> Option<String> {
            self.inner.pick_next().map(|id| id.to_string())
        }

        /// Number of ready agents
        #[getter]
        fn ready_count(&self) -> usize {
            self.inner.ready_count()
        }

        /// Total agents
        #[getter]
        fn total_agents(&self) -> usize {
            self.inner.total_agents()
        }

        /// Stats as dict
        fn stats(&self) -> PyResult<PyObject> {
            Python::with_gil(|py| {
                let s = self.inner.stats();
                let dict = PyDict::new(py);
                dict.set_item("total_schedules", s.total_schedules)?;
                dict.set_item("context_switches", s.context_switches)?;
                dict.set_item("preemptions", s.preemptions)?;
                dict.set_item("ready_count", s.ready_count)?;
                dict.set_item("blocked_count", s.blocked_count)?;
                dict.set_item("running_count", s.running_count)?;
                Ok(dict.into())
            })
        }

        fn __repr__(&self) -> String {
            format!(
                "Scheduler(agents={}, ready={}, max={})",
                self.inner.total_agents(),
                self.inner.ready_count(),
                self.inner.config.max_agents,
            )
        }
    }

    /// Python-exposed VectorStore
    #[pyclass(name = "VectorStore")]
    struct PyVectorStore {
        inner: RustVectorStore,
    }

    #[pymethods]
    impl PyVectorStore {
        #[new]
        fn new(dim: usize) -> Self {
            PyVectorStore {
                inner: RustVectorStore::new(dim),
            }
        }

        /// Create a cluster
        fn create_cluster(&mut self, name: &str, parent_id: Option<u64>) -> PyResult<u64> {
            self.inner
                .create_cluster(
                    name.to_string(),
                    parent_id.unwrap_or(0),
                    AgentId::from_bytes([0; 16]),
                )
                .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))
        }

        /// Add a vector
        fn add(&mut self, name: &str, vector: Vec<f32>, cluster_id: Option<u64>) -> PyResult<u64> {
            self.inner
                .add_vector(
                    name.to_string(),
                    cluster_id.unwrap_or(0),
                    vector,
                    AgentId::from_bytes([0; 16]),
                )
                .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))
        }

        /// Search for similar vectors
        fn search(&self, query: Vec<f32>, top_k: usize) -> Vec<(String, f32)> {
            self.inner
                .search(&query, top_k)
                .into_iter()
                .map(|r| (r.file_name, r.similarity))
                .collect()
        }

        /// Delete a vector
        fn delete(&mut self, id: u64) -> PyResult<()> {
            self.inner
                .delete_vector(id)
                .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))
        }

        /// Stats
        fn stats(&self) -> PyResult<PyObject> {
            Python::with_gil(|py| {
                let s = self.inner.stats();
                let dict = PyDict::new(py);
                dict.set_item("total_vectors", s.total_vectors)?;
                dict.set_item("total_clusters", s.total_clusters)?;
                dict.set_item("dimension", s.dimension)?;
                dict.set_item("total_bytes", s.total_bytes)?;
                Ok(dict.into())
            })
        }

        fn __repr__(&self) -> String {
            let s = self.inner.stats();
            format!(
                "VectorStore(dim={}, vectors={}, clusters={})",
                s.dimension, s.total_vectors, s.total_clusters,
            )
        }
    }

    /// Helper: parse capability name string
    fn parse_capability(name: &str) -> PyResult<Capability> {
        match name.to_lowercase().as_str() {
            "execute" => Ok(Capability::Execute),
            "spawn" => Ok(Capability::Spawn),
            "fork" => Ok(Capability::Fork),
            "mem_alloc" => Ok(Capability::MemAlloc),
            "mem_map" => Ok(Capability::MemMap),
            "mem_read" => Ok(Capability::MemRead),
            "mem_write" => Ok(Capability::MemWrite),
            "fs_read" => Ok(Capability::FsRead),
            "fs_write" => Ok(Capability::FsWrite),
            "fs_create" => Ok(Capability::FsCreate),
            "fs_delete" => Ok(Capability::FsDelete),
            "fs_search" => Ok(Capability::FsSearch),
            "net_connect" => Ok(Capability::NetConnect),
            "net_listen" => Ok(Capability::NetListen),
            "net_send" => Ok(Capability::NetSend),
            "net_recv" => Ok(Capability::NetRecv),
            "net_peer" => Ok(Capability::NetPeer),
            "ipc_send" => Ok(Capability::IpcSend),
            "ipc_recv" => Ok(Capability::IpcRecv),
            "ipc_create" => Ok(Capability::IpcCreate),
            "crypto_encrypt" => Ok(Capability::CryptoEncrypt),
            "crypto_decrypt" => Ok(Capability::CryptoDecrypt),
            "crypto_sign" => Ok(Capability::CryptoSign),
            "crypto_verify" => Ok(Capability::CryptoVerify),
            "crypto_key_access" => Ok(Capability::CryptoKeyAccess),
            "gpu_compute" => Ok(Capability::GpuCompute),
            "gpu_alloc" => Ok(Capability::GpuAlloc),
            "agent_inspect" => Ok(Capability::AgentInspect),
            "agent_kill" => Ok(Capability::AgentKill),
            "cap_delegate" => Ok(Capability::CapDelegate),
            "kernel_access" => Ok(Capability::KernelAccess),
            _ => Err(pyo3::exceptions::PyValueError::new_err(
                format!("Unknown capability: {name}"),
            )),
        }
    }

    /// Python module
    #[pymodule]
    fn neural_kernel(m: &Bound<'_, PyModule>) -> PyResult<()> {
        m.add_class::<PyAgent>()?;
        m.add_class::<PyScheduler>()?;
        m.add_class::<PyVectorStore>()?;
        m.add("__version__", env!("CARGO_PKG_VERSION"))?;
        Ok(())
    }
}
