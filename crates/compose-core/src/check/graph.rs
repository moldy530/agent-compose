//! One flow's graph, built once and read by every graph check.
//!
//! Grammar 7 states four *different* relations over a flow's nodes, and D95 is
//! emphatic that they are four questions rather than one: the **edge** relation
//! (grammar 7.2), which is what SCC termination (7.4), `dist` (7.6.2) and
//! `map.over` dominance (8.6 rule 11) are all computed over; the
//! **control-transfer** relation (7.8), which is the edge relation plus
//! `on_error: { fallback: … }` and `human.on_timeout:`, and answers only
//! "is this node addressable at all"; grammar 7.7's **component** relation,
//! which is about other definitions entirely and lives in [`reach`](super::reach);
//! and grammar 7.6.1's **concurrency** relation, which is derived from the edge
//! relation by [`convergence`](super::convergence).
//!
//! This module owns the first two. Nothing here reports a diagnostic — it
//! answers questions, and the modules that ask them decide what is wrong.
//!
//! # Vertices
//!
//! A flow's vertices are its nodes plus the two pseudo-nodes (grammar 2.4).
//! `start` and `end` are [`Vertex`] variants rather than node indexes because
//! they are not nodes: nothing targets `start`, nothing leaves `end`, and
//! neither can belong to a cycle or be a convergence. Every node id an edge
//! names has been resolved against this flow already (see
//! [`resolve`](crate::resolve)), so the lookup below cannot fail on a
//! composition that reached this pass.
//!
//! # Cycles
//!
//! [`Graph::sccs`] is Tarjan's algorithm (PRD 5.4), written iteratively: the
//! recursion depth of the textbook form is the node count, and a flow's node
//! count is bounded by nothing this compiler controls.
//!
//! An SCC is a **cycle** when it has at least one edge — either two or more
//! members, or one member with a self-edge (grammar 7.2: "Self-edges … form a
//! one-node SCC, which must be bounded like any other cycle").

use std::cell::OnceCell;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::ast::common::{ControlTarget, EdgeSource, EdgeTarget};
use crate::ir::flow::{Edge, Flow, Node, NodeKind};

/// A vertex of one flow's graph (grammar 2.4).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum Vertex {
    /// The `start` pseudo-node: the flow's entry, which nothing targets.
    Start,
    /// A node, by its index in declaration order.
    Node(usize),
    /// The `end` pseudo-node: where a branch retires.
    End,
}

/// One flow's graph.
pub(crate) struct Graph<'a> {
    nodes: Vec<&'a Node>,
    edges: &'a [Edge],
    /// Per edge, the vertices it joins — `None` on the side naming a node this
    /// flow does not declare, which the resolver has already refused.
    endpoints: Vec<(Option<Vertex>, Option<Vertex>)>,
    /// Edge indexes leaving `start`, in declaration order.
    from_start: Vec<usize>,
    /// Per node, the edge indexes leaving it, in declaration order.
    from_node: Vec<Vec<usize>>,
    /// Per node, the distinct node indexes an edge takes it to.
    successors: Vec<Vec<usize>>,
    /// Per node, the strongly connected component it belongs to.
    component: Vec<usize>,
    /// Per component, its members in ascending index order.
    components: Vec<Vec<usize>>,
    /// Per node, whether its component has at least one edge.
    cyclic: Vec<bool>,
    /// Per node, the nodes reachable from it over edges — the one answer here
    /// that costs a walk per node, so it is computed on first use. Only the
    /// concurrency relation asks for it (grammar 7.6.1), and a flow with no fork
    /// never does.
    reachable: OnceCell<Vec<Vec<bool>>>,
}

