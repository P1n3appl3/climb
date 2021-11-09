#[macro_use]
mod livesplit;

use livesplit::{Process, Splitter, TimerState};

#[derive(Debug, Default)]
struct MySplitter {
    process: Option<Process>,
}

register_hooks!(MySplitter);
impl Splitter for MySplitter {
    fn new() -> Self {
        let mut splitter = MySplitter::default();
        splitter.process = splitter.attach("Celeste.bin.x86");
        splitter.print("splitter initialized");
        splitter.set_tick_rate(120.0);
        splitter.set_variable("deaths", "9999");
        splitter.print("configured autosplitter");
        splitter
    }

    fn update(&mut self) {
        self.print("update");

        self.start();
        self.split();
        self.reset();
        self.pause();
        self.unpause();

        self.set_game_time(1.234);

        let mut buf = vec![0u8; 22];
        if let Some(p) = &self.process {
            if let Some(addr) = p.module("some_mod") {
                if p.read_into_buf(0xdeadbeef, &mut buf[..5]).is_err() {
                    self.print("Failed to read addr")
                }
                dbg!(p.read::<i128>(addr).ok());
            }
        }

        match self.state() {
            TimerState::NotRunning => {}
            TimerState::Running => {}
            TimerState::Paused => {}
            TimerState::Finished => {}
        }
    }
}
