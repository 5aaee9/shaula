import { useState } from "react";
import type { Resource } from "@/lib/api";
import { useAuthProfiles, useTemplatePools, useTemplates } from "@/lib/queries";
import { canChooseProfile } from "@/lib/profile-choice";
import { authChoices, templateChoices } from "@/lib/runner-backend";
import { githubFleetBody, githubFleetProblem } from "@/lib/fleet-form-body";
import type { ChangeRef, FleetResource, GitHubFleetSpec } from "@/lib/types";
import { useFleetInputs } from "@/lib/use-fleet-inputs";
import { useFleetPlacement } from "@/lib/use-fleet-placement";
import { useFleetSubmit } from "@/lib/use-fleet-submit";
import { FleetFormFrame, FleetStage } from "./fleet-form-parts";
import {
  GitHubAuthStage,
  GitHubCapacityStage,
  GitHubPlacementStage,
  GitHubTargetStage,
} from "./github-fleet-stages";
import { Input } from "./ui/input";
import { Field } from "./status";

const initial: GitHubFleetSpec = {
  github: {
    target: { kind: "organization", owner: "" },
    auth_profile_ref: "",
    scale_set_name: "",
    runner_group: "Default",
    labels: [],
  },
  capacity: { min_runners: 0, max_runners: 10 },
  template_profile_ref: "",
  template_inputs: {},
};

export function GitHubFleetForm({
  backendSelector,
  resource: currentResource,
  scopes,
  onClose,
  onAccepted,
  page = false,
}: {
  resource?: Resource<FleetResource<GitHubFleetSpec>>;
  backendSelector?: React.ReactNode;
  scopes: string[];
  onClose: () => void;
  onAccepted: (change: ChangeRef) => void;
  page?: boolean;
}) {
  const [resource] = useState(currentResource);
  const original = resource?.data.spec;
  const editing = !!resource;
  const [key, setKey] = useState(resource?.data.key || "");
  const [spec, setSpec] = useState<GitHubFleetSpec>(original || initial);
  const draft = useFleetPlacement(original);
  const editor = useFleetInputs(resource, scopes.includes("template.read"));
  const [labels, setLabels] = useState(() => spec.github.labels.join(", "));
  const [advancedOpen, setAdvancedOpen] = useState(false);
  const { busy, error, submit } = useFleetSubmit({
    fleetKey: key,
    resource,
    page,
    onClose,
    onAccepted,
  });
  const canReadAuth = scopes.includes("auth.read");
  const templates = templateChoices(useTemplates(editor.canRead), "github");
  const templatePools = useTemplatePools(
    draft.placement === "shared" && scopes.includes("template.read"),
  );
  const authProfiles = authChoices(useAuthProfiles(canReadAuth), "github");
  const originalAuth = original?.github.auth_profile_ref;
  const authAllowed =
    (editing && spec.github.auth_profile_ref === originalAuth) ||
    canChooseProfile(authProfiles, spec.github.auth_profile_ref, canReadAuth);
  const templateAllowed =
    (editing && editor.reference === original?.template_profile_ref) ||
    canChooseProfile(templates, editor.template, editor.canRead);
  const problem = githubFleetProblem({
    placement: draft.placement,
    editor,
    authAllowed,
    templateAllowed,
    poolRef: draft.poolRef,
    capacity: spec.capacity,
    members: draft.pool.members,
  });
  const canSubmit = problem?.kind !== "selection";
  function github(patch: Partial<GitHubFleetSpec["github"]>) {
    setSpec({ ...spec, github: { ...spec.github, ...patch } });
  }
  return (
    <FleetFormFrame
      fleetKey={key}
      editing={editing}
      page={page}
      busy={busy}
      error={error}
      canSubmit={canSubmit}
      onClose={onClose}
      onSubmit={(event) =>
        void submit(event, () => {
          if (problem) {
            if (problem.kind === "capacity") setAdvancedOpen(true);
            throw new Error(problem.message);
          }
          return githubFleetBody({ spec, fleetKey: key, editing, labels, draft, editor });
        })
      }
    >
      {backendSelector}
      <FleetStage
        title="Fleet"
        description={
          editing
            ? "The fleet key identifies this scale set and cannot be changed."
            : "A stable key for this runner fleet."
        }
      >
        <Field label="Fleet key">
          <Input
            required
            pattern="[a-z0-9][a-z0-9-]*"
            maxLength={63}
            value={key}
            disabled={editing}
            onChange={(event) => setKey(event.target.value)}
            placeholder="linux-build"
          />
        </Field>
      </FleetStage>
      <GitHubTargetStage
        editing={editing}
        target={spec.github.target}
        onChange={(target) => github({ target })}
      />
      <GitHubAuthStage
        value={spec.github.auth_profile_ref}
        original={originalAuth}
        query={authProfiles}
        canRead={canReadAuth}
        onChange={(value) => github({ auth_profile_ref: value })}
      />
      <GitHubPlacementStage
        editing={editing}
        originalPoolRef={original?.template_pool_ref}
        draft={draft}
        editor={editor}
        templates={templates}
        pools={templatePools.data?.data.pools || []}
      />
      <GitHubCapacityStage
        editing={editing}
        fleetKey={key}
        spec={spec}
        setSpec={setSpec}
        github={github}
        labels={labels}
        setLabels={setLabels}
        advancedOpen={advancedOpen}
        setAdvancedOpen={setAdvancedOpen}
      />
    </FleetFormFrame>
  );
}
