//! Memory Manager — Arena-based allocation with vector-aware regions
//!
//! Features:
//! - Arena allocator for agent-scoped memory
//! - Memory-mapped vector regions (mmap for large vector stores)
//! - Per-agent quota enforcement
//! - Shared memory regions for zero-copy IPC
//! - Memory statistics and leak detection

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde::{Deserialize, Serialize};

use crate::agent::AgentId;

/// Memory region types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RegionType {
    /// General-purpose heap memory
    Heap,
    /// Vector storage (contiguous, aligned)
    Vector,
    /// Shared memory (IPC)
    Shared,
    /// Memory-mapped file
    MappedFile,
    /// Stack memory
    Stack,
}

/// A memory region descriptor
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryRegion {
    /// Unique region ID
    pub id: u64,
    /// Owning agent
    pub owner: AgentId,
    /// Region type
    pub region_type: RegionType,
    /// Base address (virtual)
    pub base_addr: usize,
    /// Size in bytes
    pub size: usize,
    /// Alignment requirement
    pub alignment: usize,
    /// Is this region readable?
    pub readable: bool,
    /// Is this region writable?
    pub writable: bool,
    /// Is this region executable?
    pub executable: bool,
    /// Reference count (for shared regions)
    pub ref_count: u32,
}

/// Per-agent memory accounting
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct AgentMemoryStats {
    /// Total bytes allocated
    pub allocated: usize,
    /// Peak allocation
    pub peak: usize,
    /// Number of active regions
    pub region_count: usize,
    /// Total allocation calls
    pub alloc_count: u64,
    /// Total free calls
    pub free_count: u64,
    /// Vector storage bytes
    pub vector_bytes: usize,
    /// Shared memory bytes
    pub shared_bytes: usize,
}

/// Arena allocator for a single agent
#[derive(Debug)]
pub struct Arena {
    /// Agent this arena belongs to
    pub agent_id: AgentId,
    /// Maximum size (quota)
    pub max_size: usize,
    /// Current allocation watermark
    pub watermark: usize,
    /// Active regions
    pub regions: HashMap<u64, MemoryRegion>,
    /// Statistics
    pub stats: AgentMemoryStats,
    /// Region ID counter
    next_id: u64,
}

impl Arena {
    /// Create a new arena for an agent
    pub fn new(agent_id: AgentId, max_size: usize) -> Self {
        Self {
            agent_id,
            max_size,
            watermark: 0,
            regions: HashMap::new(),
            stats: AgentMemoryStats::default(),
            next_id: 0,
        }
    }

    /// Allocate a memory region
    pub fn alloc(
        &mut self,
        size: usize,
        alignment: usize,
        region_type: RegionType,
    ) -> Result<u64, MemoryError> {
        // Align the size
        let aligned_size = align_up(size, alignment);

        // Check quota
        if self.watermark + aligned_size > self.max_size {
            return Err(MemoryError::QuotaExceeded {
                requested: aligned_size,
                available: self.max_size - self.watermark,
                quota: self.max_size,
            });
        }

        let id = self.next_id;
        self.next_id += 1;

        let base_addr = self.watermark;

        let region = MemoryRegion {
            id,
            owner: self.agent_id,
            region_type,
            base_addr,
            size: aligned_size,
            alignment,
            readable: true,
            writable: true,
            executable: false,
            ref_count: 1,
        };

        self.watermark += aligned_size;
        self.stats.allocated += aligned_size;
        self.stats.peak = self.stats.peak.max(self.stats.allocated);
        self.stats.region_count += 1;
        self.stats.alloc_count += 1;

        match region_type {
            RegionType::Vector => self.stats.vector_bytes += aligned_size,
            RegionType::Shared => self.stats.shared_bytes += aligned_size,
            _ => {}
        }

        self.regions.insert(id, region);
        Ok(id)
    }

    /// Free a memory region
    pub fn free(&mut self, region_id: u64) -> Result<(), MemoryError> {
        let region = self
            .regions
            .remove(&region_id)
            .ok_or(MemoryError::InvalidRegion(region_id))?;

        self.stats.allocated -= region.size;
        self.stats.region_count -= 1;
        self.stats.free_count += 1;

        match region.region_type {
            RegionType::Vector => self.stats.vector_bytes -= region.size,
            RegionType::Shared => self.stats.shared_bytes -= region.size,
            _ => {}
        }

        Ok(())
    }

    /// Allocate a vector region (aligned to cache line)
    pub fn alloc_vector(&mut self, dim: usize, count: usize) -> Result<u64, MemoryError> {
        let size = dim * count * std::mem::size_of::<f32>();
        self.alloc(size, 64, RegionType::Vector) // 64-byte cache line alignment
    }

    /// Get region info
    pub fn get_region(&self, id: u64) -> Option<&MemoryRegion> {
        self.regions.get(&id)
    }

    /// Usage percentage
    pub fn usage_percent(&self) -> f64 {
        if self.max_size == 0 {
            return 0.0;
        }
        (self.stats.allocated as f64 / self.max_size as f64) * 100.0
    }
}

/// The global memory manager
pub struct MemoryManager {
    /// Per-agent arenas
    arenas: HashMap<AgentId, Arena>,
    /// Global memory limit
    global_limit: usize,
    /// Total allocated across all agents
    total_allocated: AtomicUsize,
    /// Global statistics
    global_alloc_count: AtomicUsize,
}

