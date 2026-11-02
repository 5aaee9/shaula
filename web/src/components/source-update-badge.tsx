import type { SourceUpdate } from "@/lib/types";
import { Badge } from "./ui/badge";

/** Only differences from an up-to-date default source are shown. */
export function SourceUpdateBadge({ update }: { update?: SourceUpdate | null }) {
  if (update?.state === "Available")
    return (
      <Badge variant="outline" className="border-sky-200 bg-sky-50 text-sky-700">
        Update available
      </Badge>
    );
  if (update?.state === "Unavailable")
    return (
      <Badge variant="outline" className="border-amber-200 bg-amber-50 text-amber-800">
        Source unavailable
      </Badge>
    );
  return null;
}