impl<'a> Graph<'a> {
    /// Build the graph of one flow.
    pub(crate) fn new(flow: &'a Flow) -> Self {
        let nodes: Vec<&'a Node> = flow.nodes.iter().collect();
        let index: BTreeMap<&str, usize> = nodes
            .iter()
            .enumerate()
            .map(|(at, node)| (node.id.value.as_str(), at))
            .collect();
        let count = nodes.len();

        // An edge naming a node this flow does not declare has been refused by
        // the resolver already, so it cannot reach this pass — but `endpoints`
        // is indexed by edge, so such an edge keeps its slot rather than
        // shifting every later one, and simply joins nothing.
        let endpoints: Vec<(Option<Vertex>, Option<Vertex>)> = flow
            .edges
            .iter()
            .map(|edge| {
                let from = match &edge.from.value {
                    EdgeSource::Start => Some(Vertex::Start),
                    EdgeSource::Node(id) => index.get(id.as_str()).map(|at| Vertex::Node(*at)),
                };
                let to = match &edge.to.value {
                    EdgeTarget::End => Some(Vertex::End),
                    EdgeTarget::Node(id) => index.get(id.as_str()).map(|at| Vertex::Node(*at)),
                };
                (from, to)
            })
            .collect();

        let mut from_start = Vec::new();
        let mut from_node = vec![Vec::new(); count];
        let mut successors: Vec<Vec<usize>> = vec![Vec::new(); count];
        for (at, (from, to)) in endpoints.iter().enumerate() {
            let (Some(from), Some(to)) = (from, to) else {
                continue;
            };
            match from {
                Vertex::Start => from_start.push(at),
                Vertex::Node(node) => {
                    from_node[*node].push(at);
                    if let Vertex::Node(target) = to
                        && !successors[*node].contains(target)
                    {
                        successors[*node].push(*target);
                    }
                }
                Vertex::End => {}
            }
        }

        let (component, components) = components(&successors);
        let cyclic = components
            .iter()
            .map(|members| {
                members.len() > 1
                    || members
                        .first()
                        .is_some_and(|member| successors[*member].contains(member))
            })
            .collect::<Vec<bool>>();
        let cyclic = component.iter().map(|at| cyclic[*at]).collect();

        Self {
            nodes,
            edges: &flow.edges,
            endpoints,
            from_start,
            from_node,
            successors,
            component,
            components,
            cyclic,
            reachable: OnceCell::new(),
        }
    }

    /// The flow's nodes, in declaration order.
    pub(crate) fn nodes(&self) -> &[&'a Node] {
        &self.nodes
    }

