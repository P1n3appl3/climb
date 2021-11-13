use livesplit_wrapper::{HostFunctions, Process, Splitter, TimerState};

#[derive(Default)]
struct MySplitter {
    process: Option<Process>,
}

livesplit_wrapper::register_autosplitter!(MySplitter);
impl Splitter for MySplitter {
    fn new() -> Self {
        let mut s = MySplitter::default();
        s.process = s.attach("CoolGame.exe");
        if s.process.is_none() {
            s.print("failed to connect to process, is the game running?");
        }
        s.set_tick_rate(120.0);
        s.set_variable("items collected", "0");
        s
    }

    fn update(&mut self) {
        if let Some(p) = &self.process {
            match (self.state(), p.read(0xD1DAC71C)) {
                (TimerState::Paused, Ok(314i16)) => self.unpause(),
                (TimerState::Running, Ok(42i16)) => self.pause(),
                _ => {}
            }
        }
    }
}
