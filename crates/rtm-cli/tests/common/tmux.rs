use std::process::{Command, Output};
use std::time::Duration;

use uuid::Uuid;

use super::{RtmHarness, output_stdout, wait_until};

pub struct TmuxSession {
    name: String,
}

impl TmuxSession {
    pub fn start(prefix: &str) -> Option<Self> {
        if !available() {
            return None;
        }
        let name = format!("{prefix}-{}", Uuid::now_v7().simple());
        tmux(["new-session", "-d", "-s", &name]);
        Some(Self { name })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn pane(&self) -> String {
        tmux_stdout(["list-panes", "-t", &self.name, "-F", "#S:#I.#P"])
            .lines()
            .next()
            .expect("pane")
            .to_owned()
    }

    pub fn send_spawn_command(&self, harness: &RtmHarness, session_id: &str) {
        let pane = self.pane();
        let command = format!(
            "RTM_SOCKET_PATH={} RTM_DB_PATH={} {} spawn --runtime claude --session-id {} --tmux-address {}",
            harness.socket_path().display(),
            harness.db_path().display(),
            harness.rtm_path().display(),
            session_id,
            pane
        );
        tmux(["send-keys", "-t", &self.name, "-l", &command]);
        tmux(["send-keys", "-t", &self.name, "Enter"]);
    }

    pub fn wait_for_capture(&self, needle: &str) {
        wait_until(Duration::from_secs(5), || {
            let capture = tmux_stdout(["capture-pane", "-p", "-t", &self.name]);
            capture.contains(needle).then_some(())
        })
        .unwrap_or_else(|| panic!("tmux pane never contained {needle}"));
    }

    pub fn kill(&self) {
        let _ = tmux_output(["kill-session", "-t", &self.name]);
    }
}

impl Drop for TmuxSession {
    fn drop(&mut self) {
        self.kill();
    }
}

pub fn available() -> bool {
    Command::new("tmux")
        .arg("-V")
        .output()
        .is_ok_and(|output| output.status.success())
}

fn tmux<const N: usize>(args: [&str; N]) {
    let output = tmux_output(args);
    assert!(output.status.success(), "tmux command failed: {output:?}");
}

fn tmux_stdout<const N: usize>(args: [&str; N]) -> String {
    let output = tmux_output(args);
    assert!(output.status.success(), "tmux command failed: {output:?}");
    output_stdout(output)
}

fn tmux_output<const N: usize>(args: [&str; N]) -> Output {
    Command::new("tmux").args(args).output().expect("tmux")
}
