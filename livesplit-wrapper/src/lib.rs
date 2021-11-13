#![deny(missing_docs)]
/*!
A safe wrapper of the [livesplit-core](https://github.com/LiveSplit/livesplit-core) api for creating autosplitters.

LiveSplit One uses dynamically loaded WASM modules for plugins, which means they have to communicate over a C interface. This crate contains ergonomic wrappers to write those plugins in safe rust. At the moment only autosplitters are supported, but there will eventually be support for more general plugins.

# Implementing an autosplitter

To write an autosplitter you need to implement the [`Splitter`] trait and invoke the [`register_autosplitter!`] macro on your splitter.

Here's a full working (albeit nonsensical) example. Build this with `--target wasm32-unknown-unknown` and it can be loaded by frontends such as [`livesplit-one-desktop`](https://github.com/CryZe/livesplit-one-desktop) or [`obs-livesplit-one`](https://github.com/CryZe/obs-livesplit-one):
```
use livesplit_wrapper::{Splitter, Process, TimerState, HostFunctions};

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
            match (self.state(), p.read::<i16>(0xD1DAC71C)) {
                (TimerState::Paused, Ok(314)) => self.unpause(),
                (TimerState::Running, Ok(42)) => self.pause(),
                _ => {}
            }
        }
    }
}
```
*/
// TODO: add link once livesplit-core provides a local debug runtime

use std::mem::{self, MaybeUninit};
use std::slice;

use bytemuck::Pod;

/// Wires up the necessary c interface for a type that implements [`Splitter`].
///
/// If you defined `struct MySplitter {...}` and `impl Splitter for MySplitter {...}` then
/// just write `register_autosplitter!(MySplitter);` and you'll be good to go.
#[macro_export]
macro_rules! register_autosplitter {
    ($struct:ident) => {
        use std::cell::RefCell;
        thread_local! {static SINGLETON: RefCell<$struct> = RefCell::default()}
        pub extern "C" fn configure() {
            SINGLETON.with(|s| s.replace($struct::new()));
        }
        pub extern "C" fn update() {
            SINGLETON.with(|s| s.borrow_mut().update());
        }
    };
}

/// Currently the only possible error is a failed memory read on the attached process.
#[derive(Debug)]
pub enum Error {
    /// A memory read on the attached process failed
    FailedRead,
}

type Result<T> = std::result::Result<T, Error>;
/// An address in the attached processes memory.
///
/// Autosplitters can attach to 32-bit processes, they'll just get an error if they try to
/// read outside it's address space.
pub type Address = u64;

/// A handle representing an attached process that can be used to read its memory.
#[derive(Debug)]
pub struct Process(u64);

impl Process {
    /// Reads a single value from the attached processes memory space. To be able to use
    /// this with your own types, they need to implement [`Pod`] (it's implemented for the
    /// numeric types and fixed size arrays by default).
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

    /// Search for a module (aka dynamic library) loaded by the attached process by name
    /// and return its base address.
    pub fn module(&self, name: &str) -> Option<Address> {
        unsafe {
            match ffi::get_module(self.0, name.as_ptr() as u32, name.len() as u32) {
                0 => None,
                n => Some(n),
            }
        }
    }

