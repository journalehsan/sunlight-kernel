//! Dependency graph and topological sort using Kahn's algorithm
//! Fixed-capacity, no heap allocations

use heapless::Vec;
use crate::unit::{UnitName, MAX_UNITS};

#[derive(Debug)]
pub enum GraphError {
    UnitNotFound,
    TooManyUnits,
    Cycle,
}

pub struct DepGraph {
    units: [Option<UnitName>; MAX_UNITS],
    edges: [[bool; MAX_UNITS]; MAX_UNITS],  // edges[a][b] = a must start before b
    count: usize,
}

impl DepGraph {
    pub fn new() -> Self {
        Self {
            units: [const { None }; MAX_UNITS],
            edges: [[false; MAX_UNITS]; MAX_UNITS],
            count: 0,
        }
    }

    /// Add a unit to the graph, returns its index
    pub fn add_unit(&mut self, name: &UnitName) -> Result<usize, GraphError> {
        // Check if already exists
        for i in 0..self.count {
            if let Some(ref u) = self.units[i] {
                if u == name {
                    return Ok(i);
                }
            }
        }

        if self.count >= MAX_UNITS {
            return Err(GraphError::TooManyUnits);
        }

        let idx = self.count;
        self.units[idx] = Some(name.clone());
        self.count += 1;
        Ok(idx)
    }

    /// Add an edge: before must start before after
    pub fn add_edge(&mut self, before: &UnitName, after: &UnitName) -> Result<(), GraphError> {
        let before_idx = self.find_unit(before).ok_or(GraphError::UnitNotFound)?;
        let after_idx = self.find_unit(after).ok_or(GraphError::UnitNotFound)?;
        self.edges[before_idx][after_idx] = true;
        Ok(())
    }

    fn find_unit(&self, name: &UnitName) -> Option<usize> {
        for i in 0..self.count {
            if let Some(ref u) = self.units[i] {
                if u == name {
                    return Some(i);
                }
            }
        }
        None
    }

    /// Returns unit indices in startup order using topological sort (Kahn's algorithm)
    pub fn topological_order(&self) -> Result<Vec<usize, MAX_UNITS>, GraphError> {
        let mut in_degree = [0u32; MAX_UNITS];
        
        // Calculate in-degrees
        for i in 0..self.count {
            for j in 0..self.count {
                if self.edges[i][j] {
                    in_degree[j] += 1;
                }
            }
        }

        // Queue of nodes with in-degree 0
        let mut queue: Vec<usize, MAX_UNITS> = Vec::new();
        for i in 0..self.count {
            if in_degree[i] == 0 {
                queue.push(i).map_err(|_| GraphError::TooManyUnits)?;
            }
        }

        let mut result: Vec<usize, MAX_UNITS> = Vec::new();

        while let Some(node) = queue.pop() {
            result.push(node).map_err(|_| GraphError::TooManyUnits)?;

            // Reduce in-degree of neighbors
            for neighbor in 0..self.count {
                if self.edges[node][neighbor] {
                    in_degree[neighbor] -= 1;
                    if in_degree[neighbor] == 0 {
                        queue.push(neighbor).map_err(|_| GraphError::TooManyUnits)?;
                    }
                }
            }
        }

        // If we didn't process all nodes, there's a cycle
        if result.len() != self.count {
            return Err(GraphError::Cycle);
        }

        Ok(result)
    }

    pub fn get_unit_name(&self, idx: usize) -> Option<&UnitName> {
        if idx < self.count {
            self.units[idx].as_ref()
        } else {
            None
        }
    }
}
