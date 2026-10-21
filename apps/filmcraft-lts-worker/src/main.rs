//! `filmcraft-lts-worker`: one contained editing session of the Video editor engine for the LTS
//! native host, driven over the Editor Worker Protocol (EWP v1) on stdin and stdout. The host
//! starts it as `filmcraft-lts-worker --workspace <session workspace>`; see the library for the
//! protocol and the containment.

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<std::ffi::OsString> = std::env::args_os().collect();
    match filmcraft_lts_worker::serve(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("filmcraft-lts-worker: {e}");
            ExitCode::FAILURE
        }
    }
}
