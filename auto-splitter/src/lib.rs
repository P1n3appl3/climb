mod process;

use std::{mem, ptr, time::Duration};

use livesplit_wrapper::{HostFunctions, Process, Splitter};
use log::*;

use process::CelesteProcess;

#[derive(Default)]
struct MySplitter {
    state: Option<Celeste>,
    old_info: Option<Info>,
    was_connected: bool,
    current_split: Split,
    failed_reads: u32,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
enum Split {
    #[default]
    Start,
    Prologue,
    City,
    Site,
    Resort,
    Ridge,
    Cassette,
    Temple,
    Reflection,
    Summit2500M,
    Summit,
    End,
}

impl Split {
    fn next(self) -> Self {
        use Split::*;
        match self {
            Start => Prologue,
            Prologue => City,
            City => Site,
            Site => Resort,
            Resort => Ridge,
            Ridge => Cassette,
            Cassette => Temple,
            Temple => Reflection,
            Reflection => Summit2500M,
            Summit2500M => Summit,
            Summit => End,
            End => End,
        }
    }
}

#[derive(Clone, Debug, Default)]
struct Info {
    asi: AutoSplitterInfo,
    death_count: u32,
    checkpoint: u32,
    in_cutscene: bool,
    room: String,
}

impl Info {
    fn file_time(&self) -> Duration {
        Duration::from_millis(self.asi.file_time as u64 / 10000)
    }

    #[allow(unused)]
    fn chapter_time(&self) -> Duration {
        Duration::from_millis(self.asi.chapter_time as u64 / 10000)
    }
}

// can't use Pod to read this because it has bools and padding bytes
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
struct AutoSplitterInfo {
    level: u64,
    /// (-1 in menus)
    chapter: i32,
    /// 0/1/2 corrresponding to A/B/C sides (-1 in menus)
    mode: i32,
    timer_active: bool,
    chapter_started: bool,
    chapter_complete: bool,
    chapter_time: i64,
    chapter_strawberries: i32,
    chapter_cassette: bool,
    chapter_heart: bool,
    file_time: i64,
    file_strawberries: i32,
    file_cassettes: i32,
    file_hearts: i32,
}

#[derive(Debug)]
struct Celeste {
    proc: Process,
    instance: u64,
    save_data_class: u64,
    engine_class: u64,
    level_class: u64,
    info: u64,
    prev_save: u64,
    mode_stats: u64,
}

impl Celeste {
    fn update(&mut self) -> Option<Info> {
        let mut buf = vec![0u8; mem::size_of::<AutoSplitterInfo>()];
        self.proc.read_into_buf(self.info, &mut buf).ok()?;
        let asi: AutoSplitterInfo = unsafe { ptr::read(buf.as_ptr() as *const _) };

        let room = if asi.level != 0 {
            self.proc.read_boxed_string(asi.level)?
        } else {
            String::new()
        };

        let mut death_count = 0;
        let mut checkpoint = 0;

        let save_addr = self.proc.static_field(self.save_data_class, "Instance")?;
        if save_addr != 0 {
            if save_addr != self.prev_save {
                self.prev_save = save_addr;
                self.mode_stats = 0;
                warn!("changed saves");
                return None;
            }
            death_count = self.proc.instance_field(save_addr, "TotalDeaths")?;
            if asi.chapter == -1 {
                self.mode_stats = 0;
            } else if self.mode_stats == 0 {
                let areas_obj = self.proc.instance_field(save_addr, "Areas")?;
                let size: u32 = self.proc.instance_field(areas_obj, "_size")?;
                let areas_arr = if size == 11 {
                    self.proc.instance_field(areas_obj, "_items")?
                } else {
                    0
                };
                if areas_arr != 0 {
                    let area_stats: u64 = self
                        .proc
                        .read(areas_arr + 0x20 + asi.chapter as u64 * 8)
                        .ok()?;
                    let mode_arr = self.proc.instance_field::<u64>(area_stats, "Modes")? + 0x20;
                    self.mode_stats = self.proc.read(mode_arr + asi.mode as u64 * 8).ok()?;
                }
            }
            if self.mode_stats != 0 {
                let checkpoints_obj = self.proc.instance_field(self.mode_stats, "Checkpoints")?;
                checkpoint = self.proc.instance_field(checkpoints_obj, "_count")?;
            }
        }

        let in_cutscene = if asi.chapter == -1 {
            false
        } else if !asi.chapter_started || asi.chapter_complete {
            true
        } else {
            let scene_field = self.proc.class_field_offset(self.engine_class, "scene")?;
            let scene = self.proc.read(self.instance + scene_field as u64).ok()?;
            if self.proc.instance_class(scene)? != self.level_class {
                false
            } else {
                let in_cutscene = self
                    .proc
                    .class_field_offset(self.level_class, "InCutscene")?;
                let bool: u8 = self.proc.read(scene + in_cutscene as u64).ok()?;
                bool != 0
            }
        };

        Some(Info {
            asi,
            death_count,
            checkpoint,
            in_cutscene,
            room,
        })
    }
}

impl MySplitter {
    fn try_init(&mut self) -> Option<()> {
        let proc = self.attach("Celeste.bin.x86")?;
        let domain_list_addr = 0xA17698;
        let domain_list = proc.read(domain_list_addr).ok()?;
        let first_domain: u64 = proc.read(domain_list).ok()?;
        let second_domain: u64 = proc.read(domain_list + 8).ok()?;
        let first_name = if first_domain != 0 {
            let strloc = proc.read(first_domain + 0xd8).ok()?;
            proc.read_cstr(strloc).ok()?
        } else {
            String::new()
        };
        if first_name != "Celeste.exe" {
            return None;
        }

        let celeste_domain = if second_domain != 0 {
            second_domain
        } else {
            first_domain
        };

        let assembly: u64 = proc.read(celeste_domain + 0xd0).ok()?;
        let image: u64 = proc.read(assembly + 0x60).ok()?;
        let class_cache = image + 1216;
        let celeste_class = proc.lookup_class(class_cache, "Celeste")?;
        let celeste_instance = proc.static_field(celeste_class, "Instance")?;
        let save_data_class = proc.lookup_class(class_cache, "SaveData")?;
        let engine_class = proc.lookup_class(class_cache, "Engine")?;
        let level_class = proc.lookup_class(class_cache, "Level")?;
        let info = proc.locate_splitter_info(celeste_instance)?;
        self.state = Some(Celeste {
            proc,
            instance: celeste_instance,
            save_data_class,
            engine_class,
            level_class,
            info,
            prev_save: 0,
            mode_stats: 0,
        });
        Some(())
    }

