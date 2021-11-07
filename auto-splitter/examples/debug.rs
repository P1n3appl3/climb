#![feature(cstring_from_vec_with_nul)]
use nix::sys::uio::{self, IoVec, RemoteIoVec};
use nix::unistd::Pid;
use num_bytes::FromBytes;
use std::borrow::BorrowMut;
use std::cell::RefCell;
use std::collections::HashSet;
use std::{ffi::CString, mem, ptr, slice, thread, time};
use sysinfo::{ProcessExt, System, SystemExt};

thread_local! {
    static ACCESS: RefCell<HashSet<u64>> = RefCell::new(HashSet::new());
    static TOTAL: RefCell<u64> = RefCell::new(0);
}

fn read_mem(pid: Pid, base: u64, len: u64, buf: &mut [u8]) -> nix::Result<usize> {
    // println!("0x{:X} : {}", base, len);
    let page = base % 4096;
    TOTAL.with(|t| *t.borrow_mut() += 1);
    ACCESS.with(|a| a.borrow_mut().insert(page));
    let local = IoVec::from_mut_slice(buf);
    let remote = RemoteIoVec {
        base: base as usize,
        len: len as usize,
    };
    uio::process_vm_readv(pid, &[local], &[remote])
}

pub fn read<T: FromBytes<LEN>, const LEN: usize>(pid: Pid, base: u64) -> T {
    let mut buf = [0; LEN];
    read_mem(pid, base, 8, &mut buf).unwrap();
    FromBytes::from_le_bytes(buf)
}

fn read_string(pid: Pid, base: u64) -> String {
    const MAX_STR_LEN: usize = 256;
    let mut buf = vec![0u8; MAX_STR_LEN];
    read_mem(pid, base, MAX_STR_LEN as u64 - 1, &mut buf).unwrap();
    buf.truncate(buf.iter().position(|&x| x == 0).unwrap() + 1);
    let cstr = CString::from_vec_with_nul(buf).unwrap();
    cstr.to_string_lossy().to_string()
}

fn class_name(pid: Pid, class: u64) -> String {
    read_string(pid, read(pid, class + 0x40))
}

fn class_kind(pid: Pid, class: u64) -> MonoKind {
    unsafe { mem::transmute(read::<u8, 1>(pid, class + 0x24) & 0b111) }
}

fn lookup_class(pid: Pid, cache: u64, name: &str) -> u64 {
    let cache_table: u64 = read(pid, cache + 0x20);
    let table_size: u32 = read(pid, cache + 0x18);
    // println!("Searching for class {}", name);
    // println!("Table size: {}, cache_table: {}", table_size, cache_table);
    for bucket in 0..table_size {
        let mut class = read(pid, cache_table + 8 * bucket as u64);
        while class != 0 {
            // println!("{:x} {:?}", class, class_name(pid, class));
            if class_name(pid, class) == name {
                return class;
            }
            class = read(pid, class + 0xf8);
        }
    }
    panic!("Couldn't find class: {}", name)
}

fn class_static_fields(pid: Pid, class: u64) -> u64 {
    let vtable_size: u32 = read(pid, class + 0x54);
    let runtime_info = read(pid, class + 0xc8);
    let max_domains = read(pid, runtime_info);
    // hack: assume this class is only valid in one domain
    for i in 0..=max_domains {
        let vtable: u64 = read(pid, runtime_info + 8 + 8 * i);
        if vtable != 0 {
            return read(pid, vtable + 64 + 8 * vtable_size as u64);
        }
    }
    panic!("Requested class isn't loaded");
}

#[allow(unused)]
#[repr(u8)]
enum MonoKind {
    MonoClassDef = 1, // non-generic type
    MonoClassGtd,     // generic type definition
    MonoClassGinst,   // generic instantiation
    MonoClassGparam,  // generic parameter
    MonoClassArray,   // vector or array, bounded or not
    MonoClassPointer, // pointer of function pointer
}

#[derive(Default, Copy, Clone)]
#[repr(C)]
struct MonoClassField {
    ty: u64,
    name: u64,
    parent: u64,
    offset: u32,
}

