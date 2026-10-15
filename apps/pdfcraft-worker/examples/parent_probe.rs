//! A parent process that exists only to be killed: starts a worker, prints its pid, then sleeps.
//! The orphan test kills this process without warning and checks that the worker dies with it.

use std::io::Write;

use pdfcraft_worker::supervisor::{Config, Worker};

fn main() {
    let Some(exe) = std::env::args().nth(1) else {
        eprintln!("usage: parent_probe <path to pdfcraft-worker>");
        std::process::exit(2);
    };
    let worker = match Worker::spawn(&Config::new(exe)) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    // With a second argument `hang`, wedge the worker first (a `fault-injection` worker obeys): it
    // then cannot notice that its input closed, so only the job object can end it.
    let mut worker = worker;
    if std::env::args().nth(2).as_deref() == Some("hang") {
        let _ = worker.debug(&pdfcraft_worker::protocol::Debug::Hang);
        std::thread::sleep(std::time::Duration::from_millis(300));
    }
    println!("PID {}", worker.pid());
    let _ = std::io::stdout().flush();
    std::thread::sleep(std::time::Duration::from_secs(3600));
}
