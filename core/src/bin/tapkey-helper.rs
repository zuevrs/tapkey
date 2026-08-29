//! The credential helper.
//!
//! A thin executable over [`tapkey_core::helper`]: read the arguments, read stdin, write the
//! answer, and end with the exit code that is all `has` ever says. Everything worth testing is
//! tested in the module, at the seam its `Io` provides; this file exists so there is a binary to
//! test.

fn main() {
    let mut io = tapkey_core::helper::Io {
        args: std::env::args().skip(1).collect(),
        stdin: {
            use std::io::Read;
            let mut buffer = Vec::new();
            std::io::stdin().read_to_end(&mut buffer).expect("stdin");
            buffer
        },
        stdout: Vec::new(),
        stderr: Vec::new(),
    };

    let code = tapkey_core::helper::run(&mut io);

    use std::io::Write;
    std::io::stdout()
        .write_all(&io.stdout)
        .expect("stdout is writable or the run is over anyway");
    std::io::stderr()
        .write_all(&io.stderr)
        .expect("stderr is writable or the run is over anyway");

    std::process::exit(code);
}
