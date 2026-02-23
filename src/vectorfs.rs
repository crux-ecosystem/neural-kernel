//! VectorFS — A filesystem where files are vectors
//!
//! VectorFS is a FUSE-based filesystem that treats:
//! - **Files** as vectors (embeddings) with metadata
//! - **Directories** as clusters (groups of similar vectors)
//! - **Search** as the primary access pattern (not just path-based)
//!
//! Mount point structure:
//! ```text
//! /vectorfs/
//! ├── .meta/                    # System metadata
//! │   ├── config.json
//! │   └── stats.json
//! ├── documents/                # Cluster: document embeddings
//! │   ├── doc_001.vec           # Vector file (768-dim f32)
//! │   ├── doc_002.vec
//! │   └── .cluster_meta.json    # Cluster centroid + metadata
//! ├── queries/                  # Cluster: query embeddings
//! │   └── ...
//! └── agents/                   # Per-agent vector spaces
//!     ├── agent_abc/
//!     └── agent_xyz/
//! ```
//!
//! Special operations:
//! - `cat file.vec` → prints vector as JSON array
//! - `echo "[0.1, 0.2, ...]" > file.vec` → creates vector from JSON
//! - `ls -la dir/` → shows vectors ranked by cluster similarity
//! - `find /vectorfs -similar file.vec -k 10` → similarity search (via xattr)
//!
//! The actual FUSE implementation requires the `fuser` crate.
//! This module provides the data structures and logic.

use std::collections::HashMap;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::agent::AgentId;

/// A vector file in VectorFS
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VectorFile {
    /// File ID (inode equivalent)
    pub id: u64,
    /// File name
    pub name: String,
    /// Parent cluster ID
    pub cluster_id: u64,
    /// The vector data (f32 array)
    pub vector: Vec<f32>,
    /// Dimensionality
    pub dim: usize,
    /// Metadata (key-value pairs)
    pub metadata: HashMap<String, String>,
    /// Owner agent
    pub owner: AgentId,
    /// Creation time
    pub created_at: SystemTime,
    /// Last modification time
    pub modified_at: SystemTime,
    /// L2 norm of the vector (cached)
    pub norm: f32,
}

impl VectorFile {
    /// Create a new vector file
    pub fn new(
        id: u64,
        name: String,
        cluster_id: u64,
        vector: Vec<f32>,
        owner: AgentId,
    ) -> Self {
        let dim = vector.len();
        let norm = l2_norm(&vector);
        let now = SystemTime::now();

        Self {
            id,
            name,
            cluster_id,
            vector,
            dim,
            metadata: HashMap::new(),
            owner,
            created_at: now,
            modified_at: now,
            norm,
        }
    }

    /// Cosine similarity with another vector
    pub fn cosine_similarity(&self, other: &[f32]) -> f32 {
        if self.vector.len() != other.len() {
            return 0.0;
        }
        let dot: f32 = self
            .vector
            .iter()
            .zip(other.iter())
            .map(|(a, b)| a * b)
            .sum();
        let other_norm = l2_norm(other);
        if self.norm == 0.0 || other_norm == 0.0 {
            return 0.0;
        }
        dot / (self.norm * other_norm)
    }

    /// Euclidean distance to another vector
    pub fn euclidean_distance(&self, other: &[f32]) -> f32 {
        if self.vector.len() != other.len() {
            return f32::MAX;
        }
        self.vector
            .iter()
            .zip(other.iter())
            .map(|(a, b)| (a - b).powi(2))
            .sum::<f32>()
            .sqrt()
    }

    /// File size in bytes (vector data + metadata estimate)
    pub fn size_bytes(&self) -> usize {
        self.vector.len() * std::mem::size_of::<f32>()
            + self
                .metadata
                .iter()
                .map(|(k, v)| k.len() + v.len())
                .sum::<usize>()
    }

    /// Serialize vector to JSON (for `cat file.vec`)
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(&VectorFileView {
            name: &self.name,
            dim: self.dim,
            norm: self.norm,
            vector: &self.vector,
            metadata: &self.metadata,
        })
        .unwrap_or_default()
    }
}