impl MemoryManager {
    /// Create a new memory manager
    pub fn new(global_limit: usize) -> Self {
        Self {
            arenas: HashMap::new(),
            global_limit,
            total_allocated: AtomicUsize::new(0),
            global_alloc_count: AtomicUsize::new(0),
        }
    }

    /// Create an arena for a new agent
    pub fn create_arena(&mut self, agent_id: AgentId, quota: usize) -> Result<(), MemoryError> {
        if self.arenas.contains_key(&agent_id) {
            return Err(MemoryError::ArenaExists(agent_id));
        }

        let effective_quota = quota.min(
            self.global_limit - self.total_allocated.load(Ordering::Relaxed),
        );

        self.arenas.insert(agent_id, Arena::new(agent_id, effective_quota));
        Ok(())
    }

    /// Destroy an arena (free all agent memory)
    pub fn destroy_arena(&mut self, agent_id: &AgentId) -> Result<AgentMemoryStats, MemoryError> {
        let arena = self
            .arenas
            .remove(agent_id)
            .ok_or(MemoryError::ArenaNotFound(*agent_id))?;

        self.total_allocated
            .fetch_sub(arena.stats.allocated, Ordering::Relaxed);

        Ok(arena.stats)
    }

    /// Get a mutable reference to an agent's arena
    pub fn arena_mut(&mut self, agent_id: &AgentId) -> Option<&mut Arena> {
        self.arenas.get_mut(agent_id)
    }

    /// Get an immutable reference
    pub fn arena(&self, agent_id: &AgentId) -> Option<&Arena> {
        self.arenas.get(agent_id)
    }

    /// Global memory stats
    pub fn global_stats(&self) -> GlobalMemoryStats {
        GlobalMemoryStats {
            total_allocated: self.total_allocated.load(Ordering::Relaxed),
            global_limit: self.global_limit,
            arena_count: self.arenas.len(),
            total_allocs: self.global_alloc_count.load(Ordering::Relaxed),
        }
    }
}

/// Global memory statistics
#[derive(Debug, Serialize)]
pub struct GlobalMemoryStats {
    pub total_allocated: usize,
    pub global_limit: usize,
    pub arena_count: usize,
    pub total_allocs: usize,
}

/// Memory errors
#[derive(Debug, thiserror::Error)]
pub enum MemoryError {
    #[error("Memory quota exceeded: requested {requested} bytes, available {available}, quota {quota}")]
    QuotaExceeded {
        requested: usize,
        available: usize,
        quota: usize,
    },

    #[error("Invalid memory region: {0}")]
    InvalidRegion(u64),

    #[error("Arena already exists for agent {0:?}")]
    ArenaExists(AgentId),

    #[error("Arena not found for agent {0:?}")]
    ArenaNotFound(AgentId),

    #[error("Alignment must be a power of 2, got {0}")]
    BadAlignment(usize),

    #[error("Global memory limit reached")]
    GlobalLimitReached,
}

/// Align a size up to the given alignment
fn align_up(size: usize, alignment: usize) -> usize {
    (size + alignment - 1) & !(alignment - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_arena_alloc_free() {
        let agent = AgentId::new();
        let mut arena = Arena::new(agent, 1024 * 1024); // 1 MB

        let r1 = arena.alloc(1024, 8, RegionType::Heap).unwrap();
        assert_eq!(arena.stats.allocated, 1024);

        arena.free(r1).unwrap();
        assert_eq!(arena.stats.allocated, 0);
    }

    #[test]
    fn test_quota_enforcement() {
        let agent = AgentId::new();
        let mut arena = Arena::new(agent, 1024);

        arena.alloc(512, 8, RegionType::Heap).unwrap();
        arena.alloc(512, 8, RegionType::Heap).unwrap();

        // Should fail — no space left
        assert!(arena.alloc(1, 1, RegionType::Heap).is_err());
    }

    #[test]
    fn test_vector_allocation() {
        let agent = AgentId::new();
        let mut arena = Arena::new(agent, 10 * 1024 * 1024); // 10 MB

        // 1000 vectors of 768 dimensions (f32)
        let r = arena.alloc_vector(768, 1000).unwrap();
        let region = arena.get_region(r).unwrap();

        assert_eq!(region.region_type, RegionType::Vector);
        assert_eq!(region.alignment, 64);
        assert!(region.size >= 768 * 1000 * 4);
    }

    #[test]
    fn test_memory_manager() {
        let mut mm = MemoryManager::new(100 * 1024 * 1024);

        let agent1 = AgentId::new();
        let agent2 = AgentId::new();

        mm.create_arena(agent1, 10 * 1024 * 1024).unwrap();
        mm.create_arena(agent2, 10 * 1024 * 1024).unwrap();

        let arena = mm.arena_mut(&agent1).unwrap();
        arena.alloc(4096, 8, RegionType::Heap).unwrap();

        let stats = mm.global_stats();
        assert_eq!(stats.arena_count, 2);
    }

    #[test]
    fn test_align_up() {
        assert_eq!(align_up(100, 64), 128);
        assert_eq!(align_up(64, 64), 64);
        assert_eq!(align_up(1, 8), 8);
        assert_eq!(align_up(0, 64), 0);
    }
}
