import { useState, useRef, useEffect, useId } from "react";
import { createPortal } from "react-dom";
export function TooltipButton({
  entry,
  fallback,
  content,
  children,
  className,
  onClick,
}: any) {
  const trigger = useRef<HTMLButtonElement>(null),
    hideTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const [tooltip, setTooltip] = useState<null | { top: number; left: number }>(
      null,
    ),
    id = useId();
  const cancelHide = () => {
    if (hideTimer.current) clearTimeout(hideTimer.current);
  };
  const hide = () => {
    cancelHide();
    hideTimer.current = setTimeout(() => {
      if (document.activeElement !== trigger.current) setTooltip(null);
    }, 150);
  };
  useEffect(() => () => cancelHide(), []);
  const position = () => {
    const rect = trigger.current?.getBoundingClientRect();
    if (!rect || rect.bottom < 0 || rect.top > window.innerHeight) {
      setTooltip(null);
      return;
    }
    setTooltip({
      left: Math.max(
        12,
        Math.min(
          rect.left,
          window.innerWidth - Math.min(440, window.innerWidth - 24) - 12,
        ),
      ),
      top: Math.max(
        12,
        Math.min(
          rect.bottom + 8,
          window.innerHeight - Math.min(400, window.innerHeight * 0.5) - 12,
        ),
      ),
    });
  };
  useEffect(() => {
    if (!tooltip) return;
    const update = (event: Event) => {
      if (
        event.target instanceof Node &&
        document.getElementById(id)?.contains(event.target)
      )
        return;
      position();
    };
    window.addEventListener("scroll", update, true);
    window.addEventListener("resize", update);
    return () => {
      window.removeEventListener("scroll", update, true);
      window.removeEventListener("resize", update);
    };
  }, [tooltip !== null]);
  const show = () => {
    cancelHide();
    position();
  };
  return (
    <>
      <button
        ref={trigger}
        className={className}
        aria-describedby={tooltip ? id : undefined}
        onMouseEnter={show}
        onMouseLeave={hide}
        onFocus={show}
        onBlur={() => setTooltip(null)}
        onClick={(e) => {
          if (onClick) {
            setTooltip(null);
            onClick();
          } else show();
        }}
      >
        {children}
      </button>
      {tooltip &&
        createPortal(
          <div
            className="ability-tooltip"
            id={id}
            role="tooltip"
            onMouseEnter={cancelHide}
            onMouseLeave={hide}
            style={tooltip}
          >
            <strong>{entry?.name ?? fallback}</strong>
            {content ?? <p>
              {entry?.description?.replaceAll("{TIMEZONE}", "время сервера") ??
                "Описание отсутствует в текущем справочнике."}
            </p>}
          </div>,
          document.body,
        )}
    </>
  );
}
