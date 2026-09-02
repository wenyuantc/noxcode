use std::io;
use std::process::Output;

use crate::process_spawn;

pub fn git_version_output() -> io::Result<Output> {
    process_spawn::std_command("git").arg("--version").output()
}
