// Browser-only mock of @tauri-apps/plugin-dialog. The native file/folder picker
// can't open in a browser, so these return null (the UI treats null as
// "user cancelled"). For click-testing a flow, temporarily return a real path.
export async function open(): Promise<string | null> {
  return null;
}

export async function save(): Promise<string | null> {
  return null;
}