fn class_field_offset(pid: Pid, class: u64, name: &str) -> u32 {
    let kind = class_kind(pid, class);
    use MonoKind::*;
    match kind {
        MonoClassGinst => {
            return class_field_offset(pid, read(pid, read(pid, class + 0xe0)), name);
        }
        MonoClassDef | MonoClassGtd => {}
        _ => {
            panic!("Something is wrong")
        }
    };
    let num_fields: u32 = read(pid, class + 0xf0);
    let fields_addr = read(pid, class + 0x90);
    let mut fields = vec![MonoClassField::default(); num_fields as usize];
    let total_size = mem::size_of::<MonoClassField>() as u64 * fields.len() as u64;
    read_mem(pid, fields_addr, total_size, unsafe {
        slice::from_raw_parts_mut::<u8>(
            fields.as_mut_ptr() as *mut u8,
            total_size as usize,
        )
    })
    .unwrap();
    for field in fields {
        let temp = read_string(pid, field.name);
        // TODO: maybe need a check for null terminated here?
        if temp == name {
            return field.offset;
        }
    }
    panic!("Tried to lookup a nonexistent field: {}", name);
}

fn instance_class(pid: Pid, instance: u64) -> u64 {
    read(pid, read(pid, instance & !1))
}

fn instance_field<T: FromBytes<LEN>, const LEN: usize>(
    pid: Pid,
    instance: u64,
    name: &str,
) -> T {
    let class = instance_class(pid, instance);
    let field_offset = class_field_offset(pid, class, name);
    read::<T, LEN>(pid, instance + field_offset as u64)
}
fn static_field<T: FromBytes<LEN>, const LEN: usize>(
    pid: Pid,
    class: u64,
    name: &str,
) -> T {
    let static_data = class_static_fields(pid, class);
    let field_offset = class_field_offset(pid, class, name);
    read::<T, LEN>(pid, static_data + field_offset as u64)
}

fn locate_splitter_info(pid: Pid, instance: u64) -> u64 {
    let splitter_instance: u64 = instance_field(pid, instance, "AutoSplitterInfo");
    splitter_instance + 0x10
}

fn read_boxed_string(pid: Pid, instance: u64) -> String {
    let class = instance_class(pid, instance);
    let data_offset = class_field_offset(pid, class, "m_firstChar");
    let size_offset = class_field_offset(pid, class, "m_stringLength");
    let size: u32 = read(pid, instance + size_offset as u64);
    let mut oversize_buf = vec![0u8; size as usize * 2];
    read_mem(
        pid,
        instance + data_offset as u64,
        size as u64 * 2,
        &mut oversize_buf,
    )
    .unwrap();
    String::from_utf16_lossy(unsafe {
        slice::from_raw_parts_mut::<u16>(
            oversize_buf.as_mut_ptr() as *mut u16,
            size as usize,
        )
    })
}

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

#[derive(Copy, Clone, Debug)]
struct Celeste {
    pid: Pid,
    instance: u64,
    save_data_class: u64,
    engine_class: u64,
    level_class: u64,
    info: u64,
    prev_save: u64,
    mode_stats: u64,
}

impl Celeste {
    fn load_from_process(pid: Pid) -> Result<Self, &'static str> {
        // let root_domain_addr = 0xA17650;
        // let root_domain = read(pid, root_domain_addr);
        let domain_list_addr = 0xA17698;
        let domain_list = read(pid, domain_list_addr);
        let first_domain: u64 = read(pid, domain_list);
        let second_domain: u64 = read(pid, domain_list + 8);
        let first_name = if first_domain != 0 {
            read_string(pid, read(pid, first_domain + 0xd8))
        } else {
            String::new()
        };
        if first_name != "Celeste.exe" {
            return Err("This is not Celeste!");
        }
        let celeste_domain = if second_domain != 0 {
            // let second_name = read_string(pid, read(pid, first_domain + 0xd8));
            // println!("Connected to: {}", second_name);
            second_domain
        } else {
            // println!("Connected to: {}", first_name);
            first_domain
        };

