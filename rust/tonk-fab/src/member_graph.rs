//! Deterministic geometry for the invitation graph. Missing history and cycles
//! remain disconnected rather than manufacturing a path to the space.

use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Member {
    pub this: String,
    pub name: String,
    pub did: String,
    pub role: String,
    pub invitation: Option<String>,
}

pub(crate) struct Node<'a> {
    pub member: &'a Member,
    /// None means the inviter is unknown; index zero is the space.
    pub parent: Option<usize>,
    pub rooted: bool,
    pub x: f64,
    pub y: f64,
}

pub(crate) struct Graph<'a> {
    pub nodes: Vec<Node<'a>>,
    pub size: f64,
}

/// Gooey's elliptical lens: full size in the middle half of the stage,
/// easing down to 40% just beyond its boundary. Coordinates are normalized
/// by the viewport's half-width/half-height, independently of graph zoom.
pub(crate) fn peripheral_scale(x: f64, y: f64) -> f64 {
    let t = ((x.hypot(y) - 0.5) / 0.6).clamp(0.0, 1.0);
    1.0 - 0.6 * t * t * (3.0 - 2.0 * t)
}

pub(crate) fn layout<'a>(
    members: &'a [Member],
    invitations: &BTreeMap<String, String>,
) -> Graph<'a> {
    let mut sorted: Vec<_> = members.iter().collect();
    sorted.sort_by(|a, b| a.did.cmp(&b.did).then(a.this.cmp(&b.this)));
    sorted.dedup_by(|a, b| a.did == b.did);
    let indices: BTreeMap<_, _> = sorted
        .iter()
        .enumerate()
        .map(|(i, m)| (m.did.as_str(), i + 1))
        .collect();
    let parents: Vec<_> = sorted
        .iter()
        .map(|m| {
            if m.role == "tonk:founder" {
                Some(0)
            } else {
                m.invitation
                    .as_ref()
                    .and_then(|id| invitations.get(id))
                    .and_then(|did| indices.get(did.as_str()).copied())
                    .filter(|parent| sorted[*parent - 1].did != m.did)
            }
        })
        .collect();
    let depths: Vec<_> = (0..sorted.len())
        .map(|i| {
            let mut seen = BTreeSet::new();
            let mut cursor = i + 1;
            while cursor != 0 {
                if !seen.insert(cursor) {
                    return None;
                }
                cursor = parents[cursor - 1]?;
            }
            Some(seen.len())
        })
        .collect();
    // Compact deterministic spring layout, with the space pinned at the centre.
    // A chain can bend around the space instead of requiring a huge empty ring
    // for every generation. Resolve glyph collisions after each spring step.
    let count = sorted.len();
    let mut positions = vec![(0.0_f64, 0.0_f64)];
    let initial_radius = (count as f64 * 160.0 / std::f64::consts::TAU).max(140.0);
    positions.extend((0..count).map(|i| {
        let angle = -std::f64::consts::FRAC_PI_2 + std::f64::consts::TAU * i as f64 / count as f64;
        (initial_radius * angle.cos(), initial_radius * angle.sin())
    }));
    for _ in 0..160 {
        for (i, parent) in parents.iter().enumerate() {
            let Some(parent) = *parent else { continue };
            let (dx, dy) = (
                positions[parent].0 - positions[i + 1].0,
                positions[parent].1 - positions[i + 1].1,
            );
            let distance = dx.hypot(dy).max(1.0);
            let strength = ((distance - 104.0) * 0.08).clamp(-8.0, 8.0) / distance;
            positions[i + 1].0 += dx * strength;
            positions[i + 1].1 += dy * strength;
            if parent != 0 {
                positions[parent].0 -= dx * strength;
                positions[parent].1 -= dy * strength;
            }
        }
        for i in 1..positions.len() {
            for j in 0..i {
                let (mut dx, mut dy) = (
                    positions[i].0 - positions[j].0,
                    positions[i].1 - positions[j].1,
                );
                let mut distance = dx.hypot(dy);
                if distance < 0.01 {
                    dx = 1.0;
                    dy = 0.0;
                    distance = 1.0;
                }
                if distance < 104.0 {
                    let shift = (104.0 - distance) / distance;
                    let share = if j == 0 { 1.0 } else { 0.5 };
                    positions[i].0 += dx * shift * share;
                    positions[i].1 += dy * shift * share;
                    if j != 0 {
                        positions[j].0 -= dx * shift * share;
                        positions[j].1 -= dy * shift * share;
                    }
                }
            }
        }
    }
    let radius = positions
        .iter()
        .fold(0.0_f64, |r, (x, y)| r.max(x.abs()).max(y.abs()));
    let size = (radius * 2.0 + 160.0).max(400.0);
    let nodes = sorted
        .into_iter()
        .enumerate()
        .map(|(i, member)| Node {
            member,
            parent: parents[i],
            rooted: depths[i].is_some(),
            x: positions[i + 1].0 + size / 2.0,
            y: positions[i + 1].1 + size / 2.0,
        })
        .collect();
    Graph { nodes, size }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn member(did: &str, invitation: Option<&str>) -> Member {
        Member {
            this: did.into(),
            did: did.into(),
            name: did.into(),
            role: "tonk:member".into(),
            invitation: invitation.map(String::from),
        }
    }
    #[test]
    fn lens_is_symmetric_bounded_and_shrinks_smoothly_toward_the_edge() {
        assert_eq!(peripheral_scale(0.0, 0.0), 1.0);
        assert_eq!(peripheral_scale(0.5, 0.0), 1.0);
        assert!((peripheral_scale(0.8, 0.0) - 0.7).abs() < 1e-9);
        assert_eq!(peripheral_scale(1.1, 0.0), 0.4);
        assert_eq!(peripheral_scale(10.0, 10.0), 0.4);
        assert_eq!(peripheral_scale(-0.8, 0.0), peripheral_scale(0.0, 0.8));
        assert!(peripheral_scale(0.8, 0.8) < peripheral_scale(0.8, 0.0));
    }

    #[test]
    fn traces_generations_and_keeps_missing_history_disconnected() {
        let mut owner = member("a", None);
        owner.role = "tonk:founder".into();
        let members = vec![
            member("d", None),
            member("c", Some("bc")),
            owner,
            member("b", Some("ab")),
        ];
        let graph = layout(
            &members,
            &BTreeMap::from([("ab".into(), "a".into()), ("bc".into(), "b".into())]),
        );
        assert_eq!(
            graph.nodes.iter().map(|n| n.parent).collect::<Vec<_>>(),
            vec![Some(0), Some(1), Some(2), None]
        );
        assert_eq!(graph.nodes[0].member.did, "a");
        assert!(graph.nodes[..3].iter().all(|n| n.rooted));
        assert!(!graph.nodes[3].rooted);
        assert!(
            graph
                .nodes
                .iter()
                .all(|n| n.x >= 0.0 && n.y >= 0.0 && n.x <= graph.size && n.y <= graph.size)
        );
    }
    #[test]
    fn cycles_and_absent_inviters_do_not_gain_a_space_edge() {
        let members = vec![
            member("a", Some("ba")),
            member("b", Some("ab")),
            member("c", Some("missing")),
        ];
        let graph = layout(
            &members,
            &BTreeMap::from([
                ("ba".into(), "b".into()),
                ("ab".into(), "a".into()),
                ("missing".into(), "gone".into()),
            ]),
        );
        assert!(graph.nodes.iter().all(|n| !n.rooted));
        assert_eq!(graph.nodes[2].parent, None);
    }
    #[test]
    fn large_graphs_leave_room_for_labels_and_layout_is_order_independent() {
        let members: Vec<_> = (0..40)
            .map(|i| member(&format!("member-{i:02}"), None))
            .collect();
        let graph = layout(&members, &BTreeMap::new());
        for (i, a) in graph.nodes.iter().enumerate() {
            for b in &graph.nodes[i + 1..] {
                assert!((a.x - b.x).hypot(a.y - b.y) > 150.0);
            }
        }
        let mut reversed = members.clone();
        reversed.reverse();
        let other = layout(&reversed, &BTreeMap::new());
        assert_eq!(
            graph.nodes.iter().map(|n| (n.x, n.y)).collect::<Vec<_>>(),
            other.nodes.iter().map(|n| (n.x, n.y)).collect::<Vec<_>>()
        );
    }
}
