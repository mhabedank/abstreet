mod create_gridlock;
mod faster_trips;
mod freeform;
mod optimize_bus;
mod play_scenario;

use crate::game::Transition;
use crate::render::AgentColorScheme;
use crate::sandbox::overlays::Overlays;
use crate::sandbox::SandboxMode;
use crate::ui::UI;
use abstutil::{prettyprint_usize, Timer};
use ezgui::{Color, EventCtx, GfxCtx, Line, ModalMenu, TextSpan, Wizard};
use geom::Duration;
use sim::{Analytics, Scenario, TripMode};

pub struct GameplayRunner {
    pub mode: GameplayMode,
    pub menu: ModalMenu,
    state: State,
    prebaked: Analytics,
}

#[derive(Clone)]
pub enum GameplayMode {
    // TODO Maybe this should be "sandbox"
    Freeform,
    PlayScenario(String),
    // Route name
    OptimizeBus(String),
    CreateGridlock,
    // TODO Be able to filter population by more factors
    FasterTrips(TripMode),
}

pub enum State {
    Freeform(freeform::Freeform),
    PlayScenario(play_scenario::PlayScenario),
    OptimizeBus(optimize_bus::OptimizeBus),
    CreateGridlock(create_gridlock::CreateGridlock),
    FasterTrips(faster_trips::FasterTrips),
}

impl GameplayRunner {
    pub fn initialize(mode: GameplayMode, ui: &mut UI, ctx: &mut EventCtx) -> GameplayRunner {
        let prebaked: Analytics = abstutil::read_binary(
            &abstutil::path_prebaked_results(ui.primary.map.get_name()),
            &mut Timer::throwaway(),
        )
        .unwrap_or_else(|_| {
            println!("WARNING! No prebaked sim analytics. Only freeform mode will work.");
            Analytics::new()
        });

        let ((menu, state), maybe_scenario) = match mode.clone() {
            GameplayMode::Freeform => (freeform::Freeform::new(ctx), None),
            GameplayMode::PlayScenario(scenario) => (
                play_scenario::PlayScenario::new(&scenario, ctx),
                Some(scenario),
            ),
            GameplayMode::OptimizeBus(route_name) => (
                optimize_bus::OptimizeBus::new(route_name, ctx, ui),
                Some("weekday_typical_traffic_from_psrc".to_string()),
            ),
            GameplayMode::CreateGridlock => (
                create_gridlock::CreateGridlock::new(ctx),
                Some("weekday_typical_traffic_from_psrc".to_string()),
            ),
            GameplayMode::FasterTrips(trip_mode) => (
                faster_trips::FasterTrips::new(trip_mode, ctx),
                Some("weekday_typical_traffic_from_psrc".to_string()),
            ),
        };
        let runner = GameplayRunner {
            mode,
            menu: menu.disable_standalone_layout(),
            state,
            prebaked,
        };
        if let Some(scenario_name) = maybe_scenario {
            ctx.loading_screen("instantiate scenario", |_, timer| {
                let num_agents = ui.primary.current_flags.num_agents;
                let builtin = if let Some(n) = num_agents {
                    format!("random scenario with {} agents", n)
                } else {
                    "random scenario with some agents".to_string()
                };
                let scenario = if scenario_name == builtin {
                    if let Some(n) = num_agents {
                        Scenario::scaled_run(&ui.primary.map, n)
                    } else {
                        Scenario::small_run(&ui.primary.map)
                    }
                } else if scenario_name == "just buses" {
                    let mut s = Scenario::empty(&ui.primary.map);
                    s.scenario_name = "just buses".to_string();
                    s.seed_buses = true;
                    s
                } else {
                    abstutil::read_binary(
                        &abstutil::path1_bin(
                            &ui.primary.map.get_name(),
                            abstutil::SCENARIOS,
                            &scenario_name,
                        ),
                        timer,
                    )
                    .unwrap()
                };
                scenario.instantiate(
                    &mut ui.primary.sim,
                    &ui.primary.map,
                    &mut ui.primary.current_flags.sim_flags.make_rng(),
                    timer,
                );
                ui.primary.sim.step(&ui.primary.map, Duration::seconds(0.1));
            });
        }
        runner
    }

    pub fn event(
        &mut self,
        ctx: &mut EventCtx,
        ui: &mut UI,
        overlays: &mut Overlays,
    ) -> Option<Transition> {
        match self.state {
            State::Freeform(ref mut f) => {
                if let Some(t) = f.event(ctx, ui, &mut self.menu) {
                    return Some(t);
                }
            }
            State::PlayScenario(ref mut p) => {
                if let Some(t) = p.event(ctx, ui, &mut self.menu) {
                    return Some(t);
                }
            }
            State::OptimizeBus(ref mut o) => {
                if let Some(t) = o.event(ctx, ui, overlays, &mut self.menu, &self.prebaked) {
                    return Some(t);
                }
            }
            State::CreateGridlock(ref mut g) => {
                if let Some(t) = g.event(ctx, ui, &mut self.menu, &self.prebaked) {
                    return Some(t);
                }
            }
            State::FasterTrips(ref mut f) => {
                if let Some(t) = f.event(ctx, ui, &mut self.menu, &self.prebaked) {
                    return Some(t);
                }
            }
        }
        None
    }

