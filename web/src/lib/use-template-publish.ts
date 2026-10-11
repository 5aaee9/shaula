import { useEffect, useRef, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { api, MutationAttempt, resourcePath, type Resource } from "./api";
import { inputEntries } from "./input-values";
import type { Accepted, ChangeRef, TemplateResource } from "./types";

/** Label of the first manual setting that is not a JSON object, if any. */
export function invalidPublishSetting(bindings: string, policy: string) {
  for (const [value, label] of [
    [bindings, "Bindings"],
    [policy, "Fleet input policy"],
  ]) {
    try {
      inputEntries(value);
    } catch {
      return label;
    }
  }
  return null;
}

/** Raw JSON body so binding and policy values keep their exact number tokens. */
export function templateProfileBody({
  artifactDigest,
  engineRef,
  sourceKey,
  bindings,
  policy,
}: {
  artifactDigest: string;
  engineRef: string;
  /** Only set when publishing from a default template; undefined is omitted. */
  sourceKey?: string;
  bindings: string;
  policy: string;
}) {
  const metadata = JSON.stringify({
    artifact_digest: artifactDigest,
    engine_ref: engineRef,
    source_key: sourceKey,
  });
  return `${metadata.slice(0, -1)},"bindings":${bindings},"fleet_input_policy":${policy}}`;
}

/** PUTs a template profile; creates it when there is no loaded snapshot to match. */
export function useTemplatePublish(
  snapshot: Resource<TemplateResource> | undefined,
  onAccepted: (change: ChangeRef | null) => void,
) {
  const client = useQueryClient();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const [attempt] = useState(() => new MutationAttempt());
  const mounted = useRef(false);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  async function publish(key: string, buildBody: () => Promise<string>) {
    setBusy(true);
    setError(null);
    try {
      const body = await buildBody();
      const path = resourcePath("template-profiles", key);
      const { data } = await api<Accepted>(path, {
        method: "PUT",
        body,
        headers: attempt.headers("PUT", path, body, snapshot?.etag || null, !snapshot),
      });
      void client.invalidateQueries();
      if (mounted.current)
        onAccepted(data.noOp ? null : { id: data.changeId, resource: key, type: "profile" });
    } catch (error) {
      if (mounted.current) setError(error);
    } finally {
      setBusy(false);
    }
  }

  return { busy, error, publish };
}
