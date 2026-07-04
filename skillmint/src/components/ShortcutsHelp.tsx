import { Dialog } from "./ui/Dialog";
import { useHotkeysRegistry } from "../hooks/useHotkeys";

export interface ShortcutsHelpProps {
  open: boolean;
  onClose: () => void;
}

function isMac(): boolean {
  if (typeof navigator === "undefined") return false;
  return navigator.platform.toLowerCase().includes("mac");
}

export function ShortcutsHelp({ open, onClose }: ShortcutsHelpProps) {
  const shortcuts = useHotkeysRegistry();

  return (
    <Dialog open={open} onClose={onClose} title="键盘快捷键">
      <div className="space-y-2">
        {shortcuts.length === 0 ? (
          <p className="text-sm text-secondary">暂无已注册快捷键。</p>
        ) : (
          shortcuts
            .filter((s) => s.description)
            .map((s) => (
              <div key={s.id} className="flex items-center justify-between py-1">
                <span className="text-sm text-secondary">{s.description}</span>
                <kbd className="rounded border border-[var(--border-subtle)] px-2 py-0.5 text-xs font-mono text-primary">
                  {s.combo.replace("mod", isMac() ? "⌘" : "Ctrl")}
                </kbd>
              </div>
            ))
        )}
      </div>
    </Dialog>
  );
}
