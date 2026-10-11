import { inputsJson } from "./input-values";
import { isSensitiveMarker } from "./types";

/**
 * Initial bindings JSON for an Update review. Non-sensitive fields prefill with
 * their stored value; sensitive fields are omitted so the server keeps them
 * (spec 0038: omitted/`null` sensitive field = retain base value).
 */
export function initialUpdateBindings(
  revisionBindings: Record<string, unknown> | null | undefined,
): string {
  const values = new Map<string, string>();
  for (const [key, value] of Object.entries(revisionBindings ?? {})) {
    if (isSensitiveMarker(value)) continue;
    values.set(key, JSON.stringify(value));
  }
  return inputsJson(values);
}
