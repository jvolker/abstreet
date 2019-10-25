use crate::common::{RoadColorer, RoadColorerBuilder};
use crate::game::{State, Transition};
use crate::helpers::ID;
use crate::ui::UI;
use ezgui::{hotkey, Color, EventCtx, GfxCtx, Key, Line, ModalMenu, Text};
use map_model::{LaneID, LaneType, Map};
use petgraph::graphmap::DiGraphMap;
use std::collections::HashSet;

pub struct Floodfiller {
    menu: ModalMenu,
    colorer: RoadColorer,
}

impl Floodfiller {
    pub fn new(ctx: &mut EventCtx, ui: &UI, parent_menu: &mut ModalMenu) -> Option<Box<dyn State>> {
        let map = &ui.primary.map;
        let (reachable_lanes, is_sidewalk, title) =
            if let Some(ID::Lane(l)) = ui.primary.current_selection {
                if !map.get_l(l).is_parking()
                    && map.get_l(l).lane_type != LaneType::SharedLeftTurn
                    && ctx
                        .input
                        .contextual_action(Key::F, "floodfill from this lane")
                {
                    (
                        find_reachable_from(l, map),
                        map.get_l(l).is_sidewalk(),
                        format!("Floodfiller from {}", l),
                    )
                } else {
                    return None;
                }
            } else if parent_menu.action("show strongly-connected component roads") {
                let mut graph = DiGraphMap::new();
                for turn in map.all_turns().values() {
                    if map.is_turn_allowed(turn.id) && !turn.between_sidewalks() {
                        graph.add_edge(turn.id.src, turn.id.dst, 1);
                    }
                }
                let components = petgraph::algo::kosaraju_scc(&graph);
                (
                    components
                        .into_iter()
                        .max_by_key(|c| c.len())
                        .unwrap()
                        .into_iter()
                        .collect(),
                    false,
                    "Strongy-connected component".to_string(),
                )
            } else {
                return None;
            };

        let reachable_color = ui.cs.get_def("reachable lane", Color::GREEN);
        let unreachable_color = ui.cs.get_def("unreachable lane", Color::RED);

        let mut colorer = RoadColorerBuilder::new(
            Text::prompt("lane connectivity"),
            vec![
                ("unreachable", unreachable_color),
                ("reachable", reachable_color),
            ],
        );
        let mut num_unreachable = 0;
        for lane in map.all_lanes() {
            if lane.is_parking() || lane.lane_type == LaneType::SharedLeftTurn {
                continue;
            }
            if is_sidewalk != lane.is_sidewalk() {
                continue;
            }
            colorer.add(
                lane.id,
                if reachable_lanes.contains(&lane.id) {
                    reachable_color
                } else {
                    num_unreachable += 1;
                    println!("{} is unreachable", lane.id);
                    unreachable_color
                },
                map,
            );
        }

        let mut menu = ModalMenu::new(title, vec![vec![(hotkey(Key::Escape), "quit")]], ctx);
        menu.set_info(
            ctx,
            Text::from(Line(format!("{} unreachable lanes", num_unreachable))),
        );

        Some(Box::new(Floodfiller {
            menu,
            colorer: colorer.build(ctx, map),
        }))
    }
}

impl State for Floodfiller {
    fn event(&mut self, ctx: &mut EventCtx, ui: &mut UI) -> Transition {
        if ctx.redo_mouseover() {
            ui.recalculate_current_selection(ctx);
        }
        ctx.canvas.handle_event(ctx.input);

        self.menu.event(ctx);
        if self.menu.action("quit") {
            return Transition::Pop;
        }

        Transition::Keep
    }

    fn draw_default_ui(&self) -> bool {
        false
    }

    fn draw(&self, g: &mut GfxCtx, ui: &UI) {
        self.colorer.draw(g, ui);
        self.menu.draw(g);
    }
}

fn find_reachable_from(start: LaneID, map: &Map) -> HashSet<LaneID> {
    let mut visited = HashSet::new();
    let mut queue = vec![start];
    while !queue.is_empty() {
        let current = queue.pop().unwrap();
        if visited.contains(&current) {
            continue;
        }
        visited.insert(current);
        for turn in map.get_turns_from_lane(current) {
            if map.is_turn_allowed(turn.id) && !visited.contains(&turn.id.dst) {
                queue.push(turn.id.dst);
            }
        }
    }
    visited
}
