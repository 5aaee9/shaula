import { useEffect, useRef, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { api, ApiError, MutationAttempt, resourcePath, type Resource } from "./api";
import type { Accepted, ChangeRef, TemplateResource } from "./types";

/**
 * Posts a template update against the reviewed resource ETag. A rejected
 * review (409/412/422) sets `conflict`, which blocks resubmitting the stale draft.
 */
export function useTemplateUpdate(
  resource: Resource<TemplateResource>,
  onAccepted: (change: ChangeRef | null) => void,
) {
  const client = useQueryClient();
  const [busy, setBusy] = useState(false);
  const [conflict, setConflict] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const [attempt] = useState(() => new MutationAttempt());
  const mounted = useRef(false);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  async function send(buildBody: () => string) {
    setBusy(true);
    setError(null);
    try {
      const path = `${resourcePath("template-profiles", resource.data.key)}/updates`;
      const body = buildBody();
      const { data } = await api<Accepted>(path, {
        method: "POST",
        body,
        headers: attempt.headers("POST", path, body, resource.etag, false),
      });
      void client.invalidateQueries();
      if (mounted.current)
        onAccepted(
          data.noOp ? null : { id: data.changeId, resource: resource.data.key, type: "profile" },
        );
    } catch (error) {
      if (mounted.current) {
        setConflict(error instanceof ApiError && [409, 412, 422].includes(error.status));
        setError(error);
      }
    } finally {
      setBusy(false);
    }
  }

  return { busy, conflict, error, send };
}
