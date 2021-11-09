#[macro_use]
mod livesplit;

use livesplit::TimerState;

fn init() {
    livesplit::print("start");
}

fn setup() {
    livesplit::attach("Celeste.bin.x86");
    livesplit::set_tick_rate(60.0);
    livesplit::set_variable("foo", "bar");
    livesplit::print("configured autosplitter");
}

fn tick() {
    livesplit::print("update");
    // livesplit::start();
    // livesplit::split();
    // livesplit::reset();
    // livesplit::pause();
    // livesplit::unpause();
    // livesplit::detach();
    livesplit::set_game_time(1.234);
    match livesplit::get_state() {
        TimerState::NotRunning => {}
        TimerState::Running => {}
        TimerState::Paused => {}
        TimerState::Finished => {}
    }
}

register_hooks!(init, setup, tick);
