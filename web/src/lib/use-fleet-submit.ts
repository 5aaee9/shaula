import { useState, type FormEvent } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { api, MutationAttempt, resourcePath, type Resource } from "./api";
import type { Accepted, ChangeRef } from "./types";

/**
 * PUTs a fleet spec and reports the accepted change. `buildBody` validates the
 * draft and throws a user-facing error before any request is made.
 */
export function useFleetSubmit({
  fleetKey,
  resource,
  page,
  onClose,
  onAccepted,
}: {
  fleetKey: string;
  /** The fleet being edited; absent when creating one. */
  resource?: Resource<unknown>;
  page: boolean;
  onClose: () => void;
  onAccepted: (change: ChangeRef) => void;
}) {
  const client = useQueryClient();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const [attempt] = useState(() => new MutationAttempt());

  async function submit(event: FormEvent, buildBody: () => string) {
    event.preventDefault();
    setError(null);
    try {
      const body = buildBody();
      const path = resourcePath("fleets", fleetKey);
      const headers = attempt.headers("PUT", path, body, resource?.etag || null, !resource);
      setBusy(true);
      const result = await api<Accepted>(path, { method: "PUT", headers, body });
      onAccepted({ id: result.data.changeId, resource: fleetKey, type: "fleet" });
      void client.invalidateQueries({ queryKey: ["fleets"] });
      void client.invalidateQueries({ queryKey: ["fleet", fleetKey] });
      void client.invalidateQueries({ queryKey: ["fleet-status", fleetKey] });
      if (!page) onClose();
    } catch (error) {
      setError(error);
    } finally {
      setBusy(false);
    }
  }

  return { busy, error, submit };
}
