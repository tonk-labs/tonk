//! Executable completion-contract spike, not connected to the renderer.
//! Register work before awaiting or scheduling it. Seal only after synchronous
//! reconciliation has registered every input, child and asynchronous task.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Ticket(u64);

fn ticket() -> Ticket {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    Ticket(
        NEXT.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .expect("ticket exhaustion"),
    )
}

/// Opaque identities supplied by the existing query-checkpoint protocol.
/// Different input scopes need not have the same durable or overlay revision.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Checkpoint {
    scope: String,
    durable: Option<String>,
    overlay: String,
}

#[derive(Default)]
struct Node {
    parent: Option<Ticket>,
    children: BTreeSet<Ticket>,
    inputs: BTreeMap<Ticket, Option<Checkpoint>>,
    work: BTreeSet<Ticket>,
    sealed: bool,
    unsupported: bool,
}

struct Barrier {
    owner: Ticket,
    root: Ticket,
    version: u64,
    nodes: BTreeMap<Ticket, Node>,
}

#[derive(Debug)]
struct Receipt {
    owner: Ticket,
    version: u64,
    /// A vector of input identities, deliberately not one renderedRevision.
    inputs: BTreeMap<Ticket, Checkpoint>,
}

impl Barrier {
    fn new() -> Self {
        let root = ticket();
        Self {
            owner: ticket(),
            root,
            version: 0,
            nodes: BTreeMap::from([(root, Node::default())]),
        }
    }

    fn changed(&mut self) {
        self.version = self.version.checked_add(1).expect("generation exhaustion");
    }

    fn child(&mut self, parent: Ticket) -> Option<Ticket> {
        let node = self.nodes.get_mut(&parent)?;
        let id = ticket();
        node.children.insert(id);
        node.sealed = false;
        self.nodes.insert(
            id,
            Node {
                parent: Some(parent),
                ..Node::default()
            },
        );
        self.changed();
        Some(id)
    }

    /// Restart invalidates the entire subtree and all of its in-flight tickets.
    fn restart(&mut self, id: Ticket) -> Option<Ticket> {
        let parent = self.nodes.get(&id)?.parent;
        self.remove(id);
        let fresh = if let Some(parent) = parent {
            self.child(parent)?
        } else {
            let fresh = ticket();
            self.root = fresh;
            self.nodes.insert(fresh, Node::default());
            fresh
        };
        self.changed();
        Some(fresh)
    }

    fn remove(&mut self, id: Ticket) {
        if let Some(node) = self.nodes.remove(&id) {
            for child in node.children {
                self.remove(child);
            }
            if let Some(parent) = node.parent.and_then(|p| self.nodes.get_mut(&p)) {
                parent.children.remove(&id);
                parent.sealed = false;
            }
            self.changed();
        }
    }

    /// Start/restart an input on subscription replacement or a new row frame.
    /// `old` is retired so its delayed checkpoint cannot acknowledge new rows.
    fn input(&mut self, id: Ticket, old: Option<Ticket>) -> Option<Ticket> {
        let node = self.nodes.get_mut(&id)?;
        if let Some(old) = old {
            node.inputs.remove(&old)?;
        }
        let input = ticket();
        node.inputs.insert(input, None);
        node.sealed = false;
        self.changed();
        Some(input)
    }

    fn acknowledge(&mut self, id: Ticket, input: Ticket, checkpoint: Checkpoint) -> bool {
        let Some(slot) = self
            .nodes
            .get_mut(&id)
            .and_then(|n| n.inputs.get_mut(&input))
        else {
            return false;
        };
        if slot.as_ref() != Some(&checkpoint) {
            *slot = Some(checkpoint);
            self.changed();
        }
        true
    }

    fn begin_work(&mut self, id: Ticket) -> Option<Ticket> {
        let node = self.nodes.get_mut(&id)?;
        let work = ticket();
        node.work.insert(work);
        node.sealed = false;
        self.changed();
        Some(work)
    }

