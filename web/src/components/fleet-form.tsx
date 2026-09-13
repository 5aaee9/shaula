import { NativeSelect, NativeSelectOption } from "@/components/ui/native-select";
import { useRef, useState, type FormEvent } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { LoaderCircle, Save } from "lucide-react";
import { api, MutationAttempt, resourcePath, type Resource } from "@/lib/api";
import { useAuthProfiles, useTemplatePools, useTemplates } from "@/lib/queries";
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
import { initialPoolMember, TemplatePoolEditor } from "./template-pool-editor";
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

/** How a fleet routes new runners to templates (spec 0037 §4). */
type Placement = "single" | "pool" | "shared";

/** One form stage: a quiet left rail that names the facet, beside its controls. */
function Stage({
  title,
  description,
  children,
}: {
  title: string;
  description: string;
  children: React.ReactNode;
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
  const [placement, setPlacement] = useState<Placement>(
    initialPool ? "pool" : resource?.data.spec.template_pool_ref ? "shared" : "single",
  );
  const [poolMembers, setPoolMembers] = useState<TemplatePoolMemberSpec[]>(
    initialPool?.members || [initialPoolMember(0)],
  );
  const [failurePolicy, setFailurePolicy] = useState<"backpressure" | "redistribute">(
    initialPool?.failure_policy || "backpressure",
  );
  const [poolRef, setPoolRef] = useState(resource?.data.spec.template_pool_ref || "");
  const editor = useFleetInputs(resource, scopes.includes("template.read"));
  const [labels, setLabels] = useState(spec.github.labels.join(", "));
  const [error, setError] = useState<unknown>(null);
  const [busy, setBusy] = useState(false);
  const [advancedOpen, setAdvancedOpen] = useState(false);
  const attempt = useRef(new MutationAttempt());
  const canReadAuth = scopes.includes("auth.read");
  const canReadTemplates = scopes.includes("template.read");
  const templates = useTemplates(editor.canRead);
  const templatePools = useTemplatePools(placement === "shared" && canReadTemplates);
  const authProfiles = useAuthProfiles(canReadAuth);
  const originalAuth = resource?.data.spec.github.auth_profile_ref;
  const authAllowed =
    (!!resource && spec.github.auth_profile_ref === originalAuth) ||
    canChooseProfile(authProfiles, spec.github.auth_profile_ref, canReadAuth);
  const templateAllowed =
    (!!resource && editor.reference === resource.data.spec.template_profile_ref) ||
    canChooseProfile(templates, editor.template, editor.canRead);
  const poolChosen = placement !== "shared" || !!poolRef.trim();
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
      if (placement === "single" && !editor.canSubmit)
        throw new Error("Load and review the template input contract before submitting.");
      if (!authAllowed || (placement === "single" && !templateAllowed))
        throw new Error(
          "Choose available profiles from a successfully loaded list before submitting.",
        );
      if (placement === "shared" && !poolRef.trim())
        throw new Error("Choose a template pool for this fleet.");
      if (spec.capacity.min_runners > spec.capacity.max_runners) {
        setAdvancedOpen(true);
        throw new Error("Minimum runners cannot exceed maximum runners.");
      }
      if (placement === "pool") {
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
      // The submitted spec carries exactly one template source; omitted
      // keys stay absent rather than empty.
      const base = {
        ...spec,
        github: {
          ...spec.github,
          scale_set_name: resource ? spec.github.scale_set_name : spec.github.scale_set_name || key,
          labels: labels
            .split(",")
            .map((value) => value.trim())
            .filter(Boolean),
        },
        template_profile_ref: undefined,
        template_pool: undefined,
        template_pool_ref: undefined,
        template_inputs: undefined,
      };
      let body: string;
      if (placement === "single") {
        const single = JSON.stringify({ ...base, template_profile_ref: editor.reference });
        body = `${single.slice(0, -1)},"template_inputs":${editor.json}}`;
      } else if (placement === "pool") {
        body = JSON.stringify({
          ...base,
          template_pool: { members: poolMembers, failure_policy: failurePolicy },
        });
      } else {
        body = JSON.stringify({ ...base, template_pool_ref: poolRef.trim() });
      }
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
  const placementDescription =
    placement === "single"
      ? "All runners in this fleet use one template profile."
      : placement === "pool"
        ? "Each new runner draws one member by weight. Weights shape the long-run mix across backends."
        : "This fleet draws runners from a shared pool resource; multiple fleets can use the same mix.";
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

          <Stage title="Template placement" description={placementDescription}>
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
                    ["shared", "Shared pool"],
                  ] as const
                ).map(([value, label]) => (
                  <button
                    key={value}
                    type="button"
                    role="radio"
                    aria-checked={placement === value}
                    onClick={() => setPlacement(value)}
                    className={cn(
                      "rounded-[5px] px-3 py-1.5 text-sm text-muted-foreground transition-colors",
                      placement === value && "bg-background font-medium text-foreground shadow-sm",
                    )}
                  >
                    {label}
                  </button>
                ))}
              </div>
            )}
            {placement === "single" ? (
              <>
                <FleetTemplateSelection editor={editor} templates={templates} />
                <FleetTemplateInputs editor={editor} />
              </>
            ) : placement === "pool" ? (
              <TemplatePoolEditor
                members={poolMembers}
                setMembers={setPoolMembers}
                failurePolicy={failurePolicy}
                setFailurePolicy={setFailurePolicy}
                canRead={editor.canRead}
              />
            ) : (
              <Field label="Template pool">
                <NativeSelect
                  required
                  value={poolRef}
                  onChange={(event) => setPoolRef(event.target.value)}
                >
                  <NativeSelectOption value="">Choose a pool</NativeSelectOption>
                  {(templatePools.data?.data.pools || []).map((pool) => (
                    <NativeSelectOption key={pool.key} value={pool.key}>
                      {pool.key} · revision {pool.revision}
                    </NativeSelectOption>
                  ))}
                </NativeSelect>
                {resource && resource.data.spec.template_pool_ref && (
                  <p className="text-muted-foreground text-sm">
                    Switching pools re-routes every future draw and needs the fleet to be empty.
                  </p>
                )}
                <p className="text-muted-foreground text-sm">
                  The pool defines members, weights and caps. Each member follows its template's
                  latest Active revision; this fleet catches up automatically when it drains.
                </p>
              </Field>
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
              busy ||
              !poolChosen ||
              (placement === "single" && (!editor.canSubmit || !templateAllowed)) ||
              !authAllowed
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
