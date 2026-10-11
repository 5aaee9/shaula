import { useEffect, useState } from "react";
import { api, resourcePath } from "./api";
import { checkInputContract, type InputContract } from "./input-contract";
import type { TemplateResource } from "./types";

/**
 * Loads the Active revision's input contract for one weighted pool member's
 * `template_profile_ref`.
 *
 * The member's `template_inputs` stay owned by the parent form (they are part
 * of the submitted `template_pool.members[i]`); this hook only resolves which
 * contract applies so the same approved-value inputs used by single-template
 * fleets can render for each member. Returns the contract plus load state.
 */
export function useMemberContract(templateKey: string, canRead: boolean) {
  const [contract, setContract] = useState<InputContract>();
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const key = templateKey.trim();

  useEffect(() => {
    // The cleanup aborts this read before the next key's read starts, so an
    // aborted signal is the only staleness check needed when it settles.
    const controller = new AbortController();
    setContract(undefined);
    setError(null);
    if (!key) {
      setLoading(false);
      return;
    }
    if (!canRead) {
      setLoading(false);
      setError(new Error("Template read permission is required to edit template inputs."));
      return;
    }
    setLoading(true);
    void loadMemberContract(key, controller.signal)
      .then((next) => {
        if (!controller.signal.aborted) setContract(next);
      })
      .catch((failure: unknown) => {
        if (!controller.signal.aborted) setError(failure);
      })
      .finally(() => {
        if (!controller.signal.aborted) setLoading(false);
      });
    return () => controller.abort();
    // The member's template key owns this contract read.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key, canRead]);

  return { contract, loading, error, canRead, ready: !!contract && !loading && !error };
}

/** The Active revision's input contract for one template key. */
async function loadMemberContract(key: string, signal: AbortSignal): Promise<InputContract> {
  const profile = await api<TemplateResource>(resourcePath("template-profiles", key), { signal });
  const revision = profile.data.activeRevision;
  if (!revision) throw new Error("This template has no Active revision.");
  const response = await api<InputContract>(
    `${resourcePath("template-profiles", key)}/revisions/${revision}/input-contract`,
    { signal },
  );
  return checkInputContract(response.data, key, revision);
}

export type MemberContract = ReturnType<typeof useMemberContract>;
