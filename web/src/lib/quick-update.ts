import { api, resourcePath } from "./api";
import type { TemplateVariables } from "./template-variables";
import { isSensitiveMarker, type TemplateResource, type TemplateRevision } from "./types";

/** `sha256:` plus the first 12 hex characters; enough to tell revisions apart. */
export function shortDigest(digest: string) {
  return digest.replace(/^(sha256:[a-f0-9]{12})[a-f0-9]+$/, "$1");
}

/** Required target bindings without a default that the base revision does not set. */
export function missingRequiredBindings(
  variables: TemplateVariables | null,
  bindings: TemplateRevision["bindings"],
) {
  if (!variables?.available) return [];
  const base = bindings ?? {};
  return variables.bindings
    .filter((variable) => variable.required && variable.defaultValueJson === undefined)
    .filter((variable) => {
      const value = base[variable.key];
      return isSensitiveMarker(value) ? !value.set : value === undefined || value === null;
    })
    .map((variable) => variable.label || variable.key);
}

/** A fresh base for one confirmation: head ETag, desired revision and target variables. */
export async function loadQuickUpdate(profileKey: string, signal: AbortSignal) {
  const path = resourcePath("template-profiles", profileKey);
  const resource = await api<TemplateResource>(path, { signal });
  const update = resource.data.sourceUpdate;
  const [revision, variables] = await Promise.all([
    api<TemplateRevision>(`${path}/revisions/${resource.data.desiredRevision}`, { signal }),
    update?.state === "Available" && update.targetDigest
      ? api<TemplateVariables>(
          `${resourcePath("template-artifacts", update.targetDigest)}/variables`,
          { signal },
        ).then(
          (result) => (result.data.artifactDigest === update.targetDigest ? result.data : null),
          () => null, // discovery is advisory; the server still validates the update
        )
      : Promise.resolve(null),
  ]);
  return { resource, revision: revision.data, variables };
}
