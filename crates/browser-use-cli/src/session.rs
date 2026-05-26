use std::path::PathBuf;
use std::process::Command as StdCommand;
use std::time::{Duration, Instant};

use browser_use_cdp::{BrowserProfile, BrowserSession, CdpBrowserSession};
use browser_use_core::{AgentHistory, BrowserActionExecutor};
use tokio::time::sleep;

use crate::print_state;

#[derive(Debug, clap::Subcommand)]
pub(crate) enum SessionCommand {
    /// Launch a persistent Chrome session and navigate it to a URL.
    Start {
        id: String,
        url: String,
        #[arg(long, default_value_t = false)]
        screenshot: bool,
    },
    /// Print state for an existing persistent session.
    State {
        id: String,
        #[arg(long, default_value_t = false)]
        screenshot: bool,
    },
    /// Run a JSON action list against an existing persistent session.
    Actions {
        id: String,
        actions: PathBuf,
        #[arg(long, default_value_t = true)]
        screenshot: bool,
    },
    /// Replay serialized AgentHistory against an existing persistent session.
    Replay { id: String, history: PathBuf },
    /// Stop an existing persistent session.
    Stop { id: String },
    /// List recorded persistent sessions.
    List,
    /// Remove stale persistent session records, or force-clean a specific record.
    Cleanup {
        id: Option<String>,
        #[arg(long, default_value_t = false)]
        force: bool,
    },
}

pub(crate) type StoredSession = browser_use_mcp::SessionRecord;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SessionCleanupDecision {
    RemoveRecord,
    StopRunning,
    SkipRunning,
    SkipUnknown,
}

pub(crate) async fn start_persistent_session(
    id: &str,
    url: &str,
    screenshot: bool,
) -> anyhow::Result<(
    StoredSession,
    CdpBrowserSession,
    browser_use_dom::BrowserStateSummary,
)> {
    validate_session_id(id)?;
    let path = session_record_path(id)?;
    if path.exists() {
        anyhow::bail!("session already exists: {id}");
    }
    let user_data_dir = session_user_data_dir(id)?;
    std::fs::create_dir_all(&user_data_dir)?;
    let profile = BrowserProfile {
        user_data_dir: Some(user_data_dir.clone()),
        ..BrowserProfile::default()
    };
    let launched = profile.launch_local().await?;
    let endpoint = launched.endpoint().clone();
    let process_id = launched.process_id();
    let session = CdpBrowserSession::connect(endpoint.clone()).await?;
    session.navigate(url, false).await?;
    sleep(Duration::from_millis(150)).await;
    let state = session.state(screenshot).await?;
    let record = StoredSession {
        id: id.to_owned(),
        endpoint,
        user_data_dir,
        process_id,
        status: None,
    };
    write_session_record(&record)?;
    let _ = launched.detach();
    Ok((annotate_session_status(record), session, state))
}

pub(crate) async fn stop_persistent_session(id: &str) -> anyhow::Result<StoredSession> {
    let mut record = read_session_record(id)?;
    if let Ok(session) = CdpBrowserSession::connect(record.endpoint.clone()).await {
        let _ = session.close_browser().await;
    }
    wait_for_process_exit(record.process_id, Duration::from_secs(2)).await;
    remove_session_dir(id)?;
    record.status = Some(browser_use_mcp::SessionStatus::Stopped);
    Ok(record)
}

