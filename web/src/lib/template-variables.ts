import { useCallback, useEffect, useRef, useState } from "react";
import { api, resourcePath } from "./api";

export interface TemplateSource {
  key: string;
  artifactDigest: string;
  platform: string;
  engineRef: string;
}

export interface TemplateVariable {
  key: string;
  label: string;
  description?: string;
  typeName: string;
  required: boolean;
  sensitive: boolean;
  defaultValueJson?: string;
  options: { valueJson: string }[];
}

export interface TemplateVariables {
  artifactDigest: string;
  available: boolean;
  reason?: string;
  bindings: TemplateVariable[];
  parameters: TemplateVariable[];
}

/** One artifact identity owns each inspection; neither reads nor uploads edit form values. */
export function useTemplateVariables(
  digest: string,
  file: File | null,
  source: string,
  canRead: boolean,
) {
  const [variables, setVariables] = useState<TemplateVariables | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const request = useRef<{ sequence: number; controller?: AbortController }>({ sequence: 0 });
  const uploaded = useRef(new WeakMap<File, string>());

  const artifact = useCallback(
    async (signal?: AbortSignal): Promise<string> => {
      if (source !== "archive") {
        if (!/^sha256:[a-f0-9]{64}$/.test(digest))
          throw new Error("Enter a valid artifact digest.");
        return digest;
      }
      if (!file) throw new Error("Choose a template archive.");
      if (file.size > 64 * 1024 * 1024) throw new Error("Artifact exceeds 64 MiB.");
      const cached = uploaded.current.get(file);
      if (cached) return cached;
      const bytes = await file.arrayBuffer();
      signal?.throwIfAborted();
      const hash = await crypto.subtle.digest("SHA-256", bytes);
      signal?.throwIfAborted();
      const artifactDigest = `sha256:${Array.from(new Uint8Array(hash), (value) => value.toString(16).padStart(2, "0")).join("")}`;
      await api(resourcePath("template-artifacts", artifactDigest), {
        method: "PUT",
        body: bytes,
        signal,
        headers: { "content-type": "application/gzip" },
      });
      uploaded.current.set(file, artifactDigest);
      return artifactDigest;
    },
    [digest, file, source],
  );

  const inspect = useCallback(async () => {
    if (!canRead) return;
    request.current.controller?.abort();
    const sequence = ++request.current.sequence;
    const controller = new AbortController();
    request.current.controller = controller;
    setLoading(true);
    setVariables(null);
    setError(null);
    try {
      const artifactDigest = await artifact(controller.signal);
      const { data } = await api<TemplateVariables>(
        `${resourcePath("template-artifacts", artifactDigest)}/variables`,
        { signal: controller.signal },
      );
      if (controller.signal.aborted || sequence !== request.current.sequence) return;
      if (data.artifactDigest !== artifactDigest)
        throw new Error("Variables belong to a different artifact. Inspect this template again.");
      setVariables(data);
    } catch (error) {
      if (!controller.signal.aborted && sequence === request.current.sequence) setError(error);
    } finally {
      if (sequence === request.current.sequence) setLoading(false);
    }
  }, [artifact, canRead]);

  useEffect(() => {
    request.current.controller?.abort();
    ++request.current.sequence;
    setVariables(null);
    setLoading(false);
    setError(null);
    if (source === "default" && digest && canRead) void inspect();
    return () => {
      request.current.controller?.abort();
      ++request.current.sequence;
    };
  }, [digest, source, canRead, inspect]);

  return { variables, loading, error, inspect, artifact };
}
