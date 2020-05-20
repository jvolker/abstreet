use crate::{IntersectionID, LaneID, Map, TurnID};
use abstutil::MultiMap;
use geom::{Distance, PolyLine, Pt2D};
use petgraph::graphmap::UnGraphMap;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

// This only applies to VehiclePathfinder; walking through these intersections is nothing special.
// TODO I haven't seen any cases yet with "interior" intersections. Some stuff might break.
#[derive(Clone, Serialize, Deserialize)]
pub struct IntersectionCluster {
    pub members: BTreeSet<IntersectionID>,
    pub uber_turns: Vec<UberTurn>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct UberTurn {
    pub path: Vec<TurnID>,
}

impl IntersectionCluster {
    // Based on turn restrictions
    pub fn find_all(map: &Map) -> Vec<IntersectionCluster> {
        let mut clusters = Vec::new();
        let mut graph: UnGraphMap<IntersectionID, ()> = UnGraphMap::new();
        for from in map.all_roads() {
            for (via, _) in &from.complicated_turn_restrictions {
                // Each of these tells us 2 intersections to group together
                let r = map.get_r(*via);
                graph.add_edge(r.src_i, r.dst_i, ());
            }
        }
        for intersections in petgraph::algo::kosaraju_scc(&graph) {
            let members: BTreeSet<IntersectionID> = intersections.iter().cloned().collect();
            // Discard the illegal movements
            let (ic, _) = IntersectionCluster::new(members, map);
            clusters.push(ic);
        }

        clusters
    }

    // (legal, illegal)
    pub fn new(
        members: BTreeSet<IntersectionID>,
        map: &Map,
    ) -> (IntersectionCluster, IntersectionCluster) {
        // Find all entrances and exits through this group of intersections
        let mut entrances = Vec::new();
        let mut exits = BTreeSet::new();
        for i in &members {
            for turn in map.get_turns_in_intersection(*i) {
                if turn.between_sidewalks() {
                    continue;
                }
                if !members.contains(&map.get_l(turn.id.src).src_i) {
                    entrances.push(turn.id);
                }
                if !members.contains(&map.get_l(turn.id.dst).dst_i) {
                    exits.insert(turn.id);
                }
            }
        }

        // Find all paths between entrances and exits
        let mut uber_turns = Vec::new();
        for entrance in entrances {
            uber_turns.extend(flood(entrance, map, &exits));
        }

        // Filter illegal paths
        let mut all_restrictions = Vec::new();
        for from in map.all_roads() {
            for (via, to) in &from.complicated_turn_restrictions {
                all_restrictions.push((from.id, *via, *to));
            }
        }

        // Filter out the restricted ones!
        let mut illegal = Vec::new();
        uber_turns.retain(|ut| {
            let mut ok = true;
            for pair in ut.path.windows(2) {
                let r1 = map.get_l(pair[0].src).parent;
                let r2 = map.get_l(pair[0].dst).parent;
                let r3 = map.get_l(pair[1].dst).parent;
                if all_restrictions.contains(&(r1, r2, r3)) {
                    ok = false;
                    break;
                }
            }
            if ok {
                true
            } else {
                // TODO There's surely a method in Vec to do partition like this
                illegal.push(ut.clone());
                false
            }
        });

        (
            IntersectionCluster {
                members: members.clone(),
                uber_turns,
            },
            IntersectionCluster {
                members,
                uber_turns: illegal,
            },
        )
    }
}

fn flood(start: TurnID, map: &Map, exits: &BTreeSet<TurnID>) -> Vec<UberTurn> {
    if exits.contains(&start) {
        return vec![UberTurn { path: vec![start] }];
    }

    let mut results = Vec::new();
    let mut preds: BTreeMap<TurnID, TurnID> = BTreeMap::new();
    let mut queue = vec![start];

    while !queue.is_empty() {
        let current = queue.pop().unwrap();
        for next in map.get_turns_from_lane(current.dst) {
            if preds.contains_key(&next.id) {
                continue;
            }
            preds.insert(next.id, current);
            if exits.contains(&next.id) {
                results.push(UberTurn {
                    path: trace_back(next.id, &preds),
                });
            } else {
                queue.push(next.id);
            }
        }
    }

    results
}

fn trace_back(end: TurnID, preds: &BTreeMap<TurnID, TurnID>) -> Vec<TurnID> {
    let mut path = vec![end];
    let mut current = end;
    loop {
        if let Some(prev) = preds.get(&current) {
            path.push(*prev);
            current = *prev;
        } else {
            path.reverse();
            return path;
        }
    }
}

impl UberTurn {
    pub fn entry(&self) -> LaneID {
        self.path[0].src
    }
    pub fn exit(&self) -> LaneID {
        self.path.last().unwrap().dst
    }

