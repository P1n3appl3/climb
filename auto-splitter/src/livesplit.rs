use std::mem::{self, MaybeUninit};
use std::slice;

use bytemuck::Pod;

macro_rules! register_hooks {
    ($struct:ident) => {
        use std::cell::RefCell;
        thread_local! {static SINGLETON: RefCell<$struct> = RefCell::default()}
        pub extern "C" fn register() {
            SINGLETON.with(|s| s.replace($struct::new()));
        }
        pub extern "C" fn update() {
            SINGLETON.with(|s| s.borrow_mut().update());
        }
    };
}

#[derive(Debug)]
pub enum Error {
    FailedRead,
}
pub type Result<T> = std::result::Result<T, Error>;
pub type Address = u64;
#[derive(Debug)]
pub struct Process(u64);

impl Process {
    pub fn module(&self, name: &str) -> Option<Address> {
        unsafe {
            match ffi::get_module(self.0, name.as_ptr() as u32, name.len() as u32) {
                0 => None,
                n => Some(n),
            }
        }
    }

    pub fn read_into_buf(&self, addr: Address, buf: &mut [u8]) -> Result<()> {
        unsafe {
            (ffi::read_mem(self.0, addr, buf.as_mut_ptr() as u32, buf.len() as u32) != 0)
                .then(|| ())
                .ok_or(Error::FailedRead)
        }
    }

    pub fn read<T: Pod>(&self, addr: Address) -> Result<T> {
        unsafe {
            let mut buf = MaybeUninit::uninit();
            self.read_into_buf(
                addr,
                slice::from_raw_parts_mut(
                    buf.as_mut_ptr() as *mut u8,
                    mem::size_of::<T>(),
                ),
            )?;
            Ok(buf.assume_init())
        }
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        unsafe {
            ffi::detach(self.0);
        }
    }
}

pub trait Splitter {
    fn new() -> Self;
    fn update(&mut self);
    fn print(&self, str: &str) {
        unsafe { ffi::print_message(str.as_ptr(), str.len()) }
    }

    fn attach(&self, name: &str) -> Option<Process> {
        unsafe {
            match ffi::attach(name.as_ptr() as u32, name.len() as u32) {
                0 => None,
                n => Some(Process(n)),
            }
        }
    }

    fn start(&self) {
        unsafe { ffi::start() }
    }

    fn split(&self) {
        unsafe { ffi::split() }
    }

    fn reset(&self) {
        unsafe { ffi::reset() }
    }

    fn pause(&self) {
        unsafe { ffi::pause_game_time() }
    }

    fn unpause(&self) {
        unsafe { ffi::resume_game_time() }
    }

    fn set_game_time(&self, time: f64) {
        unsafe { ffi::set_game_time(time) }
    }

    fn set_tick_rate(&self, rate: f64) {
        unsafe { ffi::set_tick_rate(rate) }
    }

    fn state(&self) -> TimerState {
        unsafe { std::mem::transmute(ffi::get_timer_state() as u8) }
    }

    fn set_variable(&self, key: &str, value: &str) {
        unsafe {
            ffi::set_variable(
                key.as_ptr() as u32,
                key.len() as u32,
                value.as_ptr() as u32,
                value.len() as u32,
            )
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[repr(u8)]
pub enum TimerState {
    NotRunning = 0,
    Running = 1,
    Paused = 2,
    Finished = 3,
}

mod ffi {
    extern "C" {
        pub(crate) fn print_message(ptr: *const u8, len: usize);
        pub(crate) fn attach(ptr: u32, len: u32) -> u64;
        pub(crate) fn detach(handle: u64);
        pub(crate) fn get_module(handle: u64, ptr: u32, len: u32) -> u64;
        pub(crate) fn read_mem(handle: u64, address: u64, buf: u32, buf_len: u32) -> u32;
        pub(crate) fn start();
        pub(crate) fn split();
        pub(crate) fn reset();
        pub(crate) fn set_tick_rate(rate: f64);
        pub(crate) fn set_variable(key: u32, key_len: u32, value: u32, value_len: u32);
        pub(crate) fn pause_game_time();
        pub(crate) fn resume_game_time();
        pub(crate) fn set_game_time(time: f64);
        pub(crate) fn get_timer_state() -> u32;
    }
}
