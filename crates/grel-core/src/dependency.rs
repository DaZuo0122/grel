//! Lightweight dependency resolution for grel packages.
//!
//! Supports topological sorting and cycle detection for DAGs.
//! No SAT solver — simple graph traversal is sufficient for
//! pre-built binary packages without version constraints.

use std::collections::{HashMap, HashSet, VecDeque};

use crate::package_ref::PackageRef;

/// A directed dependency graph.
#[derive(Debug, Clone, Default)]
pub struct DependencyGraph {
    /// Maps each package to the set of packages it directly depends on.
    edges: HashMap<PackageRef, HashSet<PackageRef>>,
}

impl DependencyGraph {
    /// Create an empty graph.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a directed edge: `from` depends on `to`.
    pub fn add_edge(&mut self, from: PackageRef, to: PackageRef) {
        self.edges.entry(from).or_default().insert(to.clone());
        // Ensure the target node exists in the graph even if it has no outgoing edges
        self.edges.entry(to).or_default();
    }

    /// Add a package with no dependencies.
    pub fn add_node(&mut self, pkg: PackageRef) {
        self.edges.entry(pkg).or_default();
    }

    /// Return a topological ordering of all packages in the graph.
    ///
    /// Packages with no dependencies come first. If a cycle is detected,
    /// returns an error listing the packages involved in the cycle.
    pub fn topological_sort(&self) -> Result<Vec<PackageRef>, DependencyError> {
        // Kahn's algorithm
        let mut in_degree: HashMap<PackageRef, usize> = HashMap::new();
        let mut adj: HashMap<PackageRef, Vec<PackageRef>> = HashMap::new();

        // Initialize in-degrees
        for (node, deps) in &self.edges {
            in_degree.entry(node.clone()).or_insert(0);
            for dep in deps {
                *in_degree.entry(dep.clone()).or_insert(0) += 1;
                adj.entry(node.clone()).or_default().push(dep.clone());
            }
        }

        let mut queue: VecDeque<PackageRef> = VecDeque::new();
        for (node, degree) in &in_degree {
            if *degree == 0 {
                queue.push_back(node.clone());
            }
        }

        let mut sorted = Vec::new();

        while let Some(node) = queue.pop_front() {
            sorted.push(node.clone());
            if let Some(neighbors) = adj.get(&node) {
                for neighbor in neighbors {
                    if let Some(degree) = in_degree.get_mut(neighbor) {
                        *degree -= 1;
                        if *degree == 0 {
                            queue.push_back(neighbor.clone());
                        }
                    }
                }
            }
        }

        if sorted.len() != in_degree.len() {
            // Cycle detected — find nodes not in sorted
            let sorted_set: HashSet<_> = sorted.iter().collect();
            let cycle_nodes: Vec<String> = in_degree
                .keys()
                .filter(|n| !sorted_set.contains(n))
                .map(|n| n.to_short_ref())
                .collect();
            return Err(DependencyError::CycleDetected(cycle_nodes.join(", ")));
        }

        // Reverse so that dependencies come before dependents
        sorted.reverse();
        Ok(sorted)
    }

    /// Find packages that are no longer required.
    ///
    /// A package is an orphan if:
    /// - It was installed implicitly (not explicitly)
    /// - No explicitly-installed package transitively depends on it
    pub fn find_orphans(
        &self,
        installed: &[PackageRef],
        explicit: &[PackageRef],
    ) -> Vec<PackageRef> {
        let explicit_set: HashSet<_> = explicit.iter().collect();
        let _installed_set: HashSet<_> = installed.iter().collect();

        // Compute transitive closure of what explicit packages need
        let mut needed: HashSet<PackageRef> = HashSet::new();
        for pkg in explicit {
            self.collect_transitive_deps(pkg, &mut needed);
        }

        // Also add explicit packages themselves as needed
        for pkg in explicit {
            needed.insert(pkg.clone());
        }

        installed
            .iter()
            .filter(|pkg| !needed.contains(pkg) && !explicit_set.contains(pkg))
            .cloned()
            .collect()
    }

    /// Collect all transitive dependencies of a package into `out`.
    fn collect_transitive_deps(&self, pkg: &PackageRef, out: &mut HashSet<PackageRef>) {
        if let Some(deps) = self.edges.get(pkg) {
            for dep in deps {
                if out.insert(dep.clone()) {
                    self.collect_transitive_deps(dep, out);
                }
            }
        }
    }

    /// Get the direct dependencies of a package.
    pub fn direct_deps(&self, pkg: &PackageRef) -> Vec<PackageRef> {
        self.edges
            .get(pkg)
            .map(|set| set.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Get all packages that directly depend on the given package.
    pub fn reverse_deps(&self, pkg: &PackageRef) -> Vec<PackageRef> {
        self.edges
            .iter()
            .filter(|(_, deps)| deps.contains(pkg))
            .map(|(from, _)| from.clone())
            .collect()
    }
}

/// Dependency resolution errors
#[derive(Debug, thiserror::Error)]
pub enum DependencyError {
    #[error("Dependency cycle detected involving: {0}")]
    CycleDetected(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pkg(forge: &str, owner: &str, repo: &str) -> PackageRef {
        use crate::Forge;
        PackageRef::new(
            forge.parse::<Forge>().unwrap(),
            owner.into(),
            repo.into(),
            None,
        )
    }

    #[test]
    fn test_topological_sort_simple() {
        let mut graph = DependencyGraph::new();
        let a = pkg("github", "owner", "a");
        let b = pkg("github", "owner", "b");
        let c = pkg("github", "owner", "c");

        // c depends on b, b depends on a
        graph.add_edge(c.clone(), b.clone());
        graph.add_edge(b.clone(), a.clone());

        let sorted = graph.topological_sort().unwrap();
        assert_eq!(sorted, vec![a, b, c]);
    }

    #[test]
    fn test_topological_sort_cycle() {
        let mut graph = DependencyGraph::new();
        let a = pkg("github", "owner", "a");
        let b = pkg("github", "owner", "b");

        graph.add_edge(a.clone(), b.clone());
        graph.add_edge(b.clone(), a.clone());

        assert!(graph.topological_sort().is_err());
    }

    #[test]
    fn test_find_orphans() {
        let mut graph = DependencyGraph::new();
        let a = pkg("github", "owner", "a");
        let b = pkg("github", "owner", "b");
        let c = pkg("github", "owner", "c");

        // a depends on b
        graph.add_edge(a.clone(), b.clone());
        graph.add_node(c.clone());

        let installed = vec![a.clone(), b.clone(), c.clone()];
        let explicit = vec![a.clone()];

        let orphans = graph.find_orphans(&installed, &explicit);
        assert_eq!(orphans, vec![c]);
    }

    #[test]
    fn test_reverse_deps() {
        let mut graph = DependencyGraph::new();
        let a = pkg("github", "owner", "a");
        let b = pkg("github", "owner", "b");
        let c = pkg("github", "owner", "c");

        graph.add_edge(a.clone(), b.clone());
        graph.add_edge(c.clone(), b.clone());

        let rev = graph.reverse_deps(&b);
        assert_eq!(rev.len(), 2);
        assert!(rev.contains(&a));
        assert!(rev.contains(&c));
    }
}
