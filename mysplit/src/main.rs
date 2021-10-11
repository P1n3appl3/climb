use nix::sys::uio::{self, IoVec, RemoteIoVec};
use nix::unistd::Pid;
use sysinfo::{ProcessExt, System, SystemExt};

fn read_mem(pid: Pid, base: u64, len: u64, buf: &mut [u8]) -> nix::Result<usize> {
    let local = IoVec::from_mut_slice(buf);
    let remote = RemoteIoVec {
        base: base as usize,
        len: len as usize,
    };
    uio::process_vm_readv(pid, &[local], &[remote])
}

fn main() {
    let s = System::new_all();
    let candidates = s.process_by_name("Celeste.bin.x86");
    let pid = Pid::from_raw(
        match candidates[..] {
            [] => Err("Couldn't find Celeste process"),
            [p] => Ok(p),
            [_, _, ..] => Err("Found more than one Celeste process"),
        }
        .unwrap()
        .pid(),
    );

    println!("{}", pid);
    let mut buf = [0u8; 32];
    read_mem(pid, 0xA17650, 8, &mut buf).unwrap();
    println!("{:?}", buf);
}
