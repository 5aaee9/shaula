import { useId, type ReactNode } from "react";
import { flushSync } from "react-dom";
import { ChevronDown } from "lucide-react";
import { Button } from "./ui/button";

/** Keep drafts mounted and reveal invalid controls before the browser focuses them. */
export function AdvancedSettings({
  open,
  onOpenChange,
  summary,
  children,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  summary?: string;
  children: ReactNode;
}) {
  const id = useId();
  return (
    <section className="border-t pt-2">
      <Button
        type="button"
        variant="ghost"
        className="w-full justify-between px-0 hover:bg-transparent"
        aria-expanded={open}
        aria-controls={id}
        aria-describedby={summary ? `${id}-summary` : undefined}
        onClick={() => onOpenChange(!open)}
      >
        Advanced settings
        <ChevronDown aria-hidden="true" className={open ? "rotate-180" : ""} />
      </Button>
      {summary && (
        <p id={`${id}-summary`} className="text-xs text-muted-foreground break-words">
          {summary}
        </p>
      )}
      <div
        id={id}
        hidden={!open}
        onInvalidCapture={() => {
          if (!open) flushSync(() => onOpenChange(true));
        }}
      >
        <div className="form-stack pt-4">{children}</div>
      </div>
    </section>
  );
}
