import { ArrowUp } from "lucide-react";
import type { SourceUpdate } from "@/lib/types";
import { Badge } from "./ui/badge";

/**
 * A clickable "Update" chip when the recorded default source has new content,
 * a warning badge when it is gone, and nothing while it is current.
 */
export function SourceUpdateStatus({
  profileKey,
  update,
  disabled,
  onUpdate,
}: {
  profileKey: string;
  update?: SourceUpdate | null;
  disabled: boolean;
  onUpdate: () => void;
}) {
  if (update?.state === "Available")
    return (
      <button
        type="button"
        disabled={disabled}
        aria-label={`Update ${profileKey} from default`}
        title={update.targetDigest ?? undefined}
        onClick={onUpdate}
        className="inline-flex h-6 shrink-0 items-center gap-1 rounded-full border border-primary/20 bg-primary/5 px-2.5 text-xs font-medium text-primary transition-colors hover:border-primary/40 hover:bg-primary/10 focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:outline-none disabled:pointer-events-none disabled:opacity-50"
      >
        <ArrowUp className="size-3" aria-hidden="true" />
        Update
      </button>
    );
  if (update?.state === "Unavailable")
    return (
      <Badge variant="outline" className="border-amber-200 bg-amber-50 text-amber-800">
        Source unavailable
      </Badge>
    );
  return null;
}
