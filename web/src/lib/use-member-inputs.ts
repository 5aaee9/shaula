import { useEffect, useRef, useState } from "react";
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
  const request = useRef<AbortController | null>(null);
  const sequence = useRef(0);
  const key = templateKey.trim();

  useEffect(() => {
    const controller = new AbortController();
    request.current?.abort();
    request.current = controller;
    const generation = ++sequence.current;
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
    void (async () => {
      try {
        const profile = await api<TemplateResource>(resourcePath("template-profiles", key), {
          signal: controller.signal,
        });
        const revision = profile.data.activeRevision;
        if (!revision) throw new Error("This template has no Active revision.");
        const response = await api<InputContract>(
          `${resourcePath("template-profiles", key)}/revisions/${revision}/input-contract`,
          { signal: controller.signal },
        );
        const next = checkInputContract(response.data, key, revision);
        if (generation !== sequence.current || controller.signal.aborted) return;
        setContract(next);
        setError(null);
      } catch (failure) {
        if (generation !== sequence.current || controller.signal.aborted) return;
        setError(failure);
      } finally {
        if (generation === sequence.current) setLoading(false);
      }
    })();
    return () => controller.abort();
    // The member's template key owns this contract read.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key, canRead]);

  return { contract, loading, error, canRead, ready: !!contract && !loading && !error };
}

export type MemberContract = ReturnType<typeof useMemberContract>;