pub(crate) async fn cleanup_persistent_sessions(
    id: Option<&str>,
    force: bool,
) -> anyhow::Result<Vec<browser_use_mcp::SessionCleanupRecord>> {
    let records = if let Some(id) = id {
        vec![annotate_session_status(read_session_record(id)?)]
    } else {
        list_session_records()?
    };
    let mut cleaned = Vec::new();

    for record in records {
        match session_cleanup_decision(&record, force, process_is_running) {
            SessionCleanupDecision::RemoveRecord => {
                remove_session_dir(&record.id)?;
                cleaned.push(browser_use_mcp::SessionCleanupRecord {
                    action: browser_use_mcp::SessionCleanupAction::Removed,
                    session: record,
                });
            }
            SessionCleanupDecision::StopRunning => {
                let record = stop_persistent_session(&record.id).await?;
                cleaned.push(browser_use_mcp::SessionCleanupRecord {
                    action: browser_use_mcp::SessionCleanupAction::Stopped,
                    session: record,
                });
            }
            SessionCleanupDecision::SkipRunning if id.is_some() => {
                anyhow::bail!(
                    "session {} is running; use session stop or pass --force",
                    record.id
                );
            }
            SessionCleanupDecision::SkipUnknown if id.is_some() => {
                anyhow::bail!(
                    "session {} has unknown liveness; pass --force to remove the record",
                    record.id
                );
            }
            SessionCleanupDecision::SkipRunning | SessionCleanupDecision::SkipUnknown => {}
        }
    }

    Ok(cleaned)
}

pub(crate) fn session_cleanup_decision(
    record: &StoredSession,
    force: bool,
    is_running: impl Fn(u32) -> bool,
) -> SessionCleanupDecision {
    match session_status_with_checker(record, is_running) {
        browser_use_mcp::SessionStatus::Running if force => SessionCleanupDecision::StopRunning,
        browser_use_mcp::SessionStatus::Running => SessionCleanupDecision::SkipRunning,
        browser_use_mcp::SessionStatus::Stale | browser_use_mcp::SessionStatus::Stopped => {
            SessionCleanupDecision::RemoveRecord
        }
        browser_use_mcp::SessionStatus::Unknown if force => SessionCleanupDecision::RemoveRecord,
        browser_use_mcp::SessionStatus::Unknown => SessionCleanupDecision::SkipUnknown,
    }
}

pub(crate) async fn run_session_command(command: SessionCommand) -> anyhow::Result<()> {
    match command {
        SessionCommand::Start {
            id,
            url,
            screenshot,
        } => {
            let (record, _session, state) = start_persistent_session(&id, &url, screenshot).await?;
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "session": record,
                    "state": state
                }))?
            );
        }
        SessionCommand::State { id, screenshot } => {
            let record = annotate_session_status(read_session_record(&id)?);
            let session = CdpBrowserSession::connect(record.endpoint).await?;
            print_state(&session, screenshot).await?;
        }
        SessionCommand::Actions {
            id,
            actions,
            screenshot,
        } => {
            let record = annotate_session_status(read_session_record(&id)?);
            let session = CdpBrowserSession::connect(record.endpoint.clone()).await?;
            let actions = std::fs::read_to_string(&actions)?;
            let actions: Vec<browser_use_tools::BrowserAction> = serde_json::from_str(&actions)?;
            let mut executor = BrowserActionExecutor::new(session);
            let results = executor.execute_sequence(&actions).await;
            let state = executor.session().state(screenshot).await?;
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "session": record,
                    "results": results,
                    "state": state,
                }))?
            );
        }
        SessionCommand::Replay { id, history } => {
            let record = annotate_session_status(read_session_record(&id)?);
            let session = CdpBrowserSession::connect(record.endpoint.clone()).await?;
            let history = read_agent_history(&history)?;
            let mut executor = BrowserActionExecutor::new(session);
            let replay = executor.replay_history(&history).await?;
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "session": record,
                    "replay": replay,
                }))?
            );
        }
        SessionCommand::Stop { id } => {
            let record = stop_persistent_session(&id).await?;
            println!("{}", serde_json::to_string_pretty(&record)?);
        }
        SessionCommand::List => {
            println!(
                "{}",
                serde_json::to_string_pretty(&list_session_records()?)?
            );
        }
        SessionCommand::Cleanup { id, force } => {
            let cleaned = cleanup_persistent_sessions(id.as_deref(), force).await?;
            let remaining = list_session_records()?;
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "cleaned_sessions": cleaned,
                    "sessions": remaining,
                }))?
            );
        }
    }

    Ok(())
}

