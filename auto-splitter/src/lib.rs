// #[link(wasm_import_module = "env")] // not needed since default
#[allow(unused)]
extern "C" {
    fn print_message(ptr: *const u8, len: usize);
    fn set_process_name(ptr: *const u8, len: usize);
    pub fn push_pointer_path(
        module_ptr: *const u8,
        module_len: usize,
        kind: PointerType,
    ) -> usize;
    pub fn push_offset(pointer_path_id: usize, offset: i64);
    pub fn get_u8(pointer_path_id: usize, current: bool) -> u8;
    pub fn get_u16(pointer_path_id: usize, current: bool) -> u16;
    pub fn get_u32(pointer_path_id: usize, current: bool) -> u32;
    pub fn get_u64(pointer_path_id: usize, current: bool) -> u64;
    pub fn get_i8(pointer_path_id: usize, current: bool) -> i8;
    pub fn get_i16(pointer_path_id: usize, current: bool) -> i16;
    pub fn get_i32(pointer_path_id: usize, current: bool) -> i32;
    pub fn get_i64(pointer_path_id: usize, current: bool) -> i64;
    pub fn get_f32(pointer_path_id: usize, current: bool) -> f32;
    pub fn get_f64(pointer_path_id: usize, current: bool) -> f64;
    pub fn scan_signature(sig_ptr: *const u8, sig_len: usize) -> Address;
    pub fn set_tick_rate(rate: f64);
    pub fn read_into_buf(address: Address, buf: *mut u8, buf_len: usize) -> i32;
    pub fn set_variable(
        key_ptr: *const u8,
        key_len: usize,
        value_ptr: *const u8,
        value_len: usize,
    );
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[repr(transparent)]
pub struct Address(pub u64);

#[derive(Copy, Clone, PartialEq, Eq)]
#[repr(u8)]
pub enum PointerType {
    U8 = 0,
    U16 = 1,
    U32 = 2,
    U64 = 3,
    I8 = 4,
    I16 = 5,
    I32 = 6,
    I64 = 7,
    F32 = 8,
    F64 = 9,
    String = 10,
}

fn print(s: &str) {
    unsafe { print_message(s.as_ptr(), s.len()) }
}

fn set_name(s: &str) {
    unsafe { set_process_name(s.as_ptr(), s.len()) }
}

#[no_mangle]
pub extern "C" fn _start() {
    print("start");
}

#[no_mangle]
pub extern "C" fn configure() {
    print("configure");
    set_name("Celeste.bin.x86");
    unsafe { set_tick_rate(60.0) };
}

#[no_mangle]
pub extern "C" fn hooked() {
    print("hooked");
}
#[no_mangle]
pub extern "C" fn unhooked() {
    print("unhooked");
}
#[no_mangle]
pub extern "C" fn should_start() -> bool {
    print("should start");
    false
}
#[no_mangle]
pub extern "C" fn should_split() -> bool {
    print("should_split");
    false
}
#[no_mangle]
pub extern "C" fn should_reset() {}
#[no_mangle]
pub extern "C" fn is_loading() -> bool {
    true // always use game time
}
#[no_mangle]
pub extern "C" fn game_time() -> f64 {
    print("get_time");
    0.0
}
#[no_mangle]
pub extern "C" fn update() {}
