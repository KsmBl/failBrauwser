//! Copies a folder with the failBrauwser engine and prints the time.
//! `cargo run --release --example bench_copy -- <src> <dest-dir>`

use failbrauwser::ops::copy::{Mode, Options, transfer};
use failbrauwser::ops::job::JobCtx;
use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (src, dest) = (PathBuf::from(&args[1]), PathBuf::from(&args[2]));
    let ctx = JobCtx::new();
    let t = std::time::Instant::now();
    transfer(&ctx, &[src], &dest, Mode::Copy, &Options { smooth_writes: true, workers: None, verify: false }).unwrap();
    let s = ctx.snapshot();
    println!("{} files, {} bytes in {:?}", s.files_done, s.bytes_done, t.elapsed());
}
