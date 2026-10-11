import { useState, type ReactNode } from "react";
import type { Resource } from "@/lib/api";
import { forgejoFleetBody, forgejoPlacementAllowed } from "@/lib/fleet-form-body";
import { useAuthProfiles, useTemplatePools, useTemplates } from "@/lib/queries";
import { canChooseProfile } from "@/lib/profile-choice";
import { authChoices, templateChoices } from "@/lib/runner-backend";
import { useFleetInputs } from "@/lib/use-fleet-inputs";
import { useFleetPlacement } from "@/lib/use-fleet-placement";
import { useFleetSubmit } from "@/lib/use-fleet-submit";
import type { FleetResource, ForgejoFleetSpec } from "@/lib/types";
import type { FleetFormProps } from "./fleet-form";
import { FleetFormFrame } from "./fleet-form-parts";
import {
  ForgejoAuthSection,
  ForgejoCapacitySection,
  ForgejoPlacementSection,
} from "./forgejo-fleet-sections";
import { Input } from "./ui/input";
import { Field } from "./status";

export function ForgejoFleetForm({
  resource: current,
  backendSelector,
  scopes,
  onClose,
  onAccepted,
  page = false,
}: Omit<FleetFormProps, "resource"> & {
  resource?: Resource<FleetResource<ForgejoFleetSpec>>;
  backendSelector?: ReactNode;
}) {
  const [resource] = useState(current);
  const original = resource?.data.spec;
  const editing = !!resource;
  const [key, setKey] = useState(resource?.data.key || "");
  const draft = useFleetPlacement(original);
  const [auth, setAuth] = useState(original?.forgejo.auth_profile_ref || "");
  const [prefix, setPrefix] = useState(original?.forgejo.runner_name_prefix || "");
  const [labels, setLabels] = useState(() => original?.forgejo.labels.join(", ") || "");
  const [capacity, setCapacity] = useState(
    original?.capacity || { min_runners: 0, max_runners: 10 },
  );
  const { busy, error, submit } = useFleetSubmit({
    fleetKey: key,
    resource,
    page,
    onClose,
    onAccepted,
  });
  const canReadAuth = scopes.includes("auth.read");
  const authQuery = useAuthProfiles(canReadAuth);
  const choices = authChoices(authQuery, "forgejo");
  const target =
    original?.forgejo ||
    authQuery.data?.data.profiles.find((p) => p.key === auth)?.active?.forgejo?.target;
  const editor = useFleetInputs(
    original?.template_profile_ref ? resource : undefined,
    scopes.includes("template.read"),
  );
  const pools = useTemplatePools(draft.placement === "shared" && editor.canRead);
  const templates = templateChoices(useTemplates(editor.canRead), "forgejo");
  const authAllowed = editing || (canChooseProfile(choices, auth, canReadAuth) && !!target);
  const placementAllowed = forgejoPlacementAllowed({
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

  return (
    <FleetFormFrame
      fleetKey={key}
      editing={editing}
      page={page}
      busy={busy}
      error={error}
      canSubmit={authAllowed && placementAllowed}
      onClose={onClose}
      onSubmit={(event) =>
        void submit(event, () =>
          forgejoFleetBody({
            fleetKey: key,
            target,
            allowed: authAllowed && placementAllowed,
            auth,
            prefix,
            labels,
            capacity,
            draft,
            editor,
          }),
        )
      }
    >
      {backendSelector}
      <Field label="Fleet key">
        <Input
          required
          disabled={editing}
          pattern="[a-z0-9][a-z0-9-]*"
          maxLength={63}
          value={key}
          onChange={(e) => setKey(e.target.value)}
          placeholder="forgejo-linux"
        />
      </Field>
      <ForgejoAuthSection
        editing={editing}
        fleetKey={key}
        choices={choices}
        canRead={canReadAuth}
        auth={auth}
        setAuth={setAuth}
        target={target}
        prefix={prefix}
        setPrefix={setPrefix}
        labels={labels}
        setLabels={setLabels}
      />
      <ForgejoPlacementSection
        editing={editing}
        originalPoolRef={original?.template_pool_ref}
        draft={draft}
        editor={editor}
        templates={templates}
        pools={pools.data?.data.pools}
        poolsError={pools.error}
      />
      <ForgejoCapacitySection capacity={capacity} setCapacity={setCapacity} />
    </FleetFormFrame>
  );
}
