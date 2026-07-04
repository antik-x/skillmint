// Browser-only mock of @tauri-apps/api/core for click-testing the real React UI
// against the live SQLite DB via tools/mock_server.py. Not used in the Tauri
// build (only active when vite resolves this path via the alias in
// vite.config.web.ts).
const BASE = "http://127.0.0.1:18220";

export async function invoke<T>(
  cmd: string,
  args?: Record<string, unknown>
): Promise<T> {
  const res = await fetch(`${BASE}/invoke`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ cmd, args: args ?? {} }),
  });
  if (!res.ok) {
    const txt = await res.text();
    throw new Error(`invoke(${cmd}) → ${res.status}: ${txt}`);
  }
  const data = await res.json();
  return data as T;
}

export const transformCallback = () => 0;
export const convertFileSrc = (src: string) => src;
