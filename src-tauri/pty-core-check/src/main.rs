// Standalone smoke test validating the exact portable-pty 0.9 API calls
// shared by src-tauri/src/pty.rs. This crate depends ONLY on portable-pty,
// so it can compile/run on this Linux box without the Tauri/webkit stack.
//
// It exercises: native_pty_system(), openpty(PtySize), CommandBuilder,
// slave.spawn_command(), drop(slave), master.take_writer(),
// master.try_clone_reader(), master.resize(), child.kill(), child.wait().

use std::io::{Read, Write};
use std::sync::mpsc;
use std::time::Duration;

use portable_pty::{native_pty_system, CommandBuilder, PtySize};

fn main() {
    let pty_system = native_pty_system();

    // Open a PTY with an initial size.
    let pair = pty_system
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("openpty failed");

    // Build the command (a bash shell) and set env like the real backend.
    let mut cmd = CommandBuilder::new("/bin/bash");
    cmd.env("TERM", "xterm-256color");
    if let Some(home) = dirs_home() {
        cmd.cwd(home);
    }

    // Spawn the command attached to the slave side.
    let mut child = pair.slave.spawn_command(cmd).expect("spawn_command failed");

    // Per portable-pty guidance: drop the slave after spawn so that EOF is
    // delivered on the master reader once the child exits.
    drop(pair.slave);

    // Writer for PTY stdin.
    let mut writer = pair.master.take_writer().expect("take_writer failed");

    // Dedicated reader thread reading from a cloned reader.
    let mut reader = pair
        .master
        .try_clone_reader()
        .expect("try_clone_reader failed");

    let (tx, rx) = mpsc::channel::<String>();
    let reader_thread = std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break, // EOF
                Ok(n) => {
                    let chunk = String::from_utf8_lossy(&buf[..n]).to_string();
                    if tx.send(chunk).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    // Send a command to stdin.
    writer
        .write_all(b"echo hello\n")
        .expect("write_all failed");
    writer.flush().expect("flush failed");

    // Collect output for a short while, looking for "hello".
    let mut collected = String::new();
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while std::time::Instant::now() < deadline {
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(chunk) => {
                collected.push_str(&chunk);
                if collected.contains("hello") {
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if collected.contains("hello") {
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    println!("=== captured output start ===");
    print!("{}", collected);
    println!("\n=== captured output end ===");

    // Validate resize works.
    pair.master
        .resize(PtySize {
            rows: 40,
            cols: 120,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("resize failed");
    println!("resize OK");

    // Kill the child and reap it.
    child.kill().expect("kill failed");
    let status = child.wait().expect("wait failed");
    println!("child exit status success={}", status.success());

    // Ensure the writer/master are dropped so the reader thread sees EOF.
    drop(writer);
    drop(pair.master);
    let _ = reader_thread.join();

    if collected.contains("hello") {
        println!("SMOKE TEST PASSED: found 'hello' in PTY output");
    } else {
        eprintln!("SMOKE TEST FAILED: 'hello' not found");
        std::process::exit(1);
    }
}

// Minimal home-dir lookup without extra deps (mirrors backend intent).
fn dirs_home() -> Option<String> {
    std::env::var("HOME").ok().filter(|s| !s.is_empty())
}
