import { useEffect, useRef, useState, type ReactNode } from "react";

interface PageTransitionProps {
  children: ReactNode;
  trigger: string;
}

/**
 * Apple-style cross-fade + slide transition for tab switching.
 * `trigger` should be the active tab id; whenever it changes the
 * current child fades/slides out and the new child fades/slides in.
 */
export function PageTransition({ children, trigger }: PageTransitionProps) {
  const [displayChildren, setDisplayChildren] = useState(children);
  const [phase, setPhase] = useState<"idle" | "exiting" | "entering">("idle");
  const prevTriggerRef = useRef(trigger);

  useEffect(() => {
    if (trigger === prevTriggerRef.current) return;

    setPhase("exiting");
    const exitTimer = setTimeout(() => {
      setDisplayChildren(children);
      prevTriggerRef.current = trigger;
      setPhase("entering");
      const enterTimer = setTimeout(() => {
        setPhase("idle");
      }, 200);
      return () => clearTimeout(enterTimer);
    }, 150);

    return () => clearTimeout(exitTimer);
  }, [trigger, children]);

  const animationClass =
    phase === "exiting"
      ? "page-exit-active"
      : phase === "entering"
      ? "page-enter-active"
      : "";

  return (
    <div
      className={`h-full w-full ${animationClass}`}
      style={{
        willChange: phase === "idle" ? "auto" : "transform, opacity",
      }}
    >
      {displayChildren}
    </div>
  );
}