    fn debug_state(&self, old: &Info, new: &Info) {
        let mut diff = Vec::new();
        if old.asi.timer_active != new.asi.timer_active {
            diff.push(format!("timer={}", new.asi.timer_active));
        }
        if old.asi.chapter != new.asi.chapter {
            diff.push(format!("chapter={}", new.asi.chapter));
        }
        if old.asi.mode != new.asi.mode {
            diff.push(format!("mode={}", new.asi.mode));
        }
        if old.asi.chapter_started != new.asi.chapter_started {
            diff.push(format!("start={}", new.asi.chapter_started));
        }
        if old.asi.chapter_complete != new.asi.chapter_complete {
            diff.push(format!("finish={}", new.asi.chapter_complete));
        }
        if old.asi.file_cassettes != new.asi.file_cassettes {
            diff.push(format!("📼={}", new.asi.file_cassettes));
        }
        if old.asi.file_hearts != new.asi.file_hearts {
            diff.push(format!("💙={}", new.asi.file_hearts));
        }
        if old.in_cutscene != new.in_cutscene {
            diff.push(format!("🎬={}", new.in_cutscene));
        }
        if old.checkpoint != new.checkpoint {
            diff.push(format!("🚩={}", new.checkpoint));
        }
        if old.room != new.room {
            diff.push(format!("room='{}'", new.room));
        }
        if !diff.is_empty() {
            info!(
                "{:>04}:{:02} {:?} {}",
                new.file_time().as_secs() / 60,
                new.file_time().as_secs() % 60,
                self.current_split,
                diff.join(" ")
            );
        }
    }
}

livesplit_wrapper::register_autosplitter!(MySplitter);
impl Splitter for MySplitter {
    fn new() -> Self {
        let s = MySplitter {
            was_connected: true,
            ..Default::default()
        };
        s.set_variable("Deaths", "0");
        s.set_tick_rate(60.0);
        s
    }

    fn update(&mut self) {
        if let Some(s) = &mut self.state {
            self.was_connected = true;
            let new_info = s.update();
            match (self.old_info.clone(), new_info) {
                (Some(old), Some(new)) => {
                    // Reset trigger
                    if new.asi.chapter == 0
                        && new.room == "0"
                        && new.asi.chapter_started
                        && !old.asi.chapter_started
                        && new.file_time() < Duration::from_secs(1)
                    {
                        self.reset();
                        self.start();
                        self.current_split = Split::Prologue;
                    }

                    // Set game time and handle pausing
                    if new.asi.chapter_started {
                        self.set_game_time(new.file_time());
                        if !old.asi.chapter_started {
                            self.unpause();
                        }
                    } else if old.asi.chapter_started {
                        self.pause();
                    }

                    use Split::*;
                    let finished_chapter = old.asi.chapter_complete && !new.asi.chapter_complete;
                    if match self.current_split {
                        Start | End => false,
                        Prologue | City | Site | Resort | Ridge | Reflection | Summit => {
                            finished_chapter
                        }
                        Cassette => new.asi.file_cassettes == 1 && !new.asi.chapter_started,
                        Temple => new.asi.file_hearts == 1 && new.asi.chapter_complete,
                        Summit2500M => new.checkpoint == 6,
                    } {
                        self.split();
                        self.current_split = self.current_split.next();
                    }

                    if old.death_count != new.death_count {
                        self.set_variable("Deaths", &new.death_count.to_string())
                    }
                    self.debug_state(&old, &new);

                    self.old_info = Some(new);
                    self.failed_reads = 0;
                }
                (None, Some(new)) => self.old_info = Some(new),
                (Some(_old), None) => {
                    // After lots of failed reads: detatch, drop state, and reset because we assume
                    // the game is closed
                    warn!("failed a read, will retry next step");
                    self.failed_reads += 1;
                    if self.failed_reads == 100 {
                        error!("disconnected, will try to re-connect");
                        self.old_info = None;
                        self.state = None;
                    }
                }
                _ => {}
            }
        } else if self.try_init().is_none() && self.was_connected {
            self.was_connected = false;
            warn!("failed to connect to Celeste");
        }
    }
}
