//! YubiKeys attached to Windows, seen from WSL through windows-fido-bridge.
//!
//! Under WSL the keys stay on the Windows side, so local HID and PC/SC find
//! nothing. The bridge's OpenSSH middleware (`SSH_SK_PROVIDER`) ships a Windows
//! executable next to it that lists the connected serials.

use anyhow::{Context, Result, bail};
use std::{
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

const EXE: &str = "windowsfidobridge.exe";

/// Bounds the interop call, which git blocks on. Starting a Windows process
/// from WSL can take a second or more when cold, so this is looser than the
/// HID timeout.
pub const TIMEOUT: Duration = Duration::from_secs(10);

/// The bridge executable, if this environment signs through windows-fido-bridge.
pub fn locate() -> Option<PathBuf> {
    let provider = std::env::var("SSH_SK_PROVIDER").ok();
    let switch = std::env::var("GFH_BRIDGE").ok();
    exe_for(provider.as_deref(), switch.as_deref())
}

/// `GFH_BRIDGE=0` turns the bridge off; otherwise it is used when the exe sits
/// next to the middleware library named by `SSH_SK_PROVIDER`.
fn exe_for(sk_provider: Option<&str>, switch: Option<&str>) -> Option<PathBuf> {
    if switch == Some("0") {
        return None;
    }
    let exe = Path::new(sk_provider.filter(|p| !p.is_empty())?)
        .parent()?
        .join(EXE);
    exe.is_file().then_some(exe)
}

/// Serials of the YubiKeys connected to Windows, killing the bridge after
/// `timeout`.
pub fn list_serials(exe: &Path, timeout: Duration) -> Result<Vec<u32>> {
    let mut child = Command::new(exe)
        .arg("--list-serials")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to run windows-fido-bridge at {}", exe.display()))?;

    // Drain the pipes on their own threads so a chatty child cannot block on a
    // full pipe while we poll for its exit.
    let stdout = drain(child.stdout.take());
    let stderr = drain(child.stderr.take());

    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            bail!(
                "windows-fido-bridge ({}) did not list the YubiKey serials within {timeout:?}",
                exe.display()
            );
        }
        thread::sleep(Duration::from_millis(10));
    };

    let stdout = stdout.join().unwrap_or_default();
    if !status.success() {
        let stderr = stderr.join().unwrap_or_default();
        bail!(
            "windows-fido-bridge could not list the YubiKey serials ({status}): {}",
            String::from_utf8_lossy(&stderr).trim()
        );
    }
    parse_serials(&String::from_utf8_lossy(&stdout))
}

fn drain(pipe: Option<impl Read + Send + 'static>) -> thread::JoinHandle<Vec<u8>> {
    thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut pipe) = pipe {
            let _ = pipe.read_to_end(&mut buf);
        }
        buf
    })
}

/// One decimal serial per line. A line that is not one means the bridge speaks
/// a different protocol than gfh expects, so it is an error rather than a
/// silently missing key.
fn parse_serials(out: &str) -> Result<Vec<u32>> {
    out.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|l| {
            l.parse()
                .with_context(|| format!("windows-fido-bridge printed {l:?}, not a serial"))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, os::unix::fs::PermissionsExt, sync::Mutex};

    /// Scratch dir holding a dummy middleware and, optionally, a fake bridge.
    struct Fixture(PathBuf);

    impl Fixture {
        fn new(name: &str, script: Option<&str>) -> Self {
            let dir =
                std::env::temp_dir().join(format!("gfh-bridge-{}-{name}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("libwindowsfidobridge.so"), b"").unwrap();
            if let Some(script) = script {
                let exe = dir.join(EXE);
                fs::write(&exe, format!("#!/bin/sh\n{script}\n")).unwrap();
                fs::set_permissions(&exe, fs::Permissions::from_mode(0o755)).unwrap();
            }
            Self(dir)
        }

        fn provider(&self) -> String {
            self.0.join("libwindowsfidobridge.so").display().to_string()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    // Writing an executable while another thread forks can fail its exec with
    // ETXTBSY, so the tests that create and run one take turns.
    static EXEC: Mutex<()> = Mutex::new(());

    fn run(name: &str, script: &str, timeout: Duration) -> Result<Vec<u32>> {
        let _guard = EXEC.lock().unwrap_or_else(|e| e.into_inner());
        let fx = Fixture::new(name, Some(script));
        let exe = exe_for(Some(&fx.provider()), None).expect("bridge exe located");
        list_serials(&exe, timeout)
    }

    #[test]
    fn parses_empty_output_as_no_keys() {
        assert_eq!(parse_serials("").unwrap(), Vec::<u32>::new());
        assert_eq!(parse_serials("\n").unwrap(), Vec::<u32>::new());
    }

    #[test]
    fn parses_one_serial_per_line() {
        assert_eq!(parse_serials("123\n4567890\n").unwrap(), [123, 4567890]);
    }

    #[test]
    fn parses_crlf_output() {
        assert_eq!(parse_serials("123\r\n456\r\n").unwrap(), [123, 456]);
    }

    #[test]
    fn rejects_a_line_that_is_not_a_serial() {
        let err = parse_serials("123\nERROR: oops\n").unwrap_err();
        assert!(format!("{err:#}").contains("ERROR: oops"), "{err:#}");
        assert!(parse_serials("-1\n").is_err());
    }

    #[test]
    fn exe_sits_next_to_the_provider() {
        let fx = Fixture::new("located", Some("exit 0"));
        assert_eq!(exe_for(Some(&fx.provider()), None), Some(fx.0.join(EXE)));
        assert_eq!(
            exe_for(Some(&fx.provider()), Some("1")),
            Some(fx.0.join(EXE))
        );
    }

    #[test]
    fn no_bridge_without_provider_or_exe_or_when_disabled() {
        let fx = Fixture::new("absent", None);
        assert_eq!(exe_for(None, None), None);
        assert_eq!(exe_for(Some(""), None), None);
        assert_eq!(exe_for(Some(&fx.provider()), None), None);

        let fx = Fixture::new("disabled", Some("exit 0"));
        assert_eq!(exe_for(Some(&fx.provider()), Some("0")), None);
    }

    #[test]
    fn lists_serials_from_the_bridge() {
        let serials = run("list", r"printf '111\r\n222\r\n'", TIMEOUT).unwrap();
        assert_eq!(serials, [111, 222]);
    }

    #[test]
    fn passes_the_list_flag() {
        let script = r#"[ "$1" = --list-serials ] && echo 7 || exit 3"#;
        assert_eq!(run("flag", script, TIMEOUT).unwrap(), [7]);
    }

    #[test]
    fn reports_bridge_failure_with_its_stderr() {
        let err = run("fail", "echo 'PC/SC unavailable' >&2; exit 1", TIMEOUT).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("windows-fido-bridge"), "{msg}");
        assert!(msg.contains("PC/SC unavailable"), "{msg}");
    }

    #[test]
    fn times_out_a_hung_bridge() {
        let start = Instant::now();
        let err = run("hang", "exec sleep 30", Duration::from_millis(200)).unwrap_err();
        assert!(format!("{err:#}").contains("within"), "{err:#}");
        assert!(start.elapsed() < Duration::from_secs(5));
    }
}