        let assembly: u64 = read(pid, celeste_domain + 0xd0);
        let image: u64 = read(pid, assembly + 0x60);
        let class_cache = image + 1216;
        let celeste_class = lookup_class(pid, class_cache, "Celeste");
        let celeste_instance = static_field(pid, celeste_class, "Instance");
        Ok(Celeste {
            pid,
            instance: celeste_instance,
            save_data_class: lookup_class(pid, class_cache, "SaveData"),
            engine_class: lookup_class(pid, class_cache, "Engine"),
            level_class: lookup_class(pid, class_cache, "Level"),
            info: locate_splitter_info(pid, celeste_instance),
            prev_save: 0,
            mode_stats: 0,
        })
    }

    fn update(&mut self) -> nix::Result<Info> {
        let info_size = mem::size_of::<AutoSplitterInfo>();
        let mut buf = vec![0u8; info_size];
        read_mem(self.pid, self.info, info_size as u64, &mut buf)?;
        let asi: AutoSplitterInfo = unsafe { ptr::read(buf.as_ptr() as *const _) };

        let current_level = if asi.level != 0 {
            read_boxed_string(self.pid, asi.level)
        } else {
            String::new()
        };

        let mut death_count = 0;
        let mut checkpoint = 0;

        let save_addr = static_field(self.pid, self.save_data_class, "Instance");
        if save_addr != 0 {
            if save_addr != self.prev_save {
                thread::sleep(time::Duration::from_secs(1));
                self.prev_save = save_addr;
                self.mode_stats = 0;
                return self.update();
            }
            death_count = instance_field(self.pid, save_addr, "TotalDeaths");
            if asi.chapter == -1 {
                self.mode_stats = 0;
            } else if self.mode_stats == 0 {
                let areas_obj: u64 = instance_field(self.pid, save_addr, "Areas");
                let size: u32 = instance_field(self.pid, areas_obj, "_size");
                let areas_arr: u64 = if size == 11 {
                    // println!("Passed");
                    instance_field(self.pid, areas_obj, "_items")
                } else {
                    // println!("Failed");
                    0
                };
                if areas_arr != 0 {
                    // println!("Areas arr: {:x}", areas_arr);
                    let area_stats: u64 =
                        read(self.pid, areas_arr + 0x20 + asi.chapter as u64 * 8);
                    // println!("Area stats: {:x}", area_stats);
                    let mode_arr =
                        instance_field::<u64, 8>(self.pid, area_stats, "Modes") + 0x20;
                    self.mode_stats = read(self.pid, mode_arr + asi.mode as u64 * 8);
                }
            }
            // println!("Mode stats: {:x}", self.mode_stats);
            if self.mode_stats != 0 {
                let checkpoints_obj =
                    instance_field(self.pid, self.mode_stats, "Checkpoints");
                // println!("checkpoint obj: {:x}", checkpoints_obj);
                checkpoint = instance_field(self.pid, checkpoints_obj, "_count");
            }
        }

        let in_cutscene = if asi.chapter != -1 {
            if !asi.chapter_started || asi.chapter_complete {
                true
            } else {
                let scene = read(
                    self.pid,
                    self.instance
                        + class_field_offset(self.pid, self.engine_class, "scene") as u64,
                );
                if instance_class(self.pid, scene) != self.level_class {
                    false
                } else {
                    let byte: u8 = read(
                        self.pid,
                        scene
                            + class_field_offset(self.pid, self.level_class, "InCutscene")
                                as u64,
                    );
                    byte != 0
                }
            }
        } else {
            false
        };

        Ok(Info {
            asi,
            death_count,
            checkpoint,
            in_cutscene,
            current_level,
        })
    }
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

fn main() -> Result<(), &'static str> {
    let s = System::new_all();
    let candidates = s.process_by_name("Celeste.bin.x86");
    let pid = Pid::from_raw(
        match candidates[..] {
            [] => Err("Couldn't find Celeste process"),
            [p] => Ok(p),
            [_, _, ..] => Err("Found more than one Celeste process"),
        }?
        .pid(),
    );
    println!("Found celeste process: {}", pid);

    let mut celeste = Celeste::load_from_process(pid)?;
    loop {
        thread::sleep(time::Duration::from_millis(100));
        let _info = celeste.update().unwrap();
        let mut total = 0;
        let mut count = 0;
        TOTAL.with(|t| {
            total = *t.borrow();
            *t.borrow_mut() = 0
        });
        ACCESS.with(|a| {
            count = a.borrow().len();
            a.borrow_mut().clear()
        });
        println!("{} / {}", count, total);
        // dbg!(info);
    }
}
