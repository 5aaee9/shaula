import { inputEntries } from "./input-values";
import type { TemplateSource } from "./template-variables";
import { isSensitiveMarker, type TemplateRevision } from "./types";

type RevisionBindings = TemplateRevision["bindings"];

/** The saved default, else the single default matching the current artifact, else the only one. */
export function initialUpdateTarget(
  candidates: TemplateSource[],
  revision: TemplateRevision,
): TemplateSource | undefined {
  if (revision.sourceKey) return candidates.find((source) => source.key === revision.sourceKey);
  const matching = candidates.filter(
    (source) =>
      source.artifactDigest === revision.artifactDigest && source.engineRef === revision.engineRef,
  );
  if (matching.length === 1) return matching[0];
  return candidates.length === 1 ? candidates[0] : undefined;
}

export function updateBindingsChanged(bindings: string, revisionBindings: RevisionBindings) {
  // Compare the desired merged set against the base: a non-secret value that
  // differs, or a sensitive field with a new (non-null) value, is a change.
  let current: Map<string, string>;
  try {
    current = inputEntries(bindings);
  } catch {
    return true; // invalid JSON: allow submit so the server reports it
  }
  const base = revisionBindings ?? {};
  const keys = new Set([...Object.keys(base), ...current.keys()]);
  for (const key of keys) {
    const baseValue = base[key];
    if (isSensitiveMarker(baseValue)) {
      if (current.has(key)) return true; // a supplied secret replaces
      continue;
    }
    const next = current.get(key);
    const baseJson = JSON.stringify(baseValue === undefined ? null : baseValue);
    if ((next ?? "null") !== baseJson) return true;
  }
  return false;
}

export function updateIsCurrent(
  target: TemplateSource | undefined,
  revision: TemplateRevision,
  policy: string | undefined,
  bindingsChanged: boolean,
) {
  return (
    !!target &&
    target.artifactDigest === revision.artifactDigest &&
    target.engineRef === revision.engineRef &&
    target.key === revision.sourceKey &&
    policy === undefined &&
    !bindingsChanged
  );
}

function mergedBindings(bindings: string, revisionBindings: RevisionBindings) {
  const current = inputEntries(bindings);
  const base = revisionBindings ?? {};
  const out = new Map<string, string>();
  const keys = new Set([...Object.keys(base), ...current.keys()]);
  for (const key of keys) {
    // Sensitive field: new value replaces, otherwise `null` = keep.
    if (isSensitiveMarker(base[key])) {
      out.set(key, current.has(key) ? current.get(key)! : "null");
    } else if (current.has(key)) {
      out.set(key, current.get(key)!);
    } else {
      out.set(key, "null");
    }
  }
  return `{${[...out].map(([k, v]) => `${JSON.stringify(k)}:${v}`).join(",")}}`;
}

/** Raw JSON body so binding and policy values keep their exact number tokens. */
export function templateUpdateBody({
  target,
  revision,
  bindings,
  bindingsChanged,
  policy,
}: {
  target: TemplateSource;
  revision: TemplateRevision;
  bindings: string;
  bindingsChanged: boolean;
  policy: string | undefined;
}) {
  let body = JSON.stringify({
    artifact_digest: target.artifactDigest,
    engine_ref: target.engineRef,
    source_key: target.key,
  });
  // Omit `bindings` entirely when nothing changed so the request replays
  // the plain reuse-the-base identity (spec 0038: omission = reuse).
  if (bindingsChanged)
    body = `${body.slice(0, -1)},"bindings":${mergedBindings(bindings, revision.bindings)}}`;
  if (policy !== undefined) body = `${body.slice(0, -1)},"fleet_input_policy":${policy}}`;
  return body;
}
