import { useQuery } from "@tanstack/react-query";
import { api, resourcePath } from "./api";
import { checkInputContract, type InputContract } from "./input-contract";
import type { TemplateResource } from "./types";

const NO_TEMPLATE_READ = new Error("Template read permission is required to edit template inputs.");

/**
 * Loads the Active revision's input contract for one weighted pool member's
 * `template_profile_ref`.
 *
 * The member's `template_inputs` stay owned by the parent form (they are part
 * of the submitted `template_pool.members[i]`); this hook only resolves which
 * contract applies so the same approved-value inputs used by single-template
 * fleets can render for each member.
 */
export function useMemberContract(templateKey: string, canRead: boolean) {
  const key = templateKey.trim();
  const query = useQuery({
    queryKey: ["member-input-contract", key],
    enabled: !!key && canRead,
    retry: false,
    queryFn: ({ signal }) => loadMemberContract(key, signal),
  });
  return {
    contract: key && canRead ? query.data : undefined,
    loading: !!key && canRead && query.isPending,
    error: !key ? null : !canRead ? NO_TEMPLATE_READ : query.error,
  };
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
