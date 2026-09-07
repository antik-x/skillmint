use super::*;

#[tauri::command(async)]
pub fn scan_agents(state: State<'_, AppState>) -> Result<Vec<Agent>, String> {
    let _t = CmdTimer::new("scan_agents");
    let db = state.db.lock().map_err(|e| e.to_string())?;
    scan_and_persist_agents(&db).map_err(|e| e.to_string())
}

