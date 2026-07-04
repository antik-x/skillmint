import { useState } from "react";
import { CheckCircle2, Info, X, XCircle } from "lucide-react";
import { useToastStore, type Toast } from "../stores/toastStore";
import { cn } from "./ui/utils";

const icons = {
  success: CheckCircle2,
  error: XCircle,
  info: Info,
};

const styles = {
  success: "border-success/20 bg-success/10 text-success",
  error: "border-danger/20 bg-danger/10 text-danger",
  info: "border-accent/20 bg-accent/10 text-accent",
};

export default function ToastContainer() {
  const toasts = useToastStore((state) => state.toasts);
  const removeToast = useToastStore((state) => state.removeToast);

  return (
    <div className="fixed bottom-6 right-6 z-[100] flex flex-col gap-2">
      {toasts.map((toast) => {
        const Icon = icons[toast.type];
        return (
          <ToastItem key={toast.id} toast={toast} Icon={Icon} onClose={() => removeToast(toast.id)} />
        );
      })}
    </div>
  );
}

function ToastItem({
  toast,
  Icon,
  onClose,
}: {
  toast: Toast;
  Icon: React.ElementType;
  onClose: () => void;
}) {
  const [showDetail, setShowDetail] = useState(false);

  return (
    <div
      className={cn(
        "toast-enter surface-glass flex min-w-[260px] max-w-md items-start gap-3 rounded-xl border px-4 py-3 shadow-lg transition-all duration-300",
        styles[toast.type]
      )}
    >
      <Icon className="mt-0.5 h-5 w-5 shrink-0" />
      <div className="flex flex-1 flex-col gap-2">
        <span className="text-sm font-medium">{toast.message}</span>
        {toast.detail && (
          <button
            onClick={() => setShowDetail((v) => !v)}
            className="self-start text-xs opacity-80 hover:underline"
          >
            {showDetail ? "隐藏详情" : "查看详情"}
          </button>
        )}
        {showDetail && toast.detail && (
          <div className="max-h-32 overflow-auto rounded-md bg-primary/50 p-2 text-xs break-all opacity-80">
            {toast.detail}
          </div>
        )}
        {toast.action && (
          <button
            onClick={() => {
              toast.action?.onClick();
              onClose();
            }}
            className="self-start rounded-md bg-accent/20 px-2.5 py-1 text-xs font-medium text-accent hover:bg-accent/30 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--accent)]"
          >
            {toast.action.label}
          </button>
        )}
      </div>
      <button
        onClick={onClose}
        className="shrink-0 rounded-md p-1 text-current opacity-60 hover:bg-black/5 hover:opacity-100 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--accent)]"
        aria-label="关闭"
      >
        <X className="h-4 w-4" />
      </button>
    </div>
  );
}
