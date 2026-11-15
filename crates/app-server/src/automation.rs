//! Closed start/stop protocol to the separately elevated helper. Never shell text or paths.
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    path::Path,
    time::{Duration, SystemTime},
};
pub fn status(dir: Option<&Path>) -> Value {
    let Some(dir) = dir else {
        return json!({"configured":false,"enabled":false,"watcher_alive":false,"state":"not_configured"});
    };
    let portable_root = dir
        .parent()
        .and_then(Path::parent)
        .filter(|root| root.join("portable.json").is_file());
    let script =
        portable_root.map(|root| root.join("components/scripts/Configure-AutoCapture.ps1"));
    let command = script.as_ref().map(|p| format!("powershell.exe -NoProfile -ExecutionPolicy RemoteSigned -File \"{}\"", p.display()))
        .unwrap_or_else(|| "powershell.exe -NoProfile -ExecutionPolicy RemoteSigned -File .\\scripts\\Configure-AutoCapture.ps1".into());
    let configured = dir.join("configured.json").is_file();
    let enabled = configured && !dir.join("disabled").exists();
    let path = dir.join("status.json");
    let metadata = std::fs::metadata(&path).ok();
    let recent = metadata
        .as_ref()
        .and_then(|m| m.modified().ok())
        .and_then(|t| SystemTime::now().duration_since(t).ok())
        .is_some_and(|age| age < Duration::from_secs(20));
    let data = metadata
        .filter(|m| m.len() <= 8192)
        .and_then(|_| std::fs::read(&path).ok())
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .unwrap_or(Value::Null);
    let alive = recent && data["schema_version"] == 1 && data["state"] != "disabled";
    json!({"configured":configured,"enabled":enabled,"watcher_alive":alive,
        "state":if !enabled {if configured{"disabled"}else{"not_configured"}}else if !alive{"watcher_unavailable"}else{data["state"].as_str().unwrap_or("unknown")},
        "updated_at":data["updated_at"].as_str(),"last_exit_code":data["last_exit_code"].as_i64(),
        "control_version":data["control_version"],"control_available":alive&&data["control_version"]==1,
        "watcher_instance":data["watcher_instance"],"capture_run_id":data["capture_run_id"],
        "capture_state":data["state"],"capture_id":data["capture_id"],"external_capture":data["external_capture"]==true,
        "last_command":data["last_command"],"command_pending":dir.join("command.json").exists(),
        "portable":portable_root.is_some(),
        "installation_command":command,
        "disable_command":format!("{command} -Disable")})
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Start,
    Stop,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ControlRequest {
    pub request_id: uuid::Uuid,
    pub action: Action,
    pub target_run_id: Option<uuid::Uuid>,
}
#[derive(Debug)]
pub struct ControlError(pub StatusCode, pub &'static str);
fn error(code: &'static str) -> ControlError {
    ControlError(StatusCode::CONFLICT, code)
}
pub fn request(dir: Option<&Path>, req: ControlRequest) -> Result<Value, ControlError> {
    let dir = dir.ok_or_else(|| error("helper_unavailable"))?;
    let status = status(Some(dir));
    if status["control_available"] != true {
        return Err(error(if status["watcher_alive"] == true {
            "helper_upgrade_required"
        } else {
            "helper_unavailable"
        }));
    }
    if req.request_id.is_nil() {
        return Err(ControlError(StatusCode::BAD_REQUEST, "invalid_request_id"));
    }
    let id = req.request_id.to_string();
    let action = serde_json::to_value(&req.action).unwrap();
    if status["last_command"]["request_id"] == id {
        if status["last_command"]["action"] != action
            || status["last_command"]["target_run_id"]
                != req
                    .target_run_id
                    .map(|u| u.to_string())
                    .map(Value::String)
                    .unwrap_or(Value::Null)
        {
            return Err(error("request_id_conflict"));
        }
        return Ok(json!({"request_id":id,"queued":false,"result":status["last_command"]}));
    }
    match req.action {
        Action::Start => {
            if req.target_run_id.is_some() {
                return Err(ControlError(StatusCode::BAD_REQUEST, "invalid_target"));
            }
            if status["capture_run_id"].is_string() || status["external_capture"] == true {
                return Err(error("already_running"));
            }
        }
        Action::Stop => {
            if req.target_run_id.is_none() {
                return Err(ControlError(StatusCode::BAD_REQUEST, "invalid_target"));
            }
            if status["capture_run_id"].as_str()
                != req.target_run_id.map(|u| u.to_string()).as_deref()
            {
                return Err(error("run_changed"));
            }
        }
    }
    let instance = status["watcher_instance"]
        .as_str()
        .and_then(|s| uuid::Uuid::parse_str(s).ok())
        .ok_or_else(|| error("helper_unavailable"))?;
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_err(|_| error("clock_error"))?
        .as_millis();
    let command = json!({"schema_version":1,"request_id":id,"watcher_instance":instance.to_string(),"action":action,"target_run_id":req.target_run_id.map(|u|u.to_string()),"expires_unix_ms":(now+30000).to_string()});
    let destination = dir.join("command.json");
    if let Ok(bytes) = std::fs::read(&destination) {
        let old: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        if old["request_id"] == id
            && old["action"] == action
            && old["target_run_id"] == command["target_run_id"]
        {
            return Ok(json!({"request_id":id,"queued":true}));
        }
        return Err(error("command_pending"));
    }
    // A complete, flushed command becomes visible atomically, without overwriting a concurrent request.
    let temporary = dir.join(format!("command-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| -> std::io::Result<()> {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(&serde_json::to_vec(&command)?)?;
        file.sync_all()?;
        drop(file);
        std::fs::hard_link(&temporary, &destination)
    })();
    let _ = std::fs::remove_file(&temporary);
    result.map_err(|e| {
        if e.kind() == std::io::ErrorKind::AlreadyExists {
            error("command_pending")
        } else {
            ControlError(
                StatusCode::SERVICE_UNAVAILABLE,
                "control_storage_unavailable",
            )
        }
    })?;
    Ok(json!({"request_id":id,"queued":true}))
}
#[cfg(test)]
mod tests {
    use super::*;
    fn helper(path: &Path, run: Option<uuid::Uuid>) {
        std::fs::write(path.join("configured.json"), "{}").unwrap();
        std::fs::write(path.join("status.json"),json!({"schema_version":1,"control_version":1,"watcher_instance":uuid::Uuid::new_v4(),"state":"waiting","capture_run_id":run,"external_capture":false}).to_string()).unwrap();
    }
    #[test]
    fn commands_are_atomic_bounded_and_require_the_current_run() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        helper(dir, None);
        let start = ControlRequest {
            request_id: uuid::Uuid::new_v4(),
            action: Action::Start,
            target_run_id: None,
        };
        request(Some(dir), start.clone()).unwrap();
        request(Some(dir), start).unwrap();
        let next = ControlRequest {
            request_id: uuid::Uuid::new_v4(),
            action: Action::Start,
            target_run_id: None,
        };
        assert_eq!(request(Some(dir), next).unwrap_err().1, "command_pending");
        let contents: Value =
            serde_json::from_slice(&std::fs::read(dir.join("command.json")).unwrap()).unwrap();
        assert_eq!(contents["action"], "start");
        assert_eq!(std::fs::read_dir(dir).unwrap().count(), 3);
        std::fs::remove_file(dir.join("command.json")).unwrap();
        let current = uuid::Uuid::new_v4();
        helper(dir, Some(current));
        let late = ControlRequest {
            request_id: uuid::Uuid::new_v4(),
            action: Action::Stop,
            target_run_id: Some(uuid::Uuid::new_v4()),
        };
        assert_eq!(request(Some(dir), late).unwrap_err().1, "run_changed");
        let stop = ControlRequest {
            request_id: uuid::Uuid::new_v4(),
            action: Action::Stop,
            target_run_id: Some(current),
        };
        request(Some(dir), stop).unwrap();
    }
    #[test]
    fn concurrent_requests_never_overwrite_each_other() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        helper(dir, None);
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let path = dir.to_path_buf();
                std::thread::spawn(move || {
                    request(
                        Some(&path),
                        ControlRequest {
                            request_id: uuid::Uuid::new_v4(),
                            action: Action::Start,
                            target_run_id: None,
                        },
                    )
                })
            })
            .collect();
        let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
        let body: Value =
            serde_json::from_slice(&std::fs::read(dir.join("command.json")).unwrap()).unwrap();
        assert_eq!(body["action"], "start");
        assert_eq!(std::fs::read_dir(dir).unwrap().count(), 3);
    }
    #[test]
    fn configuration_heartbeat_and_disable_are_distinct() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path();
        assert_eq!(status(Some(path))["state"], "not_configured");
        std::fs::write(path.join("configured.json"), "{}").unwrap();
        assert_eq!(status(Some(path))["state"], "watcher_unavailable");
        std::fs::write(
            path.join("status.json"),
            r#"{"schema_version":1,"state":"waiting","last_exit_code":0}"#,
        )
        .unwrap();
        assert_eq!(status(Some(path))["watcher_alive"], true);
        std::fs::write(path.join("disabled"), "").unwrap();
        assert_eq!(status(Some(path))["enabled"], false);
        assert_eq!(status(Some(path))["state"], "disabled");
        std::fs::write(path.join("status.json"), "partial {").unwrap();
        assert_eq!(status(Some(path))["watcher_alive"], false);
    }
}
