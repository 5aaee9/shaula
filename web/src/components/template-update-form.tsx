import { useMemo, useState, type FormEvent } from "react";
import type { Resource } from "@/lib/api";
import {
  initialUpdateTarget,
  templateUpdateBody,
  updateBindingsChanged,
  updateIsCurrent,
} from "@/lib/template-update-body";
import { useTemplateVariables, type TemplateSource } from "@/lib/template-variables";
import type { ChangeRef, TemplateResource, TemplateRevision } from "@/lib/types";
import { initialUpdateBindings } from "@/lib/update-bindings";
import { useTemplateUpdate } from "@/lib/use-template-update";
import { TemplateUpdateBindings } from "./template-update-bindings";
import { TemplateUpdatePolicy } from "./template-update-policy";
import {
  RevisionComparison,
  TargetDiscoveryStatus,
  UpdateActionBar,
  UpdateSourceSection,
} from "./template-update-sections";

export function TemplateUpdateForm({
  resource,
  revision,
  sources,
  onClose,
  onAccepted,
}: {
  resource: Resource<TemplateResource>;
  revision: TemplateRevision;
  sources: TemplateSource[];
  onClose: () => void;
  onAccepted: (change: ChangeRef | null) => void;
}) {
  const [snapshot] = useState(() => ({
    resource,
    revision,
    candidates: sources.filter(
      (source) => source.platform === (revision.platform || resource.data.platform),
    ),
  }));
  const { candidates } = snapshot;
  const [target, setTarget] = useState(() => initialUpdateTarget(candidates, snapshot.revision));
  const [policy, setPolicy] = useState<string>();
  const [bindings, setBindings] = useState<string>(() =>
    initialUpdateBindings(snapshot.revision.bindings),
  );
  const update = useTemplateUpdate(snapshot.resource, onAccepted);
  const discovery = useTemplateVariables(target?.artifactDigest || "", null, "default", true);
  const variables =
    discovery.variables?.artifactDigest === target?.artifactDigest ? discovery.variables : null;
  const bindingsChanged = useMemo(
    () => updateBindingsChanged(bindings, snapshot.revision.bindings),
    [bindings, snapshot.revision.bindings],
  );
  const upToDate = updateIsCurrent(target, snapshot.revision, policy, bindingsChanged);

  async function submit(event: FormEvent) {
    event.preventDefault();
    if (!target || upToDate || update.busy || update.conflict) return;
    await update.send(() =>
      templateUpdateBody({
        target,
        revision: snapshot.revision,
        bindings,
        bindingsChanged,
        policy,
      }),
    );
  }

  return (
    <form aria-label="Template update review" className="min-w-0 space-y-6" onSubmit={submit}>
      <fieldset disabled={update.busy} className="min-w-0 space-y-6">
        <UpdateSourceSection
          sourceKey={snapshot.revision.sourceKey}
          target={target}
          candidates={candidates}
          platform={snapshot.revision.platform || snapshot.resource.data.platform}
          onSelect={(key) => {
            setTarget(candidates.find((source) => source.key === key));
            setPolicy(undefined);
          }}
        />

        <RevisionComparison
          resource={snapshot.resource.data}
          revision={snapshot.revision}
          target={target}
          upToDate={upToDate}
        />

        <TemplateUpdateBindings variables={variables} bindings={bindings} onChange={setBindings} />

        {target && (
          <>
            <TargetDiscoveryStatus
              loading={discovery.loading}
              error={discovery.error}
              variables={variables}
              retry={() => void discovery.inspect()}
            />
            <TemplateUpdatePolicy variables={variables} policy={policy} onChange={setPolicy} />
          </>
        )}
      </fieldset>

      <UpdateActionBar
        busy={update.busy}
        conflict={update.conflict}
        error={update.error}
        submitDisabled={update.busy || !target || upToDate || update.conflict || discovery.loading}
        onClose={onClose}
      />
    </form>
  );
}