    /// The node at this index.
    pub(crate) fn node(&self, at: usize) -> &'a Node {
        self.nodes[at]
    }

    /// The node's flow-local id.
    pub(crate) fn id(&self, at: usize) -> &'a str {
        self.nodes[at].id.value.as_str()
    }

    /// The edge at this index.
    pub(crate) fn edge(&self, at: usize) -> &'a Edge {
        &self.edges[at]
    }

    /// Where the edge at this index goes. Every edge [`Graph::outgoing`] yields
    /// has one.
    pub(crate) fn target(&self, at: usize) -> Option<Vertex> {
        self.endpoints[at].1
    }

    /// The nodes an edge leaving `start` reaches directly.
    fn entries(&self) -> impl Iterator<Item = usize> + '_ {
        self.from_start
            .iter()
            .filter_map(|edge| match self.endpoints[*edge].1 {
                Some(Vertex::Node(at)) => Some(at),
                _ => None,
            })
    }

    /// The edges leaving a vertex, in declaration order — which is the order
    /// grammar 7.3 evaluates them in.
    pub(crate) fn outgoing(&self, vertex: Vertex) -> &[usize] {
        match vertex {
            Vertex::Start => &self.from_start,
            Vertex::Node(at) => &self.from_node[at],
            Vertex::End => &[],
        }
    }

    /// Every vertex that has an outgoing edge and so may be a fork
    /// (grammar 7.6.1): `start`, then the nodes in declaration order.
    pub(crate) fn sources(&self) -> Vec<Vertex> {
        std::iter::once(Vertex::Start)
            .chain((0..self.nodes.len()).map(Vertex::Node))
            .collect()
    }

    /// The strongly connected components, each a list of node indexes in
    /// ascending order.
    pub(crate) fn components(&self) -> &[Vec<usize>] {
        &self.components
    }

    /// Whether two nodes belong to one strongly connected component.
    pub(crate) fn same_component(&self, left: usize, right: usize) -> bool {
        self.component[left] == self.component[right]
    }

    /// Whether this vertex is inside the component the node belongs to. `start`
    /// and `end` never are, which is what makes an edge to `end` an edge that
    /// *leaves* the SCC (grammar 7.4 clause 2(i)).
    pub(crate) fn inside_component_of(&self, node: usize, vertex: Vertex) -> bool {
        matches!(vertex, Vertex::Node(other) if self.same_component(node, other))
    }

    /// Whether the node belongs to a cycle — an SCC with at least one edge
    /// (grammar 7.2, 7.4).
    pub(crate) fn cyclic(&self, at: usize) -> bool {
        self.cyclic[at]
    }

    /// Whether one node is reachable from another over edges. A node reaches
    /// itself only through a cycle, which is what "neither is reachable from the
    /// other" needs of it (grammar 7.6.1).
    pub(crate) fn reaches(&self, from: usize, to: usize) -> bool {
        self.reachable()[from][to]
    }

    fn reachable(&self) -> &Vec<Vec<bool>> {
        self.reachable.get_or_init(|| {
            (0..self.nodes.len())
                .map(|node| {
                    walk(
                        self.nodes.len(),
                        &self.successors,
                        self.successors[node].iter().copied(),
                    )
                })
                .collect()
        })
    }

    /// The nodes reachable over edges from the target of one edge, the edge
    /// itself included — "reachable from a fork **through** this edge"
    /// (grammar 7.6.1).
    ///
    /// The answer is the node indexes themselves, in ascending order, rather than
    /// a mask over every node: its reader pairs one edge's answer with another's
    /// (see [`convergence`](super::convergence)), and pairing two *lists* costs
    /// what the branches hold instead of what the flow holds.
    pub(crate) fn reachable_through(&self, edge: usize) -> Vec<usize> {
        let Some(Vertex::Node(entry)) = self.endpoints[edge].1 else {
            return Vec::new();
        };
        let reached = &self.reachable()[entry];
        (0..self.nodes.len())
            .filter(|at| *at == entry || reached[*at])
            .collect()
    }

    /// Whether `dominator` dominates `node`: every path from `start` to `node`
    /// over **edges** passes through it (grammar 8.6 rule 11, Decision D76).
    ///
    /// A node no edge path reaches is not asked about — a dominator relation
    /// over an empty set of paths is vacuously true, and reporting a producer
    /// that "does not dominate" a node nothing reaches would name the wrong
    /// mistake. Grammar 7.8's reachability is where that node is answered for.
    pub(crate) fn dominates(&self, dominator: usize, node: usize) -> bool {
        if dominator == node {
            return true;
        }
        if !walk(self.nodes.len(), &self.successors, self.entries())[node] {
            return true;
        }
        let mut successors = self.successors.clone();
        successors[dominator].clear();
        let entries = self.entries().filter(|at| *at != dominator);
        !walk(self.nodes.len(), &successors, entries)[node]
    }

    /// Every node reachable from `start` over the **control-transfer** relation:
    /// edges, `on_error: { fallback: … }`, and `human.on_timeout:`
    /// (grammar 7.8, Decision D95).
    pub(crate) fn addressable(&self) -> Vec<bool> {
        walk(self.nodes.len(), &self.transfers(), self.entries())
    }

    /// Every node a cycle can run, the cycle's own members included — the nodes
    /// that execute **once per pass** rather than once per flow instance.
    ///
    /// The seeds are the members of every SCC with an edge, because that is what
    /// a cycle is (grammar 7.2, 7.4) and the **edge** relation is what SCCs are
    /// computed over (Decision D95). What is walked *from* them is the wider
    /// **control-transfer** relation: a node a looping node falls back to, or
    /// times out into, runs once per pass exactly as an edge target does, and the
    /// question here is how many times a node can run rather than how it was
    /// addressed.
    pub(crate) fn reached_by_cycle(&self) -> Vec<bool> {
        walk(
            self.nodes.len(),
            &self.transfers(),
            (0..self.nodes.len()).filter(|at| self.cyclic(*at)),
        )
    }

    /// The control-transfer relation as an adjacency list: the edge relation plus
    /// `on_error: { fallback: … }` and `human.on_timeout:` (grammar 7.8).
    fn transfers(&self) -> Vec<Vec<usize>> {
        let mut transfers: Vec<Vec<usize>> = self.successors.clone();
        for (at, node) in self.nodes.iter().enumerate() {
            for target in control_targets(node) {
                if let Some(to) = self
                    .nodes
                    .iter()
                    .position(|other| other.id.value.as_str() == target)
                    && !transfers[at].contains(&to)
                {
                    transfers[at].push(to);
                }
            }
        }
        transfers
    }

    /// The step distances from a fork to every node it delivers to, over the
    /// paths that leave it by one edge and traverse no node belonging to a cycle
    /// (grammar 7.6.2).
    ///
    /// Excluding cycle nodes is what makes the walk finite *and* the answer
    /// meaningful: a path that repeats a node passes through a cycle, so a path
    /// this walk admits visits each node at most once and its length is at most
    /// the node count. Where a cycle lies on the way, `dist` is not computed and
    /// the runtime rule governs instead.
    ///
    /// Only the nodes this edge actually delivers to are keyed — a node absent
    /// from the map is one no admitted path reaches, which is the empty set of
    /// distances said in the space a branch occupies rather than the space the
    /// flow occupies.
    pub(crate) fn distances(&self, edge: usize) -> BTreeMap<usize, BTreeSet<usize>> {
        let mut distances: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
        let Some(Vertex::Node(entry)) = self.endpoints[edge].1 else {
            return distances;
        };
        if self.cyclic(entry) {
            return distances;
        }
        let mut queue = VecDeque::from([(entry, 1usize)]);
        distances.entry(entry).or_default().insert(1);
        while let Some((node, distance)) = queue.pop_front() {
            for next in &self.successors[node] {
                if self.cyclic(*next) {
                    continue;
                }
                if distances.entry(*next).or_default().insert(distance + 1) {
                    queue.push_back((*next, distance + 1));
                }
            }
        }
        distances
    }
}