    pub fn geom(&self, map: &Map) -> PolyLine {
        let mut pl = map.get_t(self.path[0]).geom.clone();
        let mut first = true;
        for pair in self.path.windows(2) {
            if !first {
                pl = pl.extend(map.get_t(pair[0]).geom.clone());
                first = false;
            }
            pl = pl.extend(map.get_l(pair[0].dst).lane_center_pts.clone());
            pl = pl.extend(map.get_t(pair[1]).geom.clone());
        }
        pl
    }
}

pub struct UberTurnGroup {
    pub members: Vec<UberTurn>,
    pub geom: PolyLine,
}

impl IntersectionCluster {
    pub fn uber_turn_groups(&self, map: &Map) -> Vec<UberTurnGroup> {
        let mut groups: MultiMap<(LaneID, LaneID), usize> = MultiMap::new();
        for (idx, ut) in self.uber_turns.iter().enumerate() {
            groups.insert((ut.entry(), ut.exit()), idx);
        }

        let mut result = Vec::new();
        for (_, member_indices) in groups.consume() {
            let mut members = Vec::new();
            let mut polylines = Vec::new();
            for idx in member_indices {
                polylines.push(self.uber_turns[idx].geom(map));
                members.push(self.uber_turns[idx].clone());
            }
            result.push(UberTurnGroup {
                members,
                geom: group_geom(polylines),
            });
        }
        result
    }
}

impl UberTurnGroup {
    // TODO Share code with TurnGroup
    // Polyline points FROM intersection
    pub fn src_center_and_width(&self, map: &Map) -> (PolyLine, Distance) {
        let sample_entry = self.members[0].entry();
        let r = map.get_parent(sample_entry);
        let dir = r.is_forwards(sample_entry);
        // Points towards the intersection
        let pl = if dir {
            r.get_current_center(map)
        } else {
            r.get_current_center(map).reversed()
        };

        // TODO Poorly expressed. We just want the first leftmost value, and the last rightmost.
        let mut leftmost = Distance::meters(99999.0);
        let mut rightmost = Distance::ZERO;
        let mut left = Distance::ZERO;
        let mut right = Distance::ZERO;

        for l in r.lanes_on_side(dir) {
            right += map.get_l(l).width;

            if self.members.iter().any(|ut| ut.entry() == l) {
                leftmost = leftmost.min(left);
                rightmost = rightmost.max(right);
            }

            left += map.get_l(l).width;
        }

        let pl = map.right_shift(pl, (leftmost + rightmost) / 2.0).unwrap();
        // Flip direction, so we point away from the intersection
        (pl.reversed(), rightmost - leftmost)
    }
}

fn group_geom(mut polylines: Vec<PolyLine>) -> PolyLine {
    let num_pts = polylines[0].points().len();
    for pl in &polylines {
        if num_pts != pl.points().len() {
            return polylines.remove(0);
        }
    }

    let mut pts = Vec::new();
    for idx in 0..num_pts {
        pts.push(Pt2D::center(
            &polylines.iter().map(|pl| pl.points()[idx]).collect(),
        ));
    }
    PolyLine::new(pts)
}
