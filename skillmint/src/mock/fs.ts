// Browser-only mock of @tauri-apps/plugin-fs for click-testing the web build.
export async function writeTextFile(path: string, contents: string): Promise<void> {
  console.log("[mock-fs] writeTextFile", path, contents.length);
}

export async function readTextFile(_path: string): Promise<string> {
  return "";
}

export async function exists(_path: string): Promise<boolean> {
  return false;
}

export async function mkdir(_path: string, _options?: unknown): Promise<void> {}

export async function remove(_path: string, _options?: unknown): Promise<void> {}
