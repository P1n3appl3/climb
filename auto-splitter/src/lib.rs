mod process;

use std::{mem, ptr};

use livesplit_wrapper::{HostFunctions, Process, Splitter};
use log::{info, warn};

use process::CelesteProcess;

#[derive(Default)]
struct MySplitter {
    state: Option<Celeste>,
    old_info: Option<Info>,
    was_connected: bool,
}

#[allow(unused)]
#[derive(Clone, Debug, Default)]
struct Info {
    asi: AutoSplitterInfo,
    death_count: u32,
    checkpoint: u32,
    in_cutscene: bool,
    current_level: String,
}

// unfortunately can't use Pod to read this because it has bools and padding
// bytes
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
struct AutoSplitterInfo {
    level: u64,
    chapter: i32,
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

        let current_level = if asi.level != 0 {
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
            current_level,
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
}

livesplit_wrapper::register_autosplitter!(MySplitter);
impl Splitter for MySplitter {
    fn new() -> Self {
        let s = MySplitter {
            was_connected: true,
            ..Default::default()
        };
        s.set_variable("deaths", "💀 0");
        s.set_variable("strawberries", "🍓 0");
        s.set_tick_rate(10.0);
        s
    }

    fn update(&mut self) {
        if let Some(s) = &mut self.state {
            self.was_connected = true;
            let new_info = s.update();
            match (self.old_info.clone(), new_info) {
                (Some(old), Some(info)) => {
                    let mut diff = vec![];
                    let _file_time = (info.asi.file_time / 10000) as f32 / 1000.0;
                    let chapter_time = (info.asi.chapter_time / 10000) as f32 / 1000.0;
                    if old.asi.timer_active != info.asi.timer_active {
                        diff.push(format!("timer_active={}", info.asi.timer_active));
                    }
                    if old.asi.chapter != info.asi.chapter {
                        diff.push(format!("chapter={}", info.asi.chapter));
                    }
                    if old.asi.mode != info.asi.mode {
                        diff.push(format!("mode={}", info.asi.mode));
                    }
                    if old.asi.chapter_started != info.asi.chapter_started {
                        diff.push(format!("start_level={}", info.asi.chapter_started));
                    }
                    if old.asi.chapter_complete != info.asi.chapter_complete {
                        diff.push(format!("end_level={}", info.asi.chapter_complete));
                    }
                    if old.asi.file_strawberries != info.asi.file_strawberries {
                        diff.push(format!("🍓={}", info.asi.file_strawberries));
                    }
                    if old.asi.file_cassettes != info.asi.file_cassettes {
                        diff.push(format!("📼={}", info.asi.file_cassettes));
                    }
                    if old.asi.file_hearts != info.asi.file_hearts {
                        diff.push(format!("💙={}", info.asi.file_hearts));
                    }
                    if old.death_count != info.death_count {
                        diff.push(format!("💀={}", info.death_count));
                    }
                    if old.in_cutscene != info.in_cutscene {
                        diff.push(format!("🎬={}", info.in_cutscene));
                    }
                    if old.checkpoint != info.checkpoint {
                        diff.push(format!("🚩={}", info.checkpoint));
                    }
                    if old.current_level != info.current_level {
                        diff.push(format!("room='{}'", info.current_level));
                    }
                    if !diff.is_empty() {
                        info!("{} : {}", chapter_time, diff.join(" "));
                    }
                    self.old_info = Some(info);
                }
                (None, Some(new)) => self.old_info = Some(new),
                (Some(_old), None) => info!("failed a read, will retry next step"),
                _ => {}
            }
        } else if self.try_init().is_none() && self.was_connected {
            self.was_connected = false;
            warn!("failed to connect to Celeste");
        }
    }
}