/// The flow-local node ids one node transfers control to (grammar 7.8 clauses 2
/// and 3). `end` is not a node and is never one of them.
fn control_targets(node: &Node) -> Vec<&str> {
    let mut targets = Vec::new();
    if let Some(crate::ir::policy::OnError::Fallback { target, .. }) = &node.policy.on_error
        && let ControlTarget::Node(id) = &target.value
    {
        targets.push(id.as_str());
    }
    if let NodeKind::Human { human } = &node.kind
        && let Some(on_timeout) = &human.on_timeout
        && let ControlTarget::Node(id) = &on_timeout.value
    {
        targets.push(id.as_str());
    }
    targets
}

/// Breadth-first reachability from a set of entry nodes.
fn walk(
    count: usize,
    successors: &[Vec<usize>],
    entries: impl Iterator<Item = usize>,
) -> Vec<bool> {
    let mut found = vec![false; count];
    let mut queue = VecDeque::new();
    for entry in entries {
        if !found[entry] {
            found[entry] = true;
            queue.push_back(entry);
        }
    }
    while let Some(node) = queue.pop_front() {
        for next in &successors[node] {
            if !found[*next] {
                found[*next] = true;
                queue.push_back(*next);
            }
        }
    }
    found
}

/// Tarjan's strongly-connected-components algorithm, iteratively (PRD 5.4).
///
/// Returns each node's component and the components themselves, every member
/// list in ascending index order. The walk is over a bare adjacency list rather
/// than over a [`Graph`], because the *invocation* graph of grammar 7.7 needs
/// exactly this and for exactly the same reason: recursion is a cycle among
/// flows the way an unbounded loop is a cycle among nodes
/// ([`components`](super::components)).
///
/// It is iterative because the recursion depth of the textbook form is the
/// vertex count, and neither a flow's node count nor a composition's flow count
/// is bounded by anything this compiler controls.
pub(crate) fn components(successors: &[Vec<usize>]) -> (Vec<usize>, Vec<Vec<usize>>) {
    let count = successors.len();
    let mut order: Vec<Option<usize>> = vec![None; count];
    let mut low = vec![0usize; count];
    let mut on_stack = vec![false; count];
    let mut stack: Vec<usize> = Vec::new();
    let mut component = vec![usize::MAX; count];
    let mut components: Vec<Vec<usize>> = Vec::new();
    let mut counter = 0usize;

    for root in 0..count {
        if order[root].is_some() {
            continue;
        }
        order[root] = Some(counter);
        low[root] = counter;
        counter += 1;
        stack.push(root);
        on_stack[root] = true;
        let mut work: Vec<(usize, usize)> = vec![(root, 0)];
        while let Some((node, next)) = work.last().copied() {
            if next < successors[node].len() {
                work.last_mut().expect("the frame was just read").1 += 1;
                let child = successors[node][next];
                match order[child] {
                    None => {
                        order[child] = Some(counter);
                        low[child] = counter;
                        counter += 1;
                        stack.push(child);
                        on_stack[child] = true;
                        work.push((child, 0));
                    }
                    Some(at) if on_stack[child] => low[node] = low[node].min(at),
                    Some(_) => {}
                }
                continue;
            }
            work.pop();
            if let Some((parent, _)) = work.last().copied() {
                low[parent] = low[parent].min(low[node]);
            }
            if Some(low[node]) == order[node] {
                let mut members = Vec::new();
                while let Some(member) = stack.pop() {
                    on_stack[member] = false;
                    component[member] = components.len();
                    members.push(member);
                    if member == node {
                        break;
                    }
                }
                members.sort_unstable();
                components.push(members);
            }
        }
    }
    (component, components)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A diamond, a self-loop, and a two-node cycle, as adjacency lists.
    #[test]
    fn tarjan_finds_every_component() {
        // 0 -> 1 -> 2 -> 1, 0 -> 3 -> 3, 4 isolated.
        let successors = vec![vec![1, 3], vec![2], vec![1], vec![3], vec![]];
        let (component, components) = components(&successors);
        assert_eq!(components.len(), 4);
        assert_ne!(component[0], component[1]);
        assert_eq!(component[1], component[2], "1 and 2 form one cycle");
        assert_ne!(component[3], component[1]);
        let cycle = &components[component[1]];
        assert_eq!(cycle, &vec![1, 2], "members are sorted");
    }

    /// The walk has to survive a chain deeper than a recursive Tarjan's stack
    /// would, which is the reason it is iterative.
    #[test]
    fn tarjan_survives_a_long_chain() {
        let length = 50_000;
        let successors: Vec<Vec<usize>> = (0..length)
            .map(|at| {
                if at + 1 < length {
                    vec![at + 1]
                } else {
                    vec![]
                }
            })
            .collect();
        let (_, components) = components(&successors);
        assert_eq!(components.len(), length);
    }
}