    pub fn draw(&self, g: &mut GfxCtx) {
        self.menu.draw(g);
    }
}

fn change_scenario(wiz: &mut Wizard, ctx: &mut EventCtx, ui: &mut UI) -> Option<Transition> {
    let num_agents = ui.primary.current_flags.num_agents;
    let builtin = if let Some(n) = num_agents {
        format!("random scenario with {} agents", n)
    } else {
        "random scenario with some agents".to_string()
    };
    let scenario_name = wiz
        .wrap(ctx)
        .choose_string("Instantiate which scenario?", || {
            let mut list =
                abstutil::list_all_objects(abstutil::SCENARIOS, ui.primary.map.get_name());
            list.push(builtin.clone());
            list.push("just buses".to_string());
            list
        })?;
    Some(Transition::PopThenReplace(Box::new(SandboxMode::new(
        ctx,
        ui,
        GameplayMode::PlayScenario(scenario_name),
    ))))
}

fn load_map(wiz: &mut Wizard, ctx: &mut EventCtx, ui: &mut UI) -> Option<Transition> {
    if let Some(name) = wiz.wrap(ctx).choose_string("Load which map?", || {
        let current_map = ui.primary.map.get_name();
        abstutil::list_all_objects("maps", "")
            .into_iter()
            .filter(|n| n != current_map)
            .collect()
    }) {
        ctx.canvas.save_camera_state(ui.primary.map.get_name());
        let mut flags = ui.primary.current_flags.clone();
        flags.sim_flags.load = abstutil::path_map(&name);
        *ui = UI::new(flags, ctx, false);
        Some(Transition::PopThenReplace(Box::new(SandboxMode::new(
            ctx,
            ui,
            // TODO If we were playing a scenario, load that one...
            GameplayMode::Freeform,
        ))))
    } else if wiz.aborted() {
        Some(Transition::Pop)
    } else {
        None
    }
}

// Must call menu.event first. Returns true if the caller should set the overlay to the custom
// thing.
fn manage_overlays(
    menu: &mut ModalMenu,
    ctx: &mut EventCtx,
    show: &str,
    hide: &str,
    overlay: &mut Overlays,
    active_originally: bool,
    time_changed: bool,
) -> bool {
    // Synchronize menus if needed. Player can change these separately.
    if active_originally {
        menu.maybe_change_action(show, hide, ctx);
    } else {
        menu.maybe_change_action(hide, show, ctx);
    }

    if !active_originally && menu.swap_action(show, hide, ctx) {
        true
    } else if active_originally && menu.swap_action(hide, show, ctx) {
        *overlay = Overlays::Inactive;
        false
    } else {
        active_originally && time_changed
    }
}

// Must call menu.event first.
fn manage_acs(
    menu: &mut ModalMenu,
    ctx: &mut EventCtx,
    ui: &mut UI,
    show: &str,
    hide: &str,
    acs: AgentColorScheme,
) {
    let active_originally = ui.agent_cs == acs;

    // Synchronize menus if needed. Player can change these separately.
    if active_originally {
        menu.maybe_change_action(show, hide, ctx);
    } else {
        menu.maybe_change_action(hide, show, ctx);
    }

    if !active_originally && menu.swap_action(show, hide, ctx) {
        ui.agent_cs = acs;
    } else if active_originally && menu.swap_action(hide, show, ctx) {
        ui.agent_cs = AgentColorScheme::VehicleTypes;
    }
}

// Shorter is better
fn cmp_duration_shorter(now: Duration, baseline: Duration) -> Vec<TextSpan> {
    if now.epsilon_eq(baseline) {
        vec![Line(" (same as baseline)")]
    } else if now < baseline {
        vec![
            Line(" ("),
            Line((baseline - now).minimal_tostring()).fg(Color::GREEN),
            Line(" faster)"),
        ]
    } else if now > baseline {
        vec![
            Line(" ("),
            Line((now - baseline).minimal_tostring()).fg(Color::RED),
            Line(" slower)"),
        ]
    } else {
        unreachable!()
    }
}

// Fewer is better
fn cmp_count_fewer(now: usize, baseline: usize) -> TextSpan {
    if now < baseline {
        Line(format!("{} fewer", prettyprint_usize(baseline - now))).fg(Color::GREEN)
    } else if now > baseline {
        Line(format!("{} more", prettyprint_usize(now - baseline))).fg(Color::RED)
    } else {
        Line("same as baseline")
    }
}

// More is better
fn cmp_count_more(now: usize, baseline: usize) -> TextSpan {
    if now < baseline {
        Line(format!("{} fewer", prettyprint_usize(baseline - now))).fg(Color::RED)
    } else if now > baseline {
        Line(format!("{} more", prettyprint_usize(now - baseline))).fg(Color::GREEN)
    } else {
        Line("same as baseline")
    }
}