#[derive(Serialize)]
struct VectorFileView<'a> {
    name: &'a str,
    dim: usize,
    norm: f32,
    vector: &'a [f32],
    metadata: &'a HashMap<String, String>,
}

/// A cluster (directory) in VectorFS
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cluster {
    /// Cluster ID
    pub id: u64,
    /// Directory name
    pub name: String,
    /// Parent cluster (None for root)
    pub parent_id: Option<u64>,
    /// Centroid vector (average of all vectors in cluster)
    pub centroid: Vec<f32>,
    /// Vector IDs in this cluster
    pub vector_ids: Vec<u64>,
    /// Sub-cluster IDs
    pub sub_cluster_ids: Vec<u64>,
    /// Metadata
    pub metadata: HashMap<String, String>,
    /// Owner
    pub owner: AgentId,
    /// Creation time
    pub created_at: SystemTime,
}

impl Cluster {
    /// Create a new cluster
    pub fn new(id: u64, name: String, parent_id: Option<u64>, dim: usize, owner: AgentId) -> Self {
        Self {
            id,
            name,
            parent_id,
            centroid: vec![0.0; dim],
            vector_ids: Vec::new(),
            sub_cluster_ids: Vec::new(),
            metadata: HashMap::new(),
            owner,
            created_at: SystemTime::now(),
        }
    }

    /// Number of vectors in this cluster
    pub fn count(&self) -> usize {
        self.vector_ids.len()
    }

    /// Recompute centroid from vectors
    pub fn recompute_centroid(&mut self, vectors: &[&VectorFile]) {
        if vectors.is_empty() {
            return;
        }
        let dim = vectors[0].dim;
        let mut centroid = vec![0.0f32; dim];
        for v in vectors {
            for (i, val) in v.vector.iter().enumerate() {
                centroid[i] += val;
            }
        }
        let n = vectors.len() as f32;
        for val in &mut centroid {
            *val /= n;
        }
        self.centroid = centroid;
    }
}

/// Search result from VectorFS
#[derive(Debug, Clone, Serialize)]
pub struct SearchResult {
    pub file_id: u64,
    pub file_name: String,
    pub cluster_name: String,
    pub similarity: f32,
    pub distance: f32,
}

/// The VectorFS store (in-memory, backs the FUSE layer)
pub struct VectorStore {
    /// All vectors by ID
    vectors: HashMap<u64, VectorFile>,
    /// All clusters by ID
    clusters: HashMap<u64, Cluster>,
    /// Root cluster ID
    #[allow(dead_code)]
    root_cluster_id: u64,
    /// Dimension
    dim: usize,
    /// Next ID counter
    next_id: u64,
    /// Total vectors
    total_vectors: usize,
}

impl VectorStore {
    /// Create a new VectorFS store
    pub fn new(dim: usize) -> Self {
        let root = Cluster::new(0, "root".into(), None, dim, AgentId::from_bytes([0; 16]));
        let mut clusters = HashMap::new();
        clusters.insert(0, root);

        Self {
            vectors: HashMap::new(),
            clusters,
            root_cluster_id: 0,
            dim,
            next_id: 1,
            total_vectors: 0,
        }
    }

    /// Create a new cluster
    pub fn create_cluster(
        &mut self,
        name: String,
        parent_id: u64,
        owner: AgentId,
    ) -> Result<u64, VectorFsError> {
        if !self.clusters.contains_key(&parent_id) {
            return Err(VectorFsError::ClusterNotFound(parent_id));
        }

        let id = self.next_id;
        self.next_id += 1;

        let cluster = Cluster::new(id, name, Some(parent_id), self.dim, owner);
        self.clusters.insert(id, cluster);

        // Add to parent
        if let Some(parent) = self.clusters.get_mut(&parent_id) {
            parent.sub_cluster_ids.push(id);
        }

        Ok(id)
    }

