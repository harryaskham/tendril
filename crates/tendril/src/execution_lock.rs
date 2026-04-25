use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::error::TendrilError;

pub const DEFAULT_LOCK_TIMEOUT_MS: u64 = 60_000;
pub const DEFAULT_LOCK_STALE_MS: u64 = 30_000;
const POLL_INTERVAL_MS: u64 = 50;
const HEARTBEAT_INTERVAL_MS: u64 = 1_000;

static TOKEN_COUNTER: AtomicU64 = AtomicU64::new(1);

#[must_use]
pub fn default_execution_lock_path() -> PathBuf {
    let user = env::var("USER")
        .or_else(|_| env::var("USERNAME"))
        .or_else(|_| env::var("UID"))
        .unwrap_or_else(|_| "unknown-user".to_owned());
    let session = env::var("TENDRIL_LOCK_SESSION")
        .or_else(|_| env::var("WAYLAND_DISPLAY"))
        .or_else(|_| env::var("DISPLAY"))
        .or_else(|_| env::var("XDG_SESSION_ID"))
        .unwrap_or_else(|_| "default-session".to_owned());
    env::temp_dir().join(format!(
        "tendril-execution-lock-{}-{}",
        sanitize_path_component(&user),
        sanitize_path_component(&session)
    ))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionLockRequest {
    pub enabled: bool,
    pub lock_path: PathBuf,
    pub timeout_ms: u64,
    pub stale_ms: u64,
    pub command: String,
    pub target_kind: Option<String>,
    pub target_id: Option<String>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionLockReport {
    pub enabled: bool,
    pub acquired: bool,
    pub lock_path: String,
    pub timeout_ms: u64,
    pub stale_ms: u64,
    pub wait_ms: u64,
    pub queue_position_at_join: usize,
    pub queue_depth_at_join: usize,
    pub stale_locks_reaped: u32,
    pub stale_tickets_reaped: u32,
    pub owner_pid: u32,
    pub token: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

pub struct ExecutionLockPermit {
    report: ExecutionLockReport,
    guard: Option<ExecutionLockGuard>,
}

impl ExecutionLockPermit {
    #[must_use]
    pub fn report(&self) -> &ExecutionLockReport {
        &self.report
    }
}

struct ExecutionLockGuard {
    lock_dir: PathBuf,
    token: String,
    stop: Arc<(Mutex<bool>, Condvar)>,
    heartbeat_thread: Option<JoinHandle<()>>,
}

impl Drop for ExecutionLockPermit {
    fn drop(&mut self) {
        let _ = self.guard.take();
    }
}

impl Drop for ExecutionLockGuard {
    fn drop(&mut self) {
        {
            let (lock, condvar) = &*self.stop;
            if let Ok(mut stopped) = lock.lock() {
                *stopped = true;
                condvar.notify_all();
            }
        }

        if let Some(handle) = self.heartbeat_thread.take() {
            let _ = handle.join();
        }

        if lock_owner_token(&self.lock_dir).as_deref() == Some(self.token.as_str()) {
            let _ = fs::remove_dir_all(&self.lock_dir);
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OwnerMetadata {
    schema_version: u32,
    token: String,
    owner_pid: u32,
    command: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    target_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    target_id: Option<String>,
    acquired_at_unix_ms: u128,
    heartbeat_unix_ms: u128,
    stale_after_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct QueueTicketMetadata {
    schema_version: u32,
    token: String,
    owner_pid: u32,
    command: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    target_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    target_id: Option<String>,
    queued_at_unix_ms: u128,
    heartbeat_unix_ms: u128,
    timeout_ms: u64,
    stale_after_ms: u64,
}

#[allow(clippy::too_many_lines)]
pub fn acquire_execution_lock(
    request: &ExecutionLockRequest,
) -> Result<ExecutionLockPermit, TendrilError> {
    validate_request(request)?;

    let token = make_token();
    let owner_pid = std::process::id();
    if !request.enabled {
        return Ok(ExecutionLockPermit {
            report: ExecutionLockReport {
                enabled: false,
                acquired: false,
                lock_path: request.lock_path.display().to_string(),
                timeout_ms: request.timeout_ms,
                stale_ms: request.stale_ms,
                wait_ms: 0,
                queue_position_at_join: 0,
                queue_depth_at_join: 0,
                stale_locks_reaped: 0,
                stale_tickets_reaped: 0,
                owner_pid,
                token,
                reason: request
                    .reason
                    .clone()
                    .or_else(|| Some("disabled".to_owned())),
            },
            guard: None,
        });
    }

    let started = Instant::now();
    fs::create_dir_all(request.lock_path.join("queue")).map_err(|error| {
        TendrilError::execution_failure(
            "execution_lock_unavailable",
            format!(
                "failed to create Tendril execution lock directory `{}`: {error}",
                request.lock_path.display()
            ),
            None,
        )
    })?;

    let queue_dir = request.lock_path.join("queue");
    let lock_dir = request.lock_path.join("held");
    let ticket_path = queue_dir.join(format!(
        "{:020}-{}-{}.json",
        now_unix_ms(),
        owner_pid,
        token
    ));
    let ticket = QueueTicketMetadata {
        schema_version: 1,
        token: token.clone(),
        owner_pid,
        command: request.command.clone(),
        target_kind: request.target_kind.clone(),
        target_id: request.target_id.clone(),
        queued_at_unix_ms: now_unix_ms(),
        heartbeat_unix_ms: now_unix_ms(),
        timeout_ms: request.timeout_ms,
        stale_after_ms: request.stale_ms,
    };
    write_json_file(&ticket_path, &ticket).map_err(|error| {
        TendrilError::execution_failure(
            "execution_lock_unavailable",
            format!(
                "failed to write Tendril execution queue ticket `{}`: {error}",
                ticket_path.display()
            ),
            None,
        )
    })?;

    let mut stale_locks_reaped = 0_u32;
    let mut stale_tickets_reaped = 0_u32;
    let mut join_position = 0_usize;
    let mut join_depth = 0_usize;
    let mut last_ticket_heartbeat = Instant::now();

    loop {
        stale_tickets_reaped = stale_tickets_reaped.saturating_add(clean_stale_tickets(
            &queue_dir,
            &ticket_path,
            request.stale_ms,
        ));

        let queue_state = queue_position(&queue_dir, &ticket_path);
        if join_position == 0 {
            join_position = queue_state.position;
            join_depth = queue_state.depth;
        }

        if queue_state.position == 1 {
            if stale_lock(&lock_dir, request.stale_ms) {
                let _ = fs::remove_dir_all(&lock_dir);
                stale_locks_reaped = stale_locks_reaped.saturating_add(1);
            }

            match fs::create_dir(&lock_dir) {
                Ok(()) => {
                    let acquired_at_unix_ms = now_unix_ms();
                    let owner = OwnerMetadata {
                        schema_version: 1,
                        token: token.clone(),
                        owner_pid,
                        command: request.command.clone(),
                        target_kind: request.target_kind.clone(),
                        target_id: request.target_id.clone(),
                        acquired_at_unix_ms,
                        heartbeat_unix_ms: acquired_at_unix_ms,
                        stale_after_ms: request.stale_ms,
                    };
                    write_owner_metadata(&lock_dir, &owner).map_err(|error| {
                        let _ = fs::remove_dir_all(&lock_dir);
                        TendrilError::execution_failure(
                            "execution_lock_unavailable",
                            format!(
                                "failed to write Tendril execution lock metadata `{}`: {error}",
                                lock_dir.join("owner.json").display()
                            ),
                            None,
                        )
                    })?;
                    let _ = fs::remove_file(&ticket_path);
                    let wait_ms = elapsed_ms(started.elapsed());
                    let guard = start_guard(lock_dir.clone(), owner);
                    return Ok(ExecutionLockPermit {
                        report: ExecutionLockReport {
                            enabled: true,
                            acquired: true,
                            lock_path: request.lock_path.display().to_string(),
                            timeout_ms: request.timeout_ms,
                            stale_ms: request.stale_ms,
                            wait_ms,
                            queue_position_at_join: join_position,
                            queue_depth_at_join: join_depth,
                            stale_locks_reaped,
                            stale_tickets_reaped,
                            owner_pid,
                            token,
                            reason: None,
                        },
                        guard: Some(guard),
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => {
                    let _ = fs::remove_file(&ticket_path);
                    return Err(TendrilError::execution_failure(
                        "execution_lock_unavailable",
                        format!(
                            "failed to acquire Tendril execution lock `{}`: {error}",
                            lock_dir.display()
                        ),
                        None,
                    ));
                }
            }
        }

        let waited = started.elapsed();
        if waited >= Duration::from_millis(request.timeout_ms) {
            let report = ExecutionLockReport {
                enabled: true,
                acquired: false,
                lock_path: request.lock_path.display().to_string(),
                timeout_ms: request.timeout_ms,
                stale_ms: request.stale_ms,
                wait_ms: elapsed_ms(waited),
                queue_position_at_join: join_position,
                queue_depth_at_join: join_depth,
                stale_locks_reaped,
                stale_tickets_reaped,
                owner_pid,
                token,
                reason: Some("timeout".to_owned()),
            };
            let holder = read_owner_metadata(&lock_dir).ok();
            let _ = fs::remove_file(&ticket_path);
            return Err(TendrilError::timeout(
                "execution_lock_timeout",
                format!(
                    "timed out waiting for Tendril execution lock `{}` after {} ms",
                    request.lock_path.display(),
                    request.timeout_ms
                ),
                Some(json!({
                    "execution_lock": report,
                    "holder": holder,
                    "queue_position": queue_state.position,
                    "queue_depth": queue_state.depth,
                })),
            ));
        }

        if last_ticket_heartbeat.elapsed() >= Duration::from_millis(HEARTBEAT_INTERVAL_MS) {
            let refreshed = QueueTicketMetadata {
                heartbeat_unix_ms: now_unix_ms(),
                ..ticket.clone()
            };
            let _ = write_json_file(&ticket_path, &refreshed);
            last_ticket_heartbeat = Instant::now();
        }

        let remaining = Duration::from_millis(request.timeout_ms).saturating_sub(waited);
        thread::sleep(remaining.min(Duration::from_millis(POLL_INTERVAL_MS)));
    }
}

fn validate_request(request: &ExecutionLockRequest) -> Result<(), TendrilError> {
    if request.timeout_ms == 0 {
        return Err(
            TendrilError::validation("lock_timeout_ms must be greater than zero")
                .with_code("invalid_run_input")
                .with_field("lock_timeout_ms"),
        );
    }

    if request.stale_ms == 0 {
        return Err(
            TendrilError::validation("lock_stale_ms must be greater than zero")
                .with_code("invalid_run_input")
                .with_field("lock_stale_ms"),
        );
    }

    Ok(())
}

fn start_guard(lock_dir: PathBuf, owner: OwnerMetadata) -> ExecutionLockGuard {
    let token = owner.token.clone();
    let stop = Arc::new((Mutex::new(false), Condvar::new()));
    let thread_stop = Arc::clone(&stop);
    let thread_lock_dir = lock_dir.clone();
    let heartbeat_thread = thread::spawn(move || {
        let (lock, condvar) = &*thread_stop;
        loop {
            let Ok(mut stopped) = lock.lock() else {
                return;
            };
            let result =
                condvar.wait_timeout(stopped, Duration::from_millis(HEARTBEAT_INTERVAL_MS));
            let Ok((next_stopped, _)) = result else {
                return;
            };
            stopped = next_stopped;
            if *stopped {
                return;
            }
            drop(stopped);

            if lock_owner_token(&thread_lock_dir).as_deref() != Some(owner.token.as_str()) {
                return;
            }
            let refreshed = OwnerMetadata {
                heartbeat_unix_ms: now_unix_ms(),
                ..owner.clone()
            };
            let _ = write_owner_metadata(&thread_lock_dir, &refreshed);
        }
    });

    ExecutionLockGuard {
        lock_dir,
        token,
        stop,
        heartbeat_thread: Some(heartbeat_thread),
    }
}

#[derive(Debug, Clone, Copy)]
struct QueueState {
    position: usize,
    depth: usize,
}

fn queue_position(queue_dir: &Path, ticket_path: &Path) -> QueueState {
    let mut entries = queue_entries(queue_dir);
    entries.sort();
    let position = entries
        .iter()
        .position(|path| path == ticket_path)
        .map_or(1, |index| index + 1);
    QueueState {
        position,
        depth: entries.len().max(1),
    }
}

fn clean_stale_tickets(queue_dir: &Path, self_ticket: &Path, stale_ms: u64) -> u32 {
    let mut removed = 0_u32;
    for path in queue_entries(queue_dir) {
        if path == self_ticket {
            continue;
        }
        if stale_ticket(&path, stale_ms) && fs::remove_file(&path).is_ok() {
            removed = removed.saturating_add(1);
        }
    }
    removed
}

fn queue_entries(queue_dir: &Path) -> Vec<PathBuf> {
    fs::read_dir(queue_dir)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("json"))
        .collect()
}

fn stale_ticket(path: &Path, stale_ms: u64) -> bool {
    read_json_file::<QueueTicketMetadata>(path).map_or_else(
        |_| metadata_age_ms(path) > u128::from(stale_ms),
        |ticket| age_ms(ticket.heartbeat_unix_ms) > u128::from(stale_ms),
    )
}

fn stale_lock(lock_dir: &Path, stale_ms: u64) -> bool {
    if !lock_dir.exists() {
        return false;
    }

    read_owner_metadata(lock_dir).map_or_else(
        |_| metadata_age_ms(lock_dir) > u128::from(stale_ms),
        |owner| age_ms(owner.heartbeat_unix_ms) > u128::from(stale_ms),
    )
}

fn lock_owner_token(lock_dir: &Path) -> Option<String> {
    read_owner_metadata(lock_dir).ok().map(|owner| owner.token)
}

fn write_owner_metadata(lock_dir: &Path, metadata: &OwnerMetadata) -> Result<(), std::io::Error> {
    write_json_file(&lock_dir.join("owner.json"), metadata)
}

fn read_owner_metadata(lock_dir: &Path) -> Result<OwnerMetadata, std::io::Error> {
    read_json_file(&lock_dir.join("owner.json"))
}

fn write_json_file<T>(path: &Path, value: &T) -> Result<(), std::io::Error>
where
    T: Serialize,
{
    let bytes = serde_json::to_vec_pretty(value).map_err(std::io::Error::other)?;
    fs::write(path, bytes)
}

fn read_json_file<T>(path: &Path) -> Result<T, std::io::Error>
where
    T: for<'de> Deserialize<'de>,
{
    let bytes = fs::read(path)?;
    serde_json::from_slice(&bytes).map_err(std::io::Error::other)
}

fn now_unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis())
}

fn age_ms(then_unix_ms: u128) -> u128 {
    now_unix_ms().saturating_sub(then_unix_ms)
}

fn metadata_age_ms(path: &Path) -> u128 {
    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map_or(u128::MAX, |duration| age_ms(duration.as_millis()))
}

fn elapsed_ms(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn make_token() -> String {
    let counter = TOKEN_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{}-{}-{counter}", std::process::id(), now_unix_ms())
}

fn sanitize_path_component(value: &str) -> String {
    let sanitized: String = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-') {
                character
            } else {
                '_'
            }
        })
        .collect();
    if sanitized.is_empty() {
        "default".to_owned()
    } else {
        sanitized
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::*;

    fn request(path: PathBuf) -> ExecutionLockRequest {
        ExecutionLockRequest {
            enabled: true,
            lock_path: path,
            timeout_ms: 2_000,
            stale_ms: 500,
            command: "run".to_owned(),
            target_kind: Some("window".to_owned()),
            target_id: Some("demo".to_owned()),
            reason: None,
        }
    }

    #[test]
    fn disabled_lock_returns_metadata_without_acquiring() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let permit = acquire_execution_lock(&ExecutionLockRequest {
            enabled: false,
            reason: Some("--no-lock".to_owned()),
            ..request(tempdir.path().join("lock"))
        })
        .expect("disabled lock should succeed");

        assert!(!permit.report().enabled);
        assert!(!permit.report().acquired);
        assert_eq!(permit.report().reason.as_deref(), Some("--no-lock"));
        assert!(!tempdir.path().join("lock/held").exists());
    }

    #[test]
    fn second_acquirer_waits_until_first_releases() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let lock_path = tempdir.path().join("lock");
        let first = acquire_execution_lock(&request(lock_path.clone())).expect("first lock");
        let (tx, rx) = mpsc::channel();

        let thread_lock_path = lock_path.clone();
        let handle = thread::spawn(move || {
            let acquired = acquire_execution_lock(&request(thread_lock_path)).expect("second lock");
            tx.send(acquired.report().wait_ms).expect("send wait");
            acquired
        });

        thread::sleep(Duration::from_millis(150));
        assert!(rx.try_recv().is_err(), "second lock should still be queued");
        drop(first);

        let wait_ms = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("wait report");
        assert!(
            wait_ms >= 100,
            "expected visible queue wait, got {wait_ms} ms"
        );
        let second = handle.join().expect("thread join");
        assert!(second.report().acquired);
    }

    #[test]
    fn lock_timeout_reports_queue_metadata() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let lock_path = tempdir.path().join("lock");
        let _first = acquire_execution_lock(&request(lock_path.clone())).expect("first lock");

        let error = match acquire_execution_lock(&ExecutionLockRequest {
            timeout_ms: 100,
            stale_ms: 2_000,
            ..request(lock_path)
        }) {
            Ok(_) => panic!("second lock should time out"),
            Err(error) => error,
        };

        assert!(matches!(error, TendrilError::Timeout { .. }));
        let details = error.to_json_error().details.expect("details");
        assert_eq!(details["execution_lock"]["acquired"], false);
        assert_eq!(details["execution_lock"]["reason"], "timeout");
        assert!(details["queue_depth"].as_u64().unwrap_or_default() >= 1);
    }

    #[test]
    fn stale_lock_is_reaped() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let lock_path = tempdir.path().join("lock");
        let held = lock_path.join("held");
        fs::create_dir_all(&held).expect("held dir");
        write_owner_metadata(
            &held,
            &OwnerMetadata {
                schema_version: 1,
                token: "stale".to_owned(),
                owner_pid: 99_999,
                command: "run".to_owned(),
                target_kind: Some("window".to_owned()),
                target_id: Some("stale".to_owned()),
                acquired_at_unix_ms: now_unix_ms().saturating_sub(5_000),
                heartbeat_unix_ms: now_unix_ms().saturating_sub(5_000),
                stale_after_ms: 10,
            },
        )
        .expect("owner metadata");

        let permit = acquire_execution_lock(&ExecutionLockRequest {
            stale_ms: 10,
            ..request(lock_path)
        })
        .expect("stale lock should be reaped");

        assert!(permit.report().acquired);
        assert_eq!(permit.report().stale_locks_reaped, 1);
    }
}
