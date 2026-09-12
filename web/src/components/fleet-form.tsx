import { NativeSelect, NativeSelectOption } from "@/components/ui/native-select";
import { useRef, useState, type FormEvent } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { LoaderCircle, Save, Plus, Trash2 } from "lucide-react";
import { api, MutationAttempt, resourcePath, type Resource } from "@/lib/api";
import { useAuthProfiles, useTemplates } from "@/lib/queries";
import { canChooseProfile } from "@/lib/profile-choice";
import type {
  Accepted,
  ChangeRef,
  FleetResource,
  FleetSpec,
  TemplatePoolMemberSpec,
} from "@/lib/types";
import { AdvancedSettings } from "./advanced-settings";
import { Modal } from "./modal";
import { Button } from "./ui/button";
import { Input } from "./ui/input";
import { useFleetInputs } from "@/lib/use-fleet-inputs";
import { FleetTemplateInputs } from "./fleet-template-inputs";
import { FleetTemplateSelection } from "./fleet-template-selection";
import { ProfileSelector } from "./profile-selector";
import { ErrorNotice, Field } from "./status";
const initial: FleetSpec = {
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
const initialMember = (index: number): TemplatePoolMemberSpec => ({
  key: `member-${index + 1}`,
  template_profile_ref: "",
  weight: 1,
  template_inputs: {},
});
export function FleetForm({
  resource: currentResource,
  scopes,
  onClose,
  onAccepted,
}: {
  resource?: Resource<FleetResource>;
  scopes: string[];
  onClose: () => void;
  onAccepted: (change: ChangeRef) => void;
}) {
  const [resource] = useState(currentResource);
  const client = useQueryClient();
  const [key, setKey] = useState(resource?.data.key || "");
  const [spec, setSpec] = useState<FleetSpec>(resource?.data.spec || initial);
  const initialPool = resource?.data.spec.template_pool;
  const [poolMode, setPoolMode] = useState(!!initialPool);
  const [poolMembers, setPoolMembers] = useState<TemplatePoolMemberSpec[]>(
    initialPool?.members || [initialMember(0)],
  );
  const [failurePolicy, setFailurePolicy] = useState<"backpressure" | "redistribute">(
    initialPool?.failure_policy || "backpressure",
  );
  const editor = useFleetInputs(resource, scopes.includes("template.read"));
  const [labels, setLabels] = useState(spec.github.labels.join(", "));
  const [error, setError] = useState<unknown>(null);
  const [busy, setBusy] = useState(false);
  const [advancedOpen, setAdvancedOpen] = useState(false);
  const attempt = useRef(new MutationAttempt());
  const canReadAuth = scopes.includes("auth.read");
  const templates = useTemplates(editor.canRead);
  const authProfiles = useAuthProfiles(canReadAuth);
  const originalAuth = resource?.data.spec.github.auth_profile_ref;
  const authAllowed =
    (!!resource && spec.github.auth_profile_ref === originalAuth) ||
    canChooseProfile(authProfiles, spec.github.auth_profile_ref, canReadAuth);
  const templateAllowed =
    (!!resource && editor.reference === resource.data.spec.template_profile_ref) ||
    canChooseProfile(templates, editor.template, editor.canRead);
  const customSettings = [
    !!spec.github.scale_set_name && spec.github.scale_set_name !== key,
    spec.github.runner_group !== "Default",
    spec.capacity.min_runners !== 0,
    !!labels.trim(),
  ].filter(Boolean).length;
  function github(patch: Partial<FleetSpec["github"]>) {
    setSpec({ ...spec, github: { ...spec.github, ...patch } });
  }
  async function submit(event: FormEvent) {
    event.preventDefault();
    setError(null);
    try {
      if (!poolMode && !editor.canSubmit)
        throw new Error("Load and review the template input contract before submitting.");
      if (!authAllowed || (!poolMode && !templateAllowed))
        throw new Error(
          "Choose available profiles from a successfully loaded list before submitting.",
        );
      if (spec.capacity.min_runners > spec.capacity.max_runners) {
        setAdvancedOpen(true);
        throw new Error("Minimum runners cannot exceed maximum runners.");
      }
      if (poolMode) {
        if (poolMembers.length < 1 || poolMembers.length > 32)
          throw new Error("Template pool must contain between 1 and 32 members.");
        const keys = new Set<string>();
        for (const member of poolMembers) {
          if (!/^[a-z0-9][a-z0-9-]*$/.test(member.key))
            throw new Error(`Invalid member key: ${member.key}`);
          if (keys.has(member.key)) throw new Error(`Duplicate member key: ${member.key}`);
          keys.add(member.key);
          if (!Number.isInteger(member.weight) || member.weight < 1 || member.weight > 10000)
            throw new Error(`Weight for ${member.key} must be an integer from 1 to 10000.`);
        }
      }
      const bodyWithoutInputs = JSON.stringify({
        ...spec,
        github: {
          ...spec.github,
          scale_set_name: resource ? spec.github.scale_set_name : spec.github.scale_set_name || key,
          labels: labels
            .split(",")
            .map((value) => value.trim())
            .filter(Boolean),
        },
        template_inputs: poolMode ? undefined : undefined,
        template_profile_ref: poolMode ? undefined : editor.reference,
        template_pool: poolMode
          ? { members: poolMembers, failure_policy: failurePolicy }
          : undefined,
      });
      const body = poolMode
        ? bodyWithoutInputs
        : `${bodyWithoutInputs.slice(0, -1)},"template_inputs":${editor.json}}`;
      const path = resourcePath("fleets", key);
      const headers = attempt.current.headers("PUT", path, body, resource?.etag || null, !resource);
      setBusy(true);
      const result = await api<Accepted>(path, { method: "PUT", headers, body });
      onAccepted({ id: result.data.changeId, resource: key, type: "fleet" });
      void client.invalidateQueries({ queryKey: ["fleets"] });
      void client.invalidateQueries({ queryKey: ["fleet", key] });
      void client.invalidateQueries({ queryKey: ["fleet-status", key] });
      onClose();
    } catch (error) {
      setError(error);
    } finally {
      setBusy(false);
    }
  }
  return (
    <Modal
      open
      title={resource ? `Edit ${key}` : "Create fleet"}
      onOpenChange={(open) => {
        if (!open && !busy) onClose();
      }}
    >
      <form onSubmit={submit} className="form-stack">
        <fieldset disabled={busy} className="form-stack">
          <Field label="Fleet key">
            <Input
              required
              pattern="[a-z0-9][a-z0-9-]*"
              maxLength={63}
              value={key}
              disabled={!!resource}
              onChange={(event) => setKey(event.target.value)}
              placeholder="linux-build"
            />
          </Field>
          <div className="form-grid">
            <Field label="GitHub target">
              <NativeSelect
                disabled={!!resource}
                value={spec.github.target.kind}
                onChange={(event) =>
                  github({
                    target:
                      event.target.value === "repository"
                        ? { kind: "repository", owner: spec.github.target.owner, repository: "" }
                        : { kind: "organization", owner: spec.github.target.owner },
                  })
                }
              >
                <NativeSelectOption value="organization">Organization</NativeSelectOption>
                <NativeSelectOption value="repository">Repository</NativeSelectOption>
              </NativeSelect>
            </Field>
            <Field label="Owner">
              <Input
                required
                disabled={!!resource}
                value={spec.github.target.owner}
                onChange={(event) =>
                  github({ target: { ...spec.github.target, owner: event.target.value } })
                }
                placeholder="acme"
              />
            </Field>
          </div>
          {spec.github.target.kind === "repository" && (
            <Field label="Repository">
              <Input
                required
                disabled={!!resource}
                value={spec.github.target.repository}
                onChange={(event) =>
                  github({
                    target: {
                      kind: "repository",
                      owner: spec.github.target.owner,
                      repository: event.target.value,
                    },
                  })
                }
              />
            </Field>
          )}
          <ProfileSelector
            label="GitHub authentication profile"
            value={spec.github.auth_profile_ref}
            query={authProfiles}
            canRead={canReadAuth}
            permission="auth.read"
            onChange={(value) => github({ auth_profile_ref: value })}
          />
          {originalAuth && spec.github.auth_profile_ref !== originalAuth && (
            <Button
              type="button"
              variant="ghost"
              onClick={() => github({ auth_profile_ref: originalAuth })}
            >
              Restore original authentication
            </Button>
          )}
          {!poolMode ? (
            <>
              <FleetTemplateSelection editor={editor} templates={templates} />
              <FleetTemplateInputs editor={editor} />
            </>
          ) : (
            <TemplatePoolEditor
              members={poolMembers}
              setMembers={setPoolMembers}
              failurePolicy={failurePolicy}
              setFailurePolicy={setFailurePolicy}
              templates={templates}
            />
          )}
          {!resource && (
            <Field label="Template placement">
              <NativeSelect
                value={poolMode ? "pool" : "single"}
                onChange={(event) => setPoolMode(event.target.value === "pool")}
              >
                <NativeSelectOption value="single">Single template</NativeSelectOption>
                <NativeSelectOption value="pool">Weighted template pool</NativeSelectOption>
              </NativeSelect>
              {poolMode && (
                <p className="text-sm text-muted-foreground">
                  Each new Runner draws a member independently by weight. Weights influence the
                  long-run mix; finite batches can vary and GitHub job routing is not controlled.
                </p>
              )}
            </Field>
          )}
          <Field label="Maximum runners">
            <Input
              required
              type="number"
              min={0}
              step={1}
              value={spec.capacity.max_runners}
              onChange={(event) =>
                setSpec({
                  ...spec,
                  capacity: { ...spec.capacity, max_runners: Number(event.target.value) },
                })
              }
            />
          </Field>
          <AdvancedSettings
            open={advancedOpen}
            onOpenChange={setAdvancedOpen}
            summary={
              customSettings
                ? `${customSettings} custom settings`
                : "Default runner group, 0 minimum runners"
            }
          >
            <div className="form-grid">
              <Field label="Scale set name">
                <Input
                  disabled={!!resource}
                  maxLength={100}
                  value={spec.github.scale_set_name}
                  placeholder={key || "Fleet key"}
                  onChange={(event) => github({ scale_set_name: event.target.value })}
                />
                {!resource && (
                  <p className="text-muted-foreground text-sm">
                    Uses the fleet key when left empty.
                  </p>
                )}
              </Field>
              <Field label="Runner group">
                <Input
                  required
                  disabled={!!resource}
                  maxLength={100}
                  value={spec.github.runner_group}
                  onChange={(event) => github({ runner_group: event.target.value })}
                />
              </Field>
            </div>
            <div className="form-grid">
              <Field label="Minimum runners">
                <Input
                  required
                  type="number"
                  min={0}
                  step={1}
                  value={spec.capacity.min_runners}
                  onChange={(event) =>
                    setSpec({
                      ...spec,
                      capacity: { ...spec.capacity, min_runners: Number(event.target.value) },
                    })
                  }
                />
              </Field>
            </div>
            <Field label="Labels">
              <Input
                value={labels}
                onChange={(event) => setLabels(event.target.value)}
                placeholder="linux, x64"
              />
              <p className="text-muted-foreground text-sm">
                Separate labels with commas. Leave empty to use the scale set name as the label.
                {resource &&
                  " Changes sync to GitHub after saving. Running jobs continue while labels update."}
              </p>
            </Field>
          </AdvancedSettings>
        </fieldset>
        {error !== null && <ErrorNotice error={error} />}
        <div className="dialog-actions">
          <Button type="button" variant="outline" onClick={onClose} disabled={busy}>
            Cancel
          </Button>
          <Button
            disabled={
              busy || (!poolMode && (!editor.canSubmit || !templateAllowed)) || !authAllowed
            }
            type="submit"
          >
            {busy ? <LoaderCircle className="animate-spin" /> : <Save />}
            {resource ? "Save changes" : "Create fleet"}
          </Button>
        </div>
      </form>
    </Modal>
  );
}

function TemplatePoolEditor({
  members,
  setMembers,
  failurePolicy,
  setFailurePolicy,
  templates,
}: {
  members: TemplatePoolMemberSpec[];
  setMembers: (members: TemplatePoolMemberSpec[]) => void;
  failurePolicy: "backpressure" | "redistribute";
  setFailurePolicy: (value: "backpressure" | "redistribute") => void;
  templates: ReturnType<typeof useTemplates>;
}) {
  const available = templates.data?.data.profiles || [];
  return (
    <section className="form-stack rounded-md border p-3" aria-label="Weighted template pool">
      <div className="flex items-center justify-between">
        <div>
          <h3 className="font-medium">Weighted template members</h3>
          <p className="text-sm text-muted-foreground">1–32 members; integer weights 1–10000.</p>
        </div>
        <Button
          type="button"
          variant="outline"
          disabled={members.length >= 32}
          onClick={() => setMembers([...members, initialMember(members.length)])}
        >
          <Plus /> Add member
        </Button>
      </div>
      {members.map((member, index) => (
        <div key={member.key} className="form-stack rounded-md border p-3">
          <div className="form-grid">
            <Field label="Member key">
              <Input
                value={member.key}
                onChange={(e) =>
                  setMembers(
                    members.map((m, i) => (i === index ? { ...m, key: e.target.value } : m)),
                  )
                }
                pattern="[a-z0-9][a-z0-9-]*"
                required
              />
            </Field>
            <Field label="Weight">
              <Input
                type="number"
                min={1}
                max={10000}
                step={1}
                value={member.weight}
                onChange={(e) =>
                  setMembers(
                    members.map((m, i) =>
                      i === index ? { ...m, weight: Number(e.target.value) } : m,
                    ),
                  )
                }
                required
              />
            </Field>
          </div>
          <Field label="Template profile">
            <NativeSelect
              value={member.template_profile_ref}
              required
              onChange={(e) =>
                setMembers(
                  members.map((m, i) =>
                    i === index ? { ...m, template_profile_ref: e.target.value } : m,
                  ),
                )
              }
            >
              <NativeSelectOption value="">Choose a template</NativeSelectOption>
              {available.map((profile) => (
                <NativeSelectOption
                  key={profile.key}
                  value={profile.key}
                  disabled={!profile.activeRevision}
                >
                  {profile.key} · {profile.status}
                </NativeSelectOption>
              ))}
            </NativeSelect>
          </Field>
          <Field label="Template inputs (JSON)">
            <textarea
              className="min-h-20 w-full rounded-md border bg-background p-2 font-mono text-sm"
              value={JSON.stringify(member.template_inputs, null, 2)}
              onChange={(e) => {
                try {
                  const parsed = JSON.parse(e.target.value);
                  if (parsed && typeof parsed === "object" && !Array.isArray(parsed))
                    setMembers(
                      members.map((m, i) => (i === index ? { ...m, template_inputs: parsed } : m)),
                    );
                } catch {
                  // Keep the last valid object until JSON is complete.
                }
              }}
            />
          </Field>
          <Field label="Maximum runners (optional)">
            <Input
              type="number"
              min={0}
              step={1}
              value={member.max_runners ?? ""}
              onChange={(e) =>
                setMembers(
                  members.map((m, i) =>
                    i === index
                      ? {
                          ...m,
                          max_runners: e.target.value === "" ? undefined : Number(e.target.value),
                        }
                      : m,
                  ),
                )
              }
            />
          </Field>
          <Button
            type="button"
            variant="ghost"
            disabled={members.length <= 1}
            onClick={() => setMembers(members.filter((_, i) => i !== index))}
          >
            <Trash2 /> Remove member
          </Button>
        </div>
      ))}
      <Field label="Failure policy">
        <NativeSelect
          value={failurePolicy}
          onChange={(e) => setFailurePolicy(e.target.value as typeof failurePolicy)}
        >
          <NativeSelectOption value="backpressure">Backpressure</NativeSelectOption>
          <NativeSelectOption value="redistribute">
            Redistribute to other members
          </NativeSelectOption>
        </NativeSelect>
      </Field>
    </section>
  );
}
