use std::{mem, slice};

use livesplit_wrapper::{Process, Pod};

#[allow(unused)]
#[repr(u8)]
pub enum MonoKind {
    Def = 1, // non-generic type
    Gtd,     // generic type definition
    Ginst,   // generic instantiation
    Gparam,  // generic parameter
    Array,   // vector or array, bounded or not
    Pointer, // pointer of function pointer
}

#[derive(Default, Copy, Clone)]
#[repr(C)]
struct MonoClassField {
    ty: u64,
    name: u64,
    parent: u64,
    offset: u32,
}

pub trait CelesteProcess {
    fn lookup_class(&self, cache: u64, name: &str) -> Option<u64>;
    fn class_kind(&self, class: u64) -> Option<MonoKind>;
    fn class_name(&self, class: u64) -> Option<String>;
    fn class_static_fields(&self, class: u64) -> Option<u64>;
    fn class_field_offset(&self, class: u64, name: &str) -> Option<u32>;
    fn instance_class(&self, instance: u64) -> Option<u64>;
    fn instance_field<T: Pod>(&self, instance: u64, name: &str) -> Option<T>;
    fn static_field<T: Pod>(&self, instance: u64, name: &str) -> Option<T>;
    fn locate_splitter_info(&self, instance: u64) -> Option<u64>;
    fn read_boxed_string(&self, instance: u64) -> Option<String>;
}

impl CelesteProcess for Process {
    fn lookup_class(&self, cache: u64, name: &str) -> Option<u64> {
        let cache_table: u64 = self.read(cache + 0x20).ok()?;
        let table_size: u32 = self.read(cache + 0x18).ok()?;
        for bucket in 0..table_size {
            let mut class = self.read(cache_table + 8 * bucket as u64).ok()?;
            while class != 0 {
                if self.class_name(class)? == name {
                    return Some(class);
                }
                class = self.read(class + 0xf8).ok()?;
            }
        }
        panic!("Couldn't find class: {}", name)
    }

    fn class_kind(&self, class: u64) -> Option<MonoKind> {
        Some(unsafe { mem::transmute(self.read::<u8>(class + 0x24).ok()? & 0b111) })
    }

    fn class_name(&self, class: u64) -> Option<String> {
        self.read_cstr(self.read(class + 0x40).ok()?).ok()
    }

    fn class_static_fields(&self, class: u64) -> Option<u64> {
        let vtable_size: u32 = self.read(class + 0x54).ok()?;
        let runtime_info = self.read(class + 0xc8).ok()?;
        let max_domains = self.read(runtime_info).ok()?;
        // hack: assume this class is only valid in one domain
        for i in 0..=max_domains {
            let vtable: u64 = self.read(runtime_info + 8 + 8 * i).ok()?;
            if vtable != 0 {
                return self.read(vtable + 64 + 8 * vtable_size as u64).ok();
            }
        }
        panic!("Requested class isn't loaded");
    }

    fn class_field_offset(&self, class: u64, name: &str) -> Option<u32> {
        let kind = self.class_kind(class)?;
        use MonoKind::*;
        match kind {
            Ginst => {
                return self
                    .class_field_offset(self.read(self.read(class + 0xe0).ok()?).ok()?, name)
            }
            Def | Gtd => {}
            _ => {
                panic!("Something is wrong")
            }
        };
        let num_fields: u32 = self.read(class + 0xf0).ok()?;
        let fields_addr = self.read(class + 0x90).ok()?;
        let mut fields = vec![MonoClassField::default(); num_fields as usize];
        let total_size = mem::size_of::<MonoClassField>() as u64 * fields.len() as u64;
        self.read_into_buf(fields_addr, unsafe {
            slice::from_raw_parts_mut::<u8>(fields.as_mut_ptr() as *mut u8, total_size as usize)
        })
        .ok()?;
        for field in fields {
            let temp = self.read_cstr(field.name).ok()?;
            // TODO: maybe need a check for null terminated here?
            if temp == name {
                return Some(field.offset);
            }
        }
        panic!("Tried to lookup a nonexistent field: {}", name);
    }
    fn instance_class(&self, instance: u64) -> Option<u64> {
        self.read(self.read(instance & !1).ok()?).ok()
    }

    fn instance_field<T: Pod>(&self, instance: u64, name: &str) -> Option<T> {
        let class = self.instance_class(instance)?;
        let field_offset = self.class_field_offset(class, name)?;
        self.read(instance + field_offset as u64).ok()
    }
    fn static_field<T: Pod>(&self, class: u64, name: &str) -> Option<T> {
        let static_data = self.class_static_fields(class)?;
        let field_offset = self.class_field_offset(class, name)?;
        self.read(static_data + field_offset as u64).ok()
    }

    fn locate_splitter_info(&self, instance: u64) -> Option<u64> {
        let splitter_instance: u64 = self.instance_field(instance, "AutoSplitterInfo")?;
        Some(splitter_instance + 0x10)
    }

    fn read_boxed_string(&self, instance: u64) -> Option<String> {
        let class = self.instance_class(instance)?;
        let data_offset = self.class_field_offset(class, "m_firstChar")?;
        let size_offset = self.class_field_offset(class, "m_stringLength")?;
        let size: u32 = self.read(instance + size_offset as u64).ok()?;
        let mut oversize_buf = vec![0u8; size as usize * 2];
        self.read_into_buf(instance + data_offset as u64, &mut oversize_buf)
            .ok()?;
        Some(String::from_utf16_lossy(unsafe {
            slice::from_raw_parts_mut::<u16>(oversize_buf.as_mut_ptr() as *mut u16, size as usize)
        }))
    }
}
