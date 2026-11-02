use std::{
    fs::OpenOptions,
    process::Command,
    time::{Duration, Instant},
};

pub fn folder() -> std::path::PathBuf {
    std::path::PathBuf::from(
        std::env::var_os("LOCALAPPDATA")
            .or_else(|| std::env::var_os("HOME"))
            .unwrap_or_default(),
    )
    .join("ImageAtlas")
}

#[derive(Default)]
struct Restarts {
    attempts: u32,
}

impl Restarts {
    fn next(&mut self, success: bool, uptime: Duration) -> Option<Duration> {
        if success {
            return None;
        }
        if uptime >= Duration::from_secs(120) {
            self.attempts = 0;
        }
        if self.attempts >= 5 {
            return None;
        }
        let delay = Duration::from_secs(1 << self.attempts);
        self.attempts += 1;
        Some(delay)
    }
}

pub fn supervise() {
    let result = (|| -> anyhow::Result<()> {
        std::fs::create_dir_all(folder())?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(folder().join("session.lock"))?;
        if lock.try_lock().is_err() {
            return Ok(());
        }
        let executable = std::env::current_exe()?;
        let arguments: Vec<_> = std::env::args_os().skip(1).collect();
        let mut restarts = Restarts::default();
        loop {
            let started = Instant::now();
            let status = Command::new(&executable)
                .arg("--ui-session")
                .args(&arguments)
                .status()?;
            if let Some(delay) = restarts.next(status.success(), started.elapsed()) {
                crate::record_error(&format!(
                    "Atlas exited with {status}; restart attempt {} of 5",
                    restarts.attempts
                ));
                std::thread::sleep(delay);
            } else {
                if !status.success() {
                    anyhow::bail!(
                        "Atlas could not stay open after five restart attempts. Your saved mapping will resume when you open Atlas again."
                    );
                }
                break;
            }
        }
        Ok(())
    })();
    if let Err(error) = result {
        crate::record_error(&error.to_string());
        rfd::MessageDialog::new()
            .set_title("Image Atlas recovery")
            .set_description(error.to_string())
            .set_level(rfd::MessageLevel::Error)
            .show();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn restarts_five_times_and_stops_after_sixth_crash() {
        let mut policy = Restarts::default();
        for seconds in [1, 2, 4, 8, 16] {
            assert_eq!(
                policy.next(false, Duration::ZERO),
                Some(Duration::from_secs(seconds))
            );
        }
        assert_eq!(policy.next(false, Duration::ZERO), None);
    }
    #[test]
    fn normal_close_never_restarts() {
        assert_eq!(Restarts::default().next(true, Duration::ZERO), None);
    }
    #[test]
    fn stable_session_resets_crash_budget() {
        let mut policy = Restarts { attempts: 5 };
        assert_eq!(
            policy.next(false, Duration::from_secs(120)),
            Some(Duration::from_secs(1))
        );
    }
}