    fn finish_work(&mut self, id: Ticket, work: Ticket) -> bool {
        if self
            .nodes
            .get_mut(&id)
            .is_some_and(|n| n.work.remove(&work))
        {
            self.changed();
            true
        } else {
            false
        }
    }

    fn seal(&mut self, id: Ticket) {
        if let Some(node) = self.nodes.get_mut(&id) {
            node.sealed = true;
            self.changed();
        }
    }

    /// Error, untracked fallback, held history or opaque portal: fail closed.
    /// Only restarting the node can restore eligibility.
    fn unsupported(&mut self, id: Ticket) {
        if let Some(node) = self.nodes.get_mut(&id) {
            node.unsupported = true;
            self.changed();
        }
    }

    fn receipt(&self) -> Option<Receipt> {
        self.nodes.get(&self.root)?;
        let mut inputs = BTreeMap::new();
        for node in self.nodes.values() {
            if !node.sealed || node.unsupported || !node.work.is_empty() || node.inputs.is_empty() {
                return None;
            }
            for (id, checkpoint) in &node.inputs {
                inputs.insert(*id, checkpoint.clone()?);
            }
        }
        Some(Receipt {
            owner: self.owner,
            version: self.version,
            inputs,
        })
    }

    fn is_current(&self, receipt: &Receipt) -> bool {
        receipt.owner == self.owner && receipt.version == self.version && self.receipt().is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn cp(scope: &str, value: &str) -> Checkpoint {
        Checkpoint {
            scope: scope.into(),
            durable: Some(value.into()),
            overlay: "overlay-1".into(),
        }
    }
    fn ready(b: &mut Barrier, node: Ticket, scope: &str) -> Ticket {
        let input = b.input(node, None).unwrap();
        assert!(b.acknowledge(node, input, cp(scope, "rev-1")));
        b.seal(node);
        input
    }

    #[test]
    fn nested_loading_and_late_child_discovery_block_completion() {
        let mut b = Barrier::new();
        let root = b.root;
        ready(&mut b, root, "root");
        let initial = b.receipt().unwrap();
        // Register the mount task BEFORE its asynchronous wait.
        let mount = b.begin_work(root).unwrap();
        b.seal(root);
        assert!(!b.is_current(&initial));
        assert!(b.receipt().is_none());
        // The mount resumes and creates a nested display before finishing.
        let child = b.child(root).unwrap();
        b.finish_work(root, mount);
        b.seal(root);
        assert!(b.receipt().is_none());
        ready(&mut b, child, "child");
        let complete = b.receipt().unwrap();
        assert_eq!(complete.inputs.len(), 2);
        assert!(b.is_current(&complete));
    }

    #[test]
    fn paused_child_mount_cannot_publish_a_premature_receipt() {
        use std::sync::{Arc, Mutex, mpsc};
        let mut b = Barrier::new();
        let root = b.root;
        ready(&mut b, root, "parent");
        let old = b.receipt().unwrap();
        let mount = b.begin_work(root).unwrap();
        b.seal(root);
        let shared = Arc::new(Mutex::new(b));
        let (resume, gate) = mpsc::channel();
        let (events, observed) = mpsc::channel();
        let worker_state = shared.clone();
        let worker = std::thread::spawn(move || {
            gate.recv().unwrap();
            let (child, input) = {
                let mut b = worker_state.lock().unwrap();
                let child = b.child(root).unwrap();
                let input = b.input(child, None).unwrap();
                b.seal(child);
                assert!(b.finish_work(root, mount));
                b.seal(root);
                (child, input)
            };
            events.send(()).unwrap();
            gate.recv().unwrap();
            let mut b = worker_state.lock().unwrap();
            assert!(b.acknowledge(child, input, cp("child", "child-revision")));
        });
        assert!(shared.lock().unwrap().receipt().is_none());
        resume.send(()).unwrap();
        observed.recv().unwrap();
        {
            let b = shared.lock().unwrap();
            assert!(
                b.receipt().is_none(),
                "parent finished but child data is pending"
            );
            assert!(!b.is_current(&old));
        }
        resume.send(()).unwrap();
        worker.join().unwrap();
        let b = shared.lock().unwrap();
        let receipt = b.receipt().unwrap();
        assert_eq!(receipt.inputs.len(), 2);
        assert!(b.is_current(&receipt));
    }

    #[test]
    fn navigation_rejects_old_work_and_child_acknowledgments() {
        let mut b = Barrier::new();
        let root = b.root;
        let child = b.child(root).unwrap();
        let input = ready(&mut b, child, "old");
        let work = b.begin_work(child).unwrap();
        let fresh = b.restart(root).unwrap();
        assert!(!b.acknowledge(child, input, cp("old", "late")));
        assert!(!b.finish_work(child, work));
        assert!(b.child(root).is_none());
        assert!(b.receipt().is_none());
        ready(&mut b, fresh, "new");
        assert_eq!(b.receipt().unwrap().inputs.len(), 1);
    }

    #[test]
    fn replaced_input_cannot_be_satisfied_by_its_old_checkpoint() {
        let mut b = Barrier::new();
        let root = b.root;
        let old = ready(&mut b, root, "entity");
        let previous = b.receipt().unwrap();
        let new = b.input(root, Some(old)).unwrap();
        b.seal(root);
        assert!(!b.acknowledge(root, old, cp("entity", "old")));
        assert!(!b.is_current(&previous));
        assert!(b.receipt().is_none());
        assert!(b.acknowledge(root, new, cp("entity", "new")));
        assert!(b.receipt().is_some());
    }

    #[test]
    fn unchanged_rows_can_advance_provenance_without_new_mount_work() {
        let mut b = Barrier::new();
        let root = b.root;
        let input = ready(&mut b, root, "entity");
        let old = b.receipt().unwrap();
        b.acknowledge(root, input, cp("entity", "rev-2"));
        assert!(!b.is_current(&old));
        let new = b.receipt().unwrap();
        b.acknowledge(root, input, cp("entity", "rev-2"));
        assert!(b.is_current(&new));
    }

    #[test]
    fn unsupported_nested_work_requires_a_restart_even_after_checkpoints_arrive() {
        let mut b = Barrier::new();
        let root = b.root;
        let child = b.child(root).unwrap();
        ready(&mut b, root, "root");
        let input = ready(&mut b, child, "child");
        let old = b.receipt().unwrap();
        b.unsupported(child);
        b.acknowledge(child, input, cp("child", "new"));
        b.seal(child);
        assert!(!b.is_current(&old));
        assert!(b.receipt().is_none());
        let new_child = b.restart(child).unwrap();
        ready(&mut b, new_child, "child");
        assert!(
            b.receipt().is_none(),
            "parent must reconcile changed child membership"
        );
        b.seal(root);
        assert!(b.receipt().is_some());
    }

    #[test]
    fn empty_or_unsealed_nodes_and_cross_tree_tickets_cannot_complete() {
        let mut a = Barrier::new();
        let root = a.root;
        a.seal(root);
        assert!(a.receipt().is_none());
        let input = ready(&mut a, root, "a");
        let receipt = a.receipt().unwrap();
        let mut b = Barrier::new();
        assert!(!b.acknowledge(b.root, input, cp("a", "late")));
        let other_root = b.root;
        ready(&mut b, other_root, "b");
        assert!(!b.is_current(&receipt));
        let child = a.child(root).unwrap();
        ready(&mut a, child, "child");
        assert!(a.receipt().is_none(), "membership changed after seal");
        a.seal(root);
        assert!(a.receipt().is_some());
        a.remove(child);
        assert!(a.receipt().is_none());
        a.seal(root);
        assert!(a.receipt().is_some());
    }
}