    /// Read bytes from the attached processes memory space starting at `addr` into `buf`.
    pub fn read_into_buf(&self, addr: Address, buf: &mut [u8]) -> Result<()> {
        unsafe {
            (ffi::read_mem(self.0, addr, buf.as_mut_ptr() as u32, buf.len() as u32) != 0)
                .then(|| ())
                .ok_or(Error::FailedRead)
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

/// The main autosplitter trait.
///
/// This trait is the entry point for the autosplitter's functionality. The `new` and
/// `update` functions are hooks that will be called by LiveSplit. To interact with
/// LiveSplit's timer, use the functions defined in [`HostFunctions`].
///
/// [`HostFunctions`] is automatically implemented on `Splitter`s, so in your `update`
/// function you can call methods like [`self.split()`](HostFunctions::split) and
/// [`self.set_game_time()`](HostFunctions::set_game_time).
///
/// ## REMEMBER!
///
/// Make sure you use the [`register_autosplitter!`] macro on your splitter! Without it,
/// your wasm library won't expose the proper functions and it'll fail to load.
pub trait Splitter {
    /// Called when the LiveSplit runtime instantiates your splitter. It's a good
    /// time to attach to a game process, set initial variables, or change your tick
    /// rate. Note that this _won't_ be called every time the run is reset.
    fn new() -> Self;

    /// Called periodically by the LiveSplit runtime. To change the rate that it's called,
    /// use [`set_tick_rate`](HostFunctions::set_tick_rate)
    fn update(&mut self);
}

/// The autosplitter's interface for interacting with the LiveSpilit timer.
pub trait HostFunctions {
    /// Output a message. This can be used for debugging and/or sending error messages to
    /// the player through whichever LiveSplit frontend they're using. Note that because
    /// autosplitters run in WASM, they don't have access to STDOUT or files, so
    /// typical solutions like `println!` and logging will not work (this could chagne
    /// in the future as LiveSplit plans to support WASI).
    fn print(&self, str: &str) {
        unsafe { ffi::print_message(str.as_ptr(), str.len()) }
    }

    /// Attach to a process running on the same machine as the autosplitter.
    fn attach(&self, name: &str) -> Option<Process> {
        unsafe {
            match ffi::attach(name.as_ptr() as u32, name.len() as u32) {
                0 => None,
                n => Some(Process(n)),
            }
        }
    }

    /// Start the timer for a run. Note that this will silently do nothing on subsequent
    /// calls. To start a new run, call `reset()` and _then_ `start()`.
    fn start(&self) {
        unsafe { ffi::start() }
    }

    /// Pause the game time counter. This is often used when entering a loading screen or
    /// end level screen for games that use in game time rather than real time. It may
    /// be a good idea to call `set_game_time()` immediately after pausing so that
    /// LiveSplit's game time counter shows the exact current time.
    fn pause(&self) {
        unsafe { ffi::pause_game_time() }
    }

    /// Resume the game time counter. Note that
    fn unpause(&self) {
        unsafe { ffi::resume_game_time() }
    }

    /// Mark the current split as finished and move to the next one.
    fn split(&self) {
        unsafe { ffi::split() }
    }

    /// Reset the run. Don't do this automatically when a run has finished, and in general
    /// be conservative about resetting runs from the autosplitter. Common practice is to
    /// do so only if there's an unambiguous signal that the player is done with this run.
    fn reset(&self) {
        unsafe { ffi::reset() }
    }

    /// Set the game time. Note that if the timer is not paused, the time shown will keep
    /// incrementing immediately after it is set to the given value.
    fn set_game_time(&self, time: f64) {
        unsafe { ffi::set_game_time(time) }
    }

    /// Set the rate at which the [`update`](Splitter::update) function will be called (in
    /// Hz).
    fn set_tick_rate(&self, rate: f64) {
        unsafe { ffi::set_tick_rate(rate) }
    }

    /// Get the current state of the timer. This is how the autosplitter can detect if the
    /// player manually paused or reset a run.
    fn state(&self) -> TimerState {
        unsafe { std::mem::transmute(ffi::get_timer_state() as u8) }
    }

    /// Set a variable which can be displayed by LiveSplit. This is commonly used for
    /// features like death counters.
    fn set_variable(&self, key: &str, value: &str) {
        unsafe {
            ffi::set_variable(
                key.as_ptr() as u32,
                key.len() as u32,
                value.as_ptr() as u32,
                value.len() as u32,
            );
        }
    }
}

impl<T: Splitter> HostFunctions for T {}

/// The possible states of the timer.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[repr(u8)]
pub enum TimerState {
    /// The timer has yet to be started.
    NotRunning = 0,
    /// The timer is currently running.
    Running = 1,
    /// The timer is paused.
    Paused = 2,
    /// The timer is stopped because a run was completed.
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
