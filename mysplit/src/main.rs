#![feature(cstring_from_vec_with_nul)]
use nix::sys::uio::{self, IoVec, RemoteIoVec};
use nix::unistd::Pid;
use num_bytes::FromBytes;
use std::{ffi::CString, mem, ptr, slice, thread, time};
use sysinfo::{ProcessExt, System, SystemExt};

fn read_mem(pid: Pid, base: u64, len: u64, buf: &mut [u8]) -> nix::Result<usize> {
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
#[derive(Clone, Copy, Debug)]
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
    let save_data = lookup_class(pid, class_cache, "SaveData");
    let engine = lookup_class(pid, class_cache, "Engine");
    let level = lookup_class(pid, class_cache, "Level");
    let celeste_instance = static_field(pid, celeste_class, "Instance");
    let info_addr = locate_splitter_info(pid, celeste_instance);
    let info_size = mem::size_of::<AutoSplitterInfo>();
    let mut buf = vec![0u8; info_size];
    let mut prev_save = 0;
    let mut mode_stats = 0;
    let mut death_count: u32 = 0;
    let mut checkpoint: u32 = 0;
    loop {
        thread::sleep(time::Duration::from_millis(500));

        if read_mem(pid, info_addr, info_size as u64, &mut buf).is_err() {
            break;
        }
        let asi: AutoSplitterInfo = unsafe { ptr::read(buf.as_ptr() as *const _) };

        let current_level = if asi.level != 0 {
            read_boxed_string(pid, asi.level)
        } else {
            String::new()
        };

        let save_addr = static_field(pid, save_data, "Instance");
        if save_addr != 0 {
            if save_addr != prev_save {
                thread::sleep(time::Duration::from_secs(2));
                prev_save = save_addr;
                mode_stats = 0;
                continue;
            }
            death_count = instance_field(pid, save_addr, "TotalDeaths");
            if asi.chapter == -1 {
                mode_stats = 0;
            } else if mode_stats == 0 {
                let areas_obj: u64 = instance_field(pid, save_addr, "Areas");
                let size: u32 = instance_field(pid, areas_obj, "_size");
                let areas_arr: u64 = if size == 11 {
                    // println!("Passed");
                    instance_field(pid, areas_obj, "_items")
                } else {
                    // println!("Failed");
                    0
                };
                if areas_arr != 0 {
                    // println!("Areas arr: {:x}", areas_arr);
                    let area_stats: u64 =
                        read(pid, areas_arr + 0x20 + asi.chapter as u64 * 8);
                    // println!("Area stats: {:x}", area_stats);
                    let mode_arr =
                        instance_field::<u64, 8>(pid, area_stats, "Modes") + 0x20;
                    mode_stats = read(pid, mode_arr + asi.mode as u64 * 8);
                }
            }
            // println!("Mode stats: {:x}", mode_stats);
            if mode_stats != 0 {
                let checkpoints_obj = instance_field(pid, mode_stats, "Checkpoints");
                // println!("checkpoint obj: {:x}", checkpoints_obj);
                checkpoint = instance_field(pid, checkpoints_obj, "_count");
            }
        }

        let in_cutscene = if asi.chapter != -1 {
            if !asi.chapter_started || asi.chapter_complete {
                true
            } else {
                let scene = read(
                    pid,
                    celeste_instance + class_field_offset(pid, engine, "scene") as u64,
                );
                if instance_class(pid, scene) != level {
                    false
                } else {
                    let byte: u8 = read(
                        pid,
                        scene + class_field_offset(pid, level, "InCutscene") as u64,
                    );
                    byte != 0
                }
            }
        } else {
            false
        };

        dbg!(in_cutscene);
        dbg!(checkpoint);
        dbg!(death_count);
        dbg!(current_level);
        dbg!(asi.file_time);
    }
    Ok(())
}
