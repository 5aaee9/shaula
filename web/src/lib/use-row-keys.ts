import { useState } from "react";

/**
 * Stable React keys for a list the user edits by appending and removing rows.
 * Rows carry no identity of their own (their fields are user-editable and the
 * row objects are submitted verbatim), so keys live beside them, index-aligned.
 */
export function useRowKeys(length: number) {
  const [keys, setKeys] = useState(() => Array.from({ length }, () => crypto.randomUUID()));
  return {
    keys,
    append: () => setKeys((current) => [...current, crypto.randomUUID()]),
    remove: (index: number) => setKeys((current) => current.filter((_, i) => i !== index)),
  };
}
