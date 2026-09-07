import { Children, cloneElement, isValidElement, useId, type ReactNode } from "react";
import { AlertCircle, Inbox } from "lucide-react";
import { errorMessage } from "@/lib/api";
import { Alert, AlertDescription } from "./ui/alert";
import { Badge } from "./ui/badge";
import { Button } from "./ui/button";
import { Empty as EmptyRoot, EmptyHeader, EmptyMedia, EmptyTitle, EmptyContent } from "./ui/empty";
import { Field as FieldRoot, FieldLabel } from "./ui/field";
import { Skeleton } from "./ui/skeleton";

export function StatusBadge({ value }: { value: string }) {
  const tone = /^(ready|active|converged|passed)$/i.test(value)
    ? "border-emerald-200 bg-emerald-50 text-emerald-700"
    : /fail|reject|block|error|absent/i.test(value)
      ? "border-rose-200 bg-rose-50 text-rose-700"
      : /pending|validat|retir|drain|accept|start|progress/i.test(value)
        ? "border-amber-200 bg-amber-50 text-amber-800"
        : "";
  return (
    <Badge variant="outline" className={tone}>
      <span className="size-1.5 shrink-0 rounded-full bg-current" />
      {value}
    </Badge>
  );
}
export function ErrorNotice({ error, retry }: { error: unknown; retry?: () => void }) {
  return (
    <Alert variant="destructive" className="my-4">
      <AlertCircle />
      <AlertDescription className="flex min-w-0 items-center justify-between gap-3">
        <span className="break-words">{errorMessage(error)}</span>
        {retry && (
          <Button variant="outline" size="sm" onClick={retry}>
            Retry
          </Button>
        )}
      </AlertDescription>
    </Alert>
  );
}
export function Loading() {
  return (
    <div role="status" className="space-y-4 py-8">
      <span className="sr-only">Loading...</span>
      <Skeleton className="h-5 w-36" />
      <Skeleton className="h-12 w-full" />
      <Skeleton className="h-12 w-full" />
      <Skeleton className="h-12 w-full" />
    </div>
  );
}
export function Empty({ title, children }: { title: string; children?: ReactNode }) {
  return (
    <EmptyRoot className="min-h-60 rounded-none border-b">
      <EmptyHeader>
        <EmptyMedia variant="icon">
          <Inbox />
        </EmptyMedia>
        <EmptyTitle className="text-base">{title}</EmptyTitle>
      </EmptyHeader>
      {children && <EmptyContent>{children}</EmptyContent>}
    </EmptyRoot>
  );
}
export function Field({ label, children }: { label: string; children: ReactNode }) {
  const id = useId();
  return (
    <FieldRoot className="min-w-0 gap-2">
      <FieldLabel htmlFor={id}>{label}</FieldLabel>
      {Children.map(children, (child, index) =>
        index === 0 && isValidElement<{ id?: string }>(child) ? cloneElement(child, { id }) : child,
      )}
    </FieldRoot>
  );
}
export function KeyValue({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="key-value">
      <dt>{label}</dt>
      <dd>{children}</dd>
    </div>
  );
}