    /// Add a vector file
    pub fn add_vector(
        &mut self,
        name: String,
        cluster_id: u64,
        vector: Vec<f32>,
        owner: AgentId,
    ) -> Result<u64, VectorFsError> {
        if vector.len() != self.dim {
            return Err(VectorFsError::DimensionMismatch {
                expected: self.dim,
                got: vector.len(),
            });
        }
        if !self.clusters.contains_key(&cluster_id) {
            return Err(VectorFsError::ClusterNotFound(cluster_id));
        }

        let id = self.next_id;
        self.next_id += 1;

        let file = VectorFile::new(id, name, cluster_id, vector, owner);
        self.vectors.insert(id, file);

        if let Some(cluster) = self.clusters.get_mut(&cluster_id) {
            cluster.vector_ids.push(id);
        }

        self.total_vectors += 1;
        Ok(id)
    }

    /// Get a vector file
    pub fn get_vector(&self, id: u64) -> Option<&VectorFile> {
        self.vectors.get(&id)
    }

    /// Delete a vector file
    pub fn delete_vector(&mut self, id: u64) -> Result<(), VectorFsError> {
        let file = self
            .vectors
            .remove(&id)
            .ok_or(VectorFsError::VectorNotFound(id))?;

        if let Some(cluster) = self.clusters.get_mut(&file.cluster_id) {
            cluster.vector_ids.retain(|&v| v != id);
        }

        self.total_vectors -= 1;
        Ok(())
    }

    /// Similarity search across all vectors
    pub fn search(&self, query: &[f32], top_k: usize) -> Vec<SearchResult> {
        if query.len() != self.dim {
            return vec![];
        }

        let mut results: Vec<SearchResult> = self
            .vectors
            .values()
            .map(|v| {
                let similarity = v.cosine_similarity(query);
                let distance = v.euclidean_distance(query);
                let cluster_name = self
                    .clusters
                    .get(&v.cluster_id)
                    .map(|c| c.name.clone())
                    .unwrap_or_default();

                SearchResult {
                    file_id: v.id,
                    file_name: v.name.clone(),
                    cluster_name,
                    similarity,
                    distance,
                }
            })
            .collect();

        // Sort by similarity (descending)
        results.sort_by(|a, b| b.similarity.partial_cmp(&a.similarity).unwrap());
        results.truncate(top_k);
        results
    }

    /// Search within a specific cluster
    pub fn search_cluster(
        &self,
        cluster_id: u64,
        query: &[f32],
        top_k: usize,
    ) -> Vec<SearchResult> {
        let cluster = match self.clusters.get(&cluster_id) {
            Some(c) => c,
            None => return vec![],
        };

        let mut results: Vec<SearchResult> = cluster
            .vector_ids
            .iter()
            .filter_map(|vid| self.vectors.get(vid))
            .map(|v| SearchResult {
                file_id: v.id,
                file_name: v.name.clone(),
                cluster_name: cluster.name.clone(),
                similarity: v.cosine_similarity(query),
                distance: v.euclidean_distance(query),
            })
            .collect();

        results.sort_by(|a, b| b.similarity.partial_cmp(&a.similarity).unwrap());
        results.truncate(top_k);
        results
    }

    /// List entries in a cluster
    pub fn list_cluster(&self, cluster_id: u64) -> Result<ClusterListing, VectorFsError> {
        let cluster = self
            .clusters
            .get(&cluster_id)
            .ok_or(VectorFsError::ClusterNotFound(cluster_id))?;

        Ok(ClusterListing {
            cluster_name: cluster.name.clone(),
            vectors: cluster
                .vector_ids
                .iter()
                .filter_map(|id| self.vectors.get(id))
                .map(|v| VectorEntry {
                    id: v.id,
                    name: v.name.clone(),
                    dim: v.dim,
                    norm: v.norm,
                    size_bytes: v.size_bytes(),
                })
                .collect(),
            sub_clusters: cluster
                .sub_cluster_ids
                .iter()
                .filter_map(|id| self.clusters.get(id))
                .map(|c| ClusterEntry {
                    id: c.id,
                    name: c.name.clone(),
                    vector_count: c.vector_ids.len(),
                })
                .collect(),
        })
    }