pub(crate) fn read_agent_history(path: &PathBuf) -> anyhow::Result<AgentHistory> {
    let history = std::fs::read_to_string(path)?;
    Ok(serde_json::from_str(&history)?)
}

fn validate_session_id(id: &str) -> anyhow::Result<()> {
    if id.is_empty()
        || !id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        anyhow::bail!("session id must contain only ASCII letters, digits, '-' or '_'");
    }
    Ok(())
}

fn state_dir() -> anyhow::Result<PathBuf> {
    if let Some(path) = std::env::var_os("BROWSER_USE_RS_STATE_DIR") {
        return Ok(PathBuf::from(path));
    }
    let home = std::env::var_os("HOME").ok_or_else(|| anyhow::anyhow!("HOME is not set"))?;
    Ok(PathBuf::from(home).join(".browser-use-rs"))
}

fn sessions_dir() -> anyhow::Result<PathBuf> {
    Ok(state_dir()?.join("sessions"))
}

fn session_dir(id: &str) -> anyhow::Result<PathBuf> {
    validate_session_id(id)?;
    Ok(sessions_dir()?.join(id))
}

fn session_user_data_dir(id: &str) -> anyhow::Result<PathBuf> {
    Ok(session_dir(id)?.join("profile"))
}

pub(crate) fn session_record_path(id: &str) -> anyhow::Result<PathBuf> {
    Ok(session_dir(id)?.join("session.json"))
}

fn write_session_record(record: &StoredSession) -> anyhow::Result<()> {
    let path = session_record_path(&record.id)?;
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("session path has no parent"))?;
    let mut stored_record = record.clone();
    stored_record.status = None;
    std::fs::create_dir_all(parent)?;
    std::fs::write(path, serde_json::to_vec_pretty(&stored_record)?)?;
    Ok(())
}

pub(crate) fn read_session_record(id: &str) -> anyhow::Result<StoredSession> {
    let path = session_record_path(id)?;
    let contents = std::fs::read_to_string(&path)?;
    Ok(serde_json::from_str(&contents)?)
}

fn remove_session_dir(id: &str) -> anyhow::Result<()> {
    let path = session_dir(id)?;
    if path.exists() {
        std::fs::remove_dir_all(path)?;
    }
    Ok(())
}

pub(crate) fn list_session_records() -> anyhow::Result<Vec<StoredSession>> {
    let dir = sessions_dir()?;
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut records = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let id = entry.file_name().to_string_lossy().to_string();
        if let Ok(record) = read_session_record(&id) {
            records.push(annotate_session_status(record));
        }
    }
    records.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(records)
}

pub(crate) fn annotate_session_status(mut record: StoredSession) -> StoredSession {
    record.status = Some(session_status(&record));
    record
}

fn session_status(record: &StoredSession) -> browser_use_mcp::SessionStatus {
    session_status_with_checker(record, process_is_running)
}

pub(crate) fn session_status_with_checker(
    record: &StoredSession,
    is_running: impl Fn(u32) -> bool,
) -> browser_use_mcp::SessionStatus {
    match record.process_id {
        Some(process_id) if is_running(process_id) => browser_use_mcp::SessionStatus::Running,
        Some(_) => browser_use_mcp::SessionStatus::Stale,
        None => browser_use_mcp::SessionStatus::Unknown,
    }
}

async fn wait_for_process_exit(process_id: Option<u32>, timeout: Duration) {
    let Some(process_id) = process_id else {
        return;
    };
    let deadline = Instant::now() + timeout;
    while process_is_running(process_id) && Instant::now() < deadline {
        sleep(Duration::from_millis(50)).await;
    }
}

#[cfg(unix)]
fn process_is_running(process_id: u32) -> bool {
    StdCommand::new("kill")
        .arg("-0")
        .arg(process_id.to_string())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn process_is_running(_process_id: u32) -> bool {
    false
}
