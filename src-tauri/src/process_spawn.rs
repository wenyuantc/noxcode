use std::ffi::OsStr;
use std::process::Command as StdCommand;

use tokio::process::Command as TokioCommand;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub fn configure_std_command(command: &mut StdCommand) {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;

        command.creation_flags(CREATE_NO_WINDOW);
    }

    #[cfg(not(target_os = "windows"))]
    let _ = command;
}

#[allow(dead_code)]
pub fn configure_tokio_command(command: &mut TokioCommand) {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;

        command.as_std_mut().creation_flags(CREATE_NO_WINDOW);
    }

    #[cfg(not(target_os = "windows"))]
    let _ = command;
}

pub fn std_command(program: impl AsRef<OsStr>) -> StdCommand {
    let mut command = StdCommand::new(program);
    configure_std_command(&mut command);
    command
}

#[allow(dead_code)]
pub fn tokio_command(program: impl AsRef<OsStr>) -> TokioCommand {
    let mut command = TokioCommand::new(program);
    configure_tokio_command(&mut command);
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configure_helpers_do_not_panic() {
        let mut std_cmd = StdCommand::new("echo");
        configure_std_command(&mut std_cmd);
        let mut tokio_cmd = TokioCommand::new("echo");
        configure_tokio_command(&mut tokio_cmd);
        let _ = std_command("echo");
        let _ = tokio_command("echo");
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn hidden_cmd_echo_captures_stdout() {
        let output = tokio_command("cmd")
            .args(["/C", "echo hello"])
            .output()
            .await
            .expect("spawn hidden cmd");
        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout).to_ascii_lowercase();
        assert!(
            stdout.contains("hello"),
            "expected stdout to contain hello, got {stdout:?}"
        );
    }
}
