import { useState } from "react";
import type { Resource } from "./api";
import { tokenPlacementAllowed } from "./fleet-form-body";
import { canChooseProfile } from "./profile-choice";
import { useAuthProfiles, useTemplatePools, useTemplates } from "./queries";
import { authChoices, templateChoices, type RunnerBackend } from "./runner-backend";
import type { AuthRevisionState, ChangeRef, FleetResource, FleetSpec } from "./types";
import { useFleetInputs } from "./use-fleet-inputs";
import { useFleetPlacement } from "./use-fleet-placement";
import { useFleetSubmit } from "./use-fleet-submit";

/**
 * Draft state shared by the token backends (Forgejo and GitLab). The target
 * comes from the frozen section when editing, otherwise from the chosen
 * profile's Active revision.
 */
export function useTokenFleet<T>({
  backend,
  resource,
  section,
  activeTarget,
  scopes,
  page,
  onClose,
  onAccepted,
}: {
  backend: Exclude<RunnerBackend, "github">;
  resource?: Resource<FleetResource>;
  section?: T & { auth_profile_ref: string; runner_name_prefix: string };
  activeTarget: (state?: AuthRevisionState) => T | undefined;
  scopes: string[];
  page: boolean;
  onClose: () => void;
  onAccepted: (change: ChangeRef) => void;
}) {
  const original: FleetSpec | undefined = resource?.data.spec;
  const editing = !!resource;
  const [key, setKey] = useState(resource?.data.key || "");
  const draft = useFleetPlacement(original);
  const [auth, setAuth] = useState(section?.auth_profile_ref || "");
  const [prefix, setPrefix] = useState(section?.runner_name_prefix || "");
  const [capacity, setCapacity] = useState(
    original?.capacity || { min_runners: 0, max_runners: 10 },
  );
  const submission = useFleetSubmit({ fleetKey: key, resource, page, onClose, onAccepted });
  const canReadAuth = scopes.includes("auth.read");
  const authQuery = useAuthProfiles(canReadAuth);
  const choices = authChoices(authQuery, backend);
  const target: T | undefined =
    section || activeTarget(authQuery.data?.data.profiles.find((p) => p.key === auth)?.active);
  const editor = useFleetInputs(
    original?.template_profile_ref ? resource : undefined,
    scopes.includes("template.read"),
  );
  const pools = useTemplatePools(draft.placement === "shared" && editor.canRead);
  const templates = templateChoices(useTemplates(editor.canRead), backend);
  const authAllowed = editing || (canChooseProfile(choices, auth, canReadAuth) && !!target);
  const placementAllowed = tokenPlacementAllowed({
    placement: draft.placement,
    editor,
    templateAllowed:
      (editing && editor.reference === original?.template_profile_ref) ||
      canChooseProfile(templates, editor.template, editor.canRead),
    editing,
    original,
    pool: draft.pool,
    poolRef: draft.poolRef,
    pools,
    templates,
  });
  return {
    original,
    editing,
    key,
    setKey,
    draft,
    auth,
    setAuth,
    prefix,
    setPrefix,
    capacity,
    setCapacity,
    ...submission,
    canReadAuth,
    choices,
    target,
    editor,
    pools,
    templates,
    allowed: authAllowed && placementAllowed,
  };
}
