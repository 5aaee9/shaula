import type { ReactNode } from "react";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "./ui/dialog";

export function Modal({
  title,
  description,
  children,
  open,
  onOpenChange,
  inline = false,
}: {
  title: string;
  description?: string;
  children: ReactNode;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  inline?: boolean;
}) {
  if (inline)
    return (
      <div
        role="dialog"
        aria-modal="false"
        className="mx-auto w-full max-w-5xl rounded-lg border bg-card p-6 shadow-sm"
      >
        {children}
      </div>
    );
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        {...(!description ? { "aria-describedby": undefined } : {})}
        className="max-h-[90dvh] overflow-y-auto sm:max-w-xl"
      >
        <DialogHeader>
          <DialogTitle className="pr-6 break-words">{title}</DialogTitle>
          {description && <DialogDescription>{description}</DialogDescription>}
        </DialogHeader>
        {children}
      </DialogContent>
    </Dialog>
  );
}