    /// Stats
    pub fn stats(&self) -> VectorFsStats {
        VectorFsStats {
            total_vectors: self.total_vectors,
            total_clusters: self.clusters.len(),
            dimension: self.dim,
            total_bytes: self.vectors.values().map(|v| v.size_bytes()).sum(),
        }
    }
}

/// Cluster listing (ls output)
#[derive(Debug, Serialize)]
pub struct ClusterListing {
    pub cluster_name: String,
    pub vectors: Vec<VectorEntry>,
    pub sub_clusters: Vec<ClusterEntry>,
}

#[derive(Debug, Serialize)]
pub struct VectorEntry {
    pub id: u64,
    pub name: String,
    pub dim: usize,
    pub norm: f32,
    pub size_bytes: usize,
}

#[derive(Debug, Serialize)]
pub struct ClusterEntry {
    pub id: u64,
    pub name: String,
    pub vector_count: usize,
}

#[derive(Debug, Serialize)]
pub struct VectorFsStats {
    pub total_vectors: usize,
    pub total_clusters: usize,
    pub dimension: usize,
    pub total_bytes: usize,
}

/// VectorFS errors
#[derive(Debug, thiserror::Error)]
pub enum VectorFsError {
    #[error("Cluster not found: {0}")]
    ClusterNotFound(u64),

    #[error("Vector not found: {0}")]
    VectorNotFound(u64),

    #[error("Dimension mismatch: expected {expected}, got {got}")]
    DimensionMismatch { expected: usize, got: usize },

    #[error("Permission denied")]
    PermissionDenied,
}

/// Compute L2 norm
fn l2_norm(v: &[f32]) -> f32 {
    v.iter().map(|x| x * x).sum::<f32>().sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_and_search() {
        let owner = AgentId::new();
        let mut store = VectorStore::new(3);

        let c1 = store.create_cluster("docs".into(), 0, owner).unwrap();

        store
            .add_vector("v1.vec".into(), c1, vec![1.0, 0.0, 0.0], owner)
            .unwrap();
        store
            .add_vector("v2.vec".into(), c1, vec![0.0, 1.0, 0.0], owner)
            .unwrap();
        store
            .add_vector("v3.vec".into(), c1, vec![0.9, 0.1, 0.0], owner)
            .unwrap();

        let results = store.search(&[1.0, 0.0, 0.0], 2);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].file_name, "v1.vec"); // Most similar
    }

    #[test]
    fn test_cosine_similarity() {
        let owner = AgentId::new();
        let f = VectorFile::new(0, "test".into(), 0, vec![1.0, 0.0, 0.0], owner);
        assert!((f.cosine_similarity(&[1.0, 0.0, 0.0]) - 1.0).abs() < 1e-6);
        assert!((f.cosine_similarity(&[0.0, 1.0, 0.0])).abs() < 1e-6);
    }

    #[test]
    fn test_dimension_mismatch() {
        let owner = AgentId::new();
        let mut store = VectorStore::new(3);
        let result = store.add_vector("bad.vec".into(), 0, vec![1.0, 2.0], owner);
        assert!(result.is_err());
    }

    #[test]
    fn test_list_cluster() {
        let owner = AgentId::new();
        let mut store = VectorStore::new(4);

        let c1 = store.create_cluster("cluster1".into(), 0, owner).unwrap();
        store
            .add_vector("a.vec".into(), c1, vec![1.0, 0.0, 0.0, 0.0], owner)
            .unwrap();
        store
            .add_vector("b.vec".into(), c1, vec![0.0, 1.0, 0.0, 0.0], owner)
            .unwrap();

        let listing = store.list_cluster(c1).unwrap();
        assert_eq!(listing.vectors.len(), 2);
    }

    #[test]
    fn test_delete_vector() {
        let owner = AgentId::new();
        let mut store = VectorStore::new(3);
        let id = store
            .add_vector("del.vec".into(), 0, vec![1.0, 0.0, 0.0], owner)
            .unwrap();

        assert_eq!(store.stats().total_vectors, 1);
        store.delete_vector(id).unwrap();
        assert_eq!(store.stats().total_vectors, 0);
    }
}
