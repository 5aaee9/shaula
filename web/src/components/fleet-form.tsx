import { NativeSelect, NativeSelectOption } from "@/components/ui/native-select";
import { useRef, useState, type FormEvent, type ReactNode } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { LoaderCircle, Save, Plus, Trash2, CircleAlert } from "lucide-react";
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
import { cn } from "cn";

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

/** One form stage: a quiet left rail that names the facet, beside its controls. */
function Stage({
  title,
  description,
  children,
}: {
  title: string;
  description: string;
  children: ReactNode;
}) {
  return (
    <section className="grid grid-cols-1 gap-4 border-t pt-6 first:border-t-0 first:pt-0 sm:grid-cols-[11rem_1fr] sm:gap-6">
      <div className="min-w-0">
        <h2 className="text-sm font-medium">{title}</h2>
        <p className="mt-1 text-sm text-muted-foreground">{description}</p>
      </div>
      <div className="form-stack min-w-0">{children}</div>
    </section>
  );
}

export function FleetForm({
  resource: currentResource,
  scopes,
  onClose,
  onAccepted,
  page = false,
}: {
  resource?: Resource<FleetResource>;
  scopes: string[];
  onClose: () => void;
  onAccepted: (change: ChangeRef) => void;
  page?: boolean;
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
      if (!page) onClose();
    } catch (error) {
      setError(error);
    } finally {
      setBusy(false);
    }
  }
  return (
    <Modal
      open
      inline={page}
      title={resource ? `Edit ${key}` : "Create fleet"}
      className="sm:max-w-2xl"
      onOpenChange={(open) => {
        if (!open && !busy) onClose();
      }}
    >
      <form onSubmit={submit} className="form-stack">
        <fieldset disabled={busy} className="form-stack">
          <Stage
            title="Fleet"
            description={
              resource
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
                disabled={!!resource}
                onChange={(event) => setKey(event.target.value)}
                placeholder="linux-build"
              />
            </Field>
          </Stage>

          <Stage
            title="GitHub target"
            description={
              resource
                ? "The organization or repository this fleet serves is fixed at creation."
                : "Where GitHub Actions jobs for this fleet run."
            }
          >
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
          </Stage>

          <Stage
            title="Authentication"
            description="The GitHub App profile used to manage runners for this fleet."
          >
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
                className="self-start"
                onClick={() => github({ auth_profile_ref: originalAuth })}
              >
                Restore original authentication
              </Button>
            )}
          </Stage>

          <Stage
            title="Template placement"
            description={
              poolMode
                ? "Each new runner draws one member by weight. Weights shape the long-run mix across backends."
                : "All runners in this fleet use one template profile."
            }
          >
            {!resource && (
              <div
                role="radiogroup"
                aria-label="Template placement"
                className="inline-flex w-fit rounded-md border bg-muted/40 p-0.5"
              >
                {(
                  [
                    ["single", "Single template"],
                    ["pool", "Weighted pool"],
                  ] as const
                ).map(([value, label]) => (
                  <button
                    key={value}
                    type="button"
                    role="radio"
                    aria-checked={poolMode === (value === "pool")}
                    onClick={() => setPoolMode(value === "pool")}
                    className={cn(
                      "rounded-[5px] px-3 py-1.5 text-sm text-muted-foreground transition-colors",
                      poolMode === (value === "pool") &&
                        "bg-background font-medium text-foreground shadow-sm",
                    )}
                  >
                    {label}
                  </button>
                ))}
              </div>
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
          </Stage>

          <Stage title="Capacity" description="How many concurrent runners this fleet may run.">
            <div className="form-grid">
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
            </div>
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
          </Stage>
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

/** A categorical ramp: readable on white, distinct enough to tell members apart. */
const POOL_TONES = [
  "bg-slate-500",
  "bg-sky-600",
  "bg-amber-500",
  "bg-emerald-600",
  "bg-violet-600",
  "bg-rose-500",
  "bg-cyan-600",
  "bg-lime-600",
];

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
  const total = members.reduce((sum, member) => sum + Math.max(0, member.weight), 0);
  const shares = members.map((member) =>
    total > 0 && member.weight > 0 ? member.weight / total : 0,
  );
  return (
    <div className="form-stack" aria-label="Weighted template pool">
      {/* Proportional share bar: weight[i] / Σweight, the spec's draw semantics. */}
      <div>
        <div
          className="flex h-2 w-full gap-px overflow-hidden rounded-full bg-muted"
          role="img"
          aria-label="Member weight shares"
        >
          {members.map((member, index) => (
            <div
              key={member.key || index}
              className={cn(POOL_TONES[index % POOL_TONES.length], "transition-all")}
              style={{ flexGrow: Math.max(member.weight, 0) || (total === 0 ? 1 : 0) }}
              title={`${member.key || `member-${index + 1}`} · ${Math.round(shares[index] * 100)}%`}
            />
          ))}
        </div>
        <p className="mt-2 text-sm text-muted-foreground">
          Each new runner draws a member independently by weight. Weights shape the long-run mix;
          finite batches can vary and GitHub job routing is not controlled.
        </p>
      </div>

      {members.map((member, index) => {
        const share = shares[index];
        const tone = POOL_TONES[index % POOL_TONES.length];
        return (
          <MemberRow
            key={index}
            index={index}
            member={member}
            tone={tone}
            share={share}
            available={available}
            canRemove={members.length > 1}
            onChange={(next) => setMembers(members.map((m, i) => (i === index ? next : m)))}
            onRemove={() => setMembers(members.filter((_, i) => i !== index))}
          />
        );
      })}

      <div className="flex items-center justify-between gap-3">
        <Button
          type="button"
          variant="outline"
          disabled={members.length >= 32}
          onClick={() => setMembers([...members, initialMember(members.length)])}
        >
          <Plus /> Add member
        </Button>
        <p className="text-sm text-muted-foreground">
          {members.length} of 32 members · integer weights 1–10000
        </p>
      </div>

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
        <p className="text-sm text-muted-foreground">
          When a member's backend fails, hold capacity (backpressure) or redraw onto the remaining
          members (redistribute).
        </p>
      </Field>
    </div>
  );
}

function MemberRow({
  index,
  member,
  tone,
  share,
  available,
  canRemove,
  onChange,
  onRemove,
}: {
  index: number;
  member: TemplatePoolMemberSpec;
  tone: string;
  share: number;
  available: ReturnType<typeof useTemplates>["data"] extends infer D
    ? D extends { data: { profiles: infer P } }
      ? P
      : never
    : never;
  canRemove: boolean;
  onChange: (member: TemplatePoolMemberSpec) => void;
  onRemove: () => void;
}) {
  const [inputs, setInputs] = useState(() => JSON.stringify(member.template_inputs, null, 2));
  const [inputsError, setInputsError] = useState<string | null>(null);
  const keyInvalid = member.key !== "" && !/^[a-z0-9][a-z0-9-]*$/.test(member.key);
  const weightInvalid =
    !Number.isInteger(member.weight) || member.weight < 1 || member.weight > 10000;
  return (
    <div className="grid grid-cols-1 gap-4 border-t py-4 first:border-t-0 first:pt-0 sm:grid-cols-[auto_1fr_auto] sm:items-start sm:gap-5">
      {/* Share readout: the member's slice of the draw. */}
      <div className="flex items-center gap-2 sm:w-24 sm:flex-col sm:items-start sm:gap-1">
        <span className={cn("mt-0.5 size-2.5 shrink-0 rounded-[3px]", tone)} aria-hidden="true" />
        <span className="text-lg font-semibold tabular-nums leading-none">
          {Math.round(share * 100)}
          <span className="text-sm font-normal text-muted-foreground">%</span>
        </span>
        <span className="text-xs text-muted-foreground">of draws</span>
      </div>

      <div className="form-stack min-w-0">
        <div className="form-grid">
          <Field label="Member key">
            <Input
              value={member.key}
              aria-invalid={keyInvalid || undefined}
              onChange={(e) => onChange({ ...member, key: e.target.value })}
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
              aria-invalid={weightInvalid || undefined}
              onChange={(e) => onChange({ ...member, weight: Number(e.target.value) })}
              required
            />
          </Field>
        </div>
        {(keyInvalid || weightInvalid) && (
          <p role="alert" className="flex items-center gap-1.5 text-sm text-destructive">
            <CircleAlert className="size-4 shrink-0" />
            {keyInvalid
              ? "Member key must be lowercase letters, digits, and dashes."
              : "Weight must be an integer from 1 to 10000."}
          </p>
        )}
        <Field label="Template profile">
          <NativeSelect
            value={member.template_profile_ref}
            required
            onChange={(e) => onChange({ ...member, template_profile_ref: e.target.value })}
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
            className={cn(
              "min-h-20 w-full rounded-md border bg-background p-2 font-mono text-sm",
              inputsError && "border-destructive",
            )}
            value={inputs}
            aria-invalid={!!inputsError || undefined}
            onChange={(e) => {
              const raw = e.target.value;
              setInputs(raw);
              try {
                const parsed = JSON.parse(raw);
                if (parsed && typeof parsed === "object" && !Array.isArray(parsed)) {
                  onChange({ ...member, template_inputs: parsed });
                  setInputsError(null);
                } else {
                  setInputsError("Inputs must be a JSON object.");
                }
              } catch {
                setInputsError("Finish the JSON object — it isn't valid yet.");
              }
            }}
          />
        </Field>
        {inputsError && (
          <p role="alert" className="flex items-center gap-1.5 text-sm text-destructive">
            <CircleAlert className="size-4 shrink-0" />
            {inputsError}
          </p>
        )}
        <Field label="Maximum runners (optional)">
          <Input
            type="number"
            min={0}
            step={1}
            value={member.max_runners ?? ""}
            onChange={(e) =>
              onChange({
                ...member,
                max_runners: e.target.value === "" ? undefined : Number(e.target.value),
              })
            }
          />
        </Field>
      </div>

      <Button
        type="button"
        variant="ghost"
        size="icon"
        aria-label={`Remove ${member.key || `member ${index + 1}`}`}
        disabled={!canRemove}
        className="sm:mt-6"
        onClick={onRemove}
      >
        <Trash2 />
      </Button>
    </div>
  );
}
