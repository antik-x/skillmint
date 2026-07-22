use super::*;

#[tauri::command]
pub fn scan_agents(state: State<'_, AppState>) -> Result<Vec<Agent>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    scan_and_persist_agents(&db).map_err(|e| e.to_string())
}

