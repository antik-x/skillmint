import { useEffect, useRef, useCallback, type ReactNode } from "react";
import { cn } from "./utils";
import { useHotkeyScope } from "../../hooks/useHotkeys";

export interface DialogProps {
  open: boolean;
  onClose: () => void;
  onSubmit?: () => void;
  /** Whether Enter should trigger onSubmit. Default true when onSubmit is provided. */
  submitOnEnter?: boolean;
  title?: ReactNode;
  description?: ReactNode;
  children: ReactNode;
  className?: string;
  /** Accessible label id for the dialog. */
  labelId?: string;
  /** Do not close on backdrop click when true. */
  disableBackdropClick?: boolean;
  /** Do not close on Esc when true. */
  disableEscape?: boolean;
}

const FOCUSABLE_SELECTORS = [
  'button:not([disabled])',
  'input:not([disabled])',
  'textarea:not([disabled])',
  'select:not([disabled])',
  '[href]',
  '[tabindex]:not([tabindex="-1"])',
].join(", ");

export function Dialog({
  open,
  onClose,
  onSubmit,
  submitOnEnter = !!onSubmit,
  title,
  description,
  children,
  className,
  labelId,
  disableBackdropClick = false,
  disableEscape = false,
}: DialogProps) {
  const dialogRef = useRef<HTMLDivElement>(null);
  const previousFocusRef = useRef<HTMLElement | null>(null);
  const labelIdRef = useRef(labelId ?? `dialog-title-${Math.random().toString(36).slice(2, 8)}`);
  const descriptionIdRef = useRef(`dialog-desc-${Math.random().toString(36).slice(2, 8)}`);

  useHotkeyScope(open ? "dialog" : undefined);

  // Save and restore focus.
  useEffect(() => {
    if (open) {
      previousFocusRef.current = document.activeElement as HTMLElement | null;
      return () => {
        previousFocusRef.current?.focus?.();
      };
    }
  }, [open]);

  // Focus first focusable element when opened.
  useEffect(() => {
    if (!open || !dialogRef.current) return;
    const timer = requestAnimationFrame(() => {
      const el = dialogRef.current?.querySelector<HTMLElement>(FOCUSABLE_SELECTORS);
      if (el) {
        el.focus();
      } else {
        dialogRef.current?.focus();
      }
    });
    return () => cancelAnimationFrame(timer);
  }, [open]);

  // Focus trap + Esc + Enter.
  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (!dialogRef.current) return;

      if (e.key === "Escape" && !disableEscape) {
        e.preventDefault();
        e.stopPropagation();
        onClose();
        return;
      }

      if (e.key === "Enter" && submitOnEnter && onSubmit) {
        const target = e.target as HTMLElement;
        const tag = target.tagName.toLowerCase();
        if (tag !== "textarea") {
          e.preventDefault();
          onSubmit();
          return;
        }
      }

      if (e.key !== "Tab") return;

      const focusable = Array.from(
        dialogRef.current.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTORS)
      );
      if (focusable.length === 0) return;

      const first = focusable[0];
      const last = focusable[focusable.length - 1];

      if (e.shiftKey && document.activeElement === first) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && document.activeElement === last) {
        e.preventDefault();
        first.focus();
      }
    },
    [disableEscape, onClose, onSubmit, submitOnEnter]
  );

  const handleBackdropClick = useCallback(
    (e: React.MouseEvent) => {
      if (e.target === e.currentTarget && !disableBackdropClick) {
        onClose();
      }
    },
    [disableBackdropClick, onClose]
  );

  if (!open) return null;

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4"
      onClick={handleBackdropClick}
      onKeyDown={handleKeyDown}
      role="presentation"
    >
      <div
        ref={dialogRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby={title ? labelIdRef.current : undefined}
        aria-describedby={description ? descriptionIdRef.current : undefined}
        tabIndex={-1}
        className={cn(
          "max-h-[85vh] w-full max-w-lg overflow-auto rounded-xl bg-primary p-6 shadow-2xl outline-none",
          className
        )}
      >
        {title && (
          <h2 id={labelIdRef.current} className="mb-1 text-lg font-semibold text-primary">
            {title}
          </h2>
        )}
        {description && (
          <p id={descriptionIdRef.current} className="mb-4 text-sm text-secondary">
            {description}
          </p>
        )}
        {children}
      </div>
    </div>
  );
}

export function DialogActions({
  children,
  className,
}: {
  children: ReactNode;
  className?: string;
}) {
  return <div className={cn("mt-5 flex justify-end gap-2", className)}>{children}</div>;
}
