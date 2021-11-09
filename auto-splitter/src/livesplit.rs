#![allow(unused)]
// TODO: check if no_mangle is needed
macro_rules! register_hooks {
    ($start:ident, $register:ident, $update:ident) => {
        #[no_mangle]
        pub extern "C" fn _start() {
            $start()
        }
        #[no_mangle]
        pub extern "C" fn register() {
            $register()
        }
        #[no_mangle]
        pub extern "C" fn update() {
            $update()
        }
    };
}

mod ffi {
    #[repr(u8)]
    #[derive(Debug, Copy, Clone, PartialEq, Eq)]
    pub enum TimerState {
        NotRunning = 0,
        Running = 1,
        Paused = 2,
        Finished = 3,
    }
    // #[link(wasm_import_module = "env")] // not needed since default
    extern "C" {
        pub(crate) fn print_message(ptr: *const u8, len: usize);
        pub(crate) fn attach(ptr: *const u8, len: usize) -> i64;
        pub(crate) fn detach();
        pub(crate) fn start();
        pub(crate) fn split();
        pub(crate) fn reset();
        pub(crate) fn set_tick_rate(rate: f64);
        pub(crate) fn read_into_buf(address: u64, buf: *mut u8, buf_len: usize) -> i32;
        pub(crate) fn set_variable(
            key_ptr: *const u8,
            key_len: usize,
            value_ptr: *const u8,
            value_len: usize,
        );
        pub(crate) fn pause_game_time();
        pub(crate) fn resume_game_time();
        pub(crate) fn set_game_time(time: f64);
        pub(crate) fn get_timer_state() -> TimerState;
    }
}

pub use ffi::TimerState;

pub fn print(str: &str) {
    unsafe {
        ffi::print_message(str.as_ptr(), str.len());
    }
}

pub fn attach(name: &str) {
    unsafe {
        ffi::attach(name.as_ptr(), name.len());
    }
}

pub fn detach() {
    unsafe {
        ffi::detach();
    }
}

pub fn start() {
    unsafe {
        ffi::detach();
    }
}

pub fn split() {
    unsafe {
        ffi::detach();
    }
}

pub fn reset() {
    unsafe {
        ffi::detach();
    }
}

pub fn pause() {
    unsafe {
        ffi::pause_game_time();
    }
}

pub fn unpause() {
    unsafe {
        ffi::resume_game_time();
    }
}

pub fn set_game_time(time: f64) {
    unsafe {
        ffi::set_game_time(time);
    }
}

pub fn set_tick_rate(rate: f64) {
    unsafe {
        ffi::set_tick_rate(rate);
    }
}

pub fn read(addr: u64, buf: &mut [u8]) {
    unsafe {
        ffi::read_into_buf(addr, buf.as_mut_ptr(), buf.len());
    }
}

pub fn get_state() -> ffi::TimerState {
    unsafe { ffi::get_timer_state() }
}

pub fn set_variable(key: &str, value: &str) {
    unsafe {
        ffi::set_variable(key.as_ptr(), key.len(), value.as_ptr(), value.len());
    }
}
