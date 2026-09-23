import { useRef, useState, type FormEvent, type ReactNode } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { LoaderCircle, Save } from "lucide-react";
import { api, MutationAttempt, resourcePath, type Resource } from "@/lib/api";
import { useAuthProfiles, useTemplatePools, useTemplates } from "@/lib/queries";
import { canChooseProfile } from "@/lib/profile-choice";
import { authChoices, forgejoTargetName, templateChoices } from "@/lib/runner-backend";
import { useFleetInputs } from "@/lib/use-fleet-inputs";
import type { Accepted, FleetResource, ForgejoFleetSpec, TemplatePoolSpec } from "@/lib/types";
import type { FleetFormProps } from "./fleet-form";
import { FleetTemplateSelection } from "./fleet-template-selection";
import { FleetTemplateInputs } from "./fleet-template-inputs";
import { ProfileSelector } from "./profile-selector";
import { Modal } from "./modal";
import { Button } from "./ui/button";
import { Input } from "./ui/input";
import { NativeSelect, NativeSelectOption } from "./ui/native-select";
import { initialPoolMember, TemplatePoolEditor } from "./template-pool-editor";
import { ErrorNotice, Field } from "./status";

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
  const [key, setKey] = useState(resource?.data.key || "");
  const [placement, setPlacement] = useState<"single" | "pool" | "shared">(
    original?.template_pool ? "pool" : original?.template_pool_ref ? "shared" : "single",
  );
  const [pool, setPool] = useState<TemplatePoolSpec>(
    original?.template_pool || {
      members: [initialPoolMember(0)],
      failure_policy: "backpressure",
    },
  );
  const [poolRef, setPoolRef] = useState(original?.template_pool_ref || "");
  const [auth, setAuth] = useState(original?.forgejo.auth_profile_ref || "");
  const [prefix, setPrefix] = useState(original?.forgejo.runner_name_prefix || "");
  const [labels, setLabels] = useState(original?.forgejo.labels.join(", ") || "");
  const [capacity, setCapacity] = useState(
    original?.capacity || { min_runners: 0, max_runners: 10 },
  );
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const client = useQueryClient();
  const attempt = useRef(new MutationAttempt());
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
  const pools = useTemplatePools(placement === "shared" && editor.canRead);
  const templates = templateChoices(useTemplates(editor.canRead), "forgejo");
  const authAllowed = !!resource || (canChooseProfile(choices, auth, canReadAuth) && !!target);
  const templateAllowed =
    (!!resource && editor.reference === original?.template_profile_ref) ||
    canChooseProfile(templates, editor.template, editor.canRead);
  const placementAllowed =
    placement === "single"
      ? templateAllowed && editor.canSubmit
      : placement === "shared"
        ? (!!resource && poolRef === original?.template_pool_ref) ||
          (editor.canRead &&
            pools.isSuccess &&
            !!pools.data?.data.pools.some((p) => p.key === poolRef))
        : (!!resource && JSON.stringify(pool) === JSON.stringify(original?.template_pool)) ||
          (pool.members.length > 0 &&
            pool.members.every((m) =>
              canChooseProfile(templates, m.template_profile_ref, editor.canRead),
            ));

  async function submit(event: FormEvent) {
    event.preventDefault();
    setError(null);
    try {
      if (!target || !authAllowed || !placementAllowed)
        throw new Error(
          "Choose an available Forgejo authentication profile and compatible template placement before submitting.",
        );
      if (placement === "pool") {
        const keys = new Set(pool.members.map((m) => m.key));
        if (
          keys.size !== pool.members.length ||
          pool.members.length > 32 ||
          pool.members.some(
            (m) =>
              !/^[a-z0-9][a-z0-9-]*$/.test(m.key) ||
              !Number.isInteger(m.weight) ||
              m.weight < 1 ||
              m.weight > 10000 ||
              (m.max_runners !== undefined &&
                (!Number.isInteger(m.max_runners) || m.max_runners < 0)),
          )
        )
          throw new Error(
            "Use 1–32 uniquely named members, integer weights 1–10000 and non-negative runner caps.",
          );
      }
      if (capacity.min_runners > capacity.max_runners)
        throw new Error("Minimum runners cannot exceed maximum runners.");
      const names = labels
        .split(",")
        .map((value) => value.trim())
        .filter(Boolean);
      if (!names.length)
        throw new Error(
          "Declare at least one label with an explicit backend target, for example linux:host.",
        );
      const spec: ForgejoFleetSpec = {
        kind: "forgejo",
        forgejo: {
          instance_url: target.instance_url,
          scope: target.scope,
          auth_profile_ref: auth,
          runner_name_prefix: prefix || `shaula-${key}-`,
          labels: names,
        },
        capacity,
      };
      let body: string;
      if (placement === "single") {
        const json = JSON.stringify({ ...spec, template_profile_ref: editor.reference });
        body = `${json.slice(0, -1)},"template_inputs":${editor.json}}`;
      } else {
        body = JSON.stringify({
          ...spec,
          ...(placement === "pool" ? { template_pool: pool } : { template_pool_ref: poolRef }),
        });
      }
      const path = resourcePath("fleets", key);
      setBusy(true);
      const result = await api<Accepted>(path, {
        method: "PUT",
        body,
        headers: attempt.current.headers("PUT", path, body, resource?.etag || null, !resource),
      });
      void client.invalidateQueries({ queryKey: ["fleets"] });
      void client.invalidateQueries({ queryKey: ["fleet", key] });
      void client.invalidateQueries({ queryKey: ["fleet-status", key] });
      onAccepted({ id: result.data.changeId, resource: key, type: "fleet" });
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
      <form className="form-stack" onSubmit={submit}>
        <fieldset disabled={busy} className="form-stack">
          {backendSelector}
          <Field label="Fleet key">
            <Input
              required
              disabled={!!resource}
              pattern="[a-z0-9][a-z0-9-]*"
              maxLength={63}
              value={key}
              onChange={(e) => setKey(e.target.value)}
              placeholder="forgejo-linux"
            />
          </Field>
          <section className="form-stack border-t pt-6">
            <h2>Forgejo authentication</h2>
            <ProfileSelector
              label="Forgejo authentication profile"
              query={choices}
              value={auth}
              canRead={canReadAuth}
              permission="auth.read"
              disabled={!!resource}
              onChange={setAuth}
            />
            {target && (
              <p className="break-all text-sm" aria-label="Forgejo target">
                {forgejoTargetName(target)}
              </p>
            )}
            <p className="text-sm text-muted-foreground">
              The instance and scope come from the profile's Active revision. Target, profile,
              runner prefix and labels are fixed at fleet creation. Rotate the profile's token to
              replace credentials.
            </p>
            <Field label="Runner name prefix">
              <Input
                disabled={!!resource}
                maxLength={100}
                value={prefix}
                onChange={(e) => setPrefix(e.target.value)}
                placeholder={key ? `shaula-${key}-` : "Uses shaula-<fleet key>-"}
              />
            </Field>
            <Field label="Labels">
              <Input
                required
                disabled={!!resource}
                value={labels}
                onChange={(e) => setLabels(e.target.value)}
                placeholder="linux:host"
              />
            </Field>
            <p className="text-sm text-muted-foreground">
              Use a unique prefix per fleet. Every workflow runs-on label must be declared by the
              runner. Bundled templates support explicit :host targets only. Do not share labels
              with persistent runners: they may consume the same queued jobs.
            </p>
          </section>
          <section className="form-stack border-t pt-6">
            <h2>Template placement</h2>
            <Field label="Template placement">
              <NativeSelect
                value={placement}
                disabled={!!resource}
                onChange={(e) => setPlacement(e.target.value as typeof placement)}
              >
                <NativeSelectOption value="single">Single template</NativeSelectOption>
                <NativeSelectOption value="pool">Weighted pool</NativeSelectOption>
                <NativeSelectOption value="shared">Shared pool</NativeSelectOption>
              </NativeSelect>
            </Field>
            <p className="text-sm text-muted-foreground">
              Every member must use the Forgejo runner backend and support this fleet's label
              targets. Weights choose infrastructure for new runners, not which job they receive.
            </p>
            {placement === "single" ? (
              <>
                <FleetTemplateSelection editor={editor} templates={templates} />
                <FleetTemplateInputs editor={editor} />
              </>
            ) : placement === "pool" ? (
              <TemplatePoolEditor
                members={pool.members}
                setMembers={(members) => setPool({ ...pool, members })}
                failurePolicy={pool.failure_policy}
                setFailurePolicy={(failure_policy) => setPool({ ...pool, failure_policy })}
                canRead={editor.canRead}
                backend="forgejo"
              />
            ) : (
              <Field label="Template pool">
                <NativeSelect required value={poolRef} onChange={(e) => setPoolRef(e.target.value)}>
                  <NativeSelectOption value="">Choose a pool</NativeSelectOption>
                  {original?.template_pool_ref &&
                    !pools.data?.data.pools.some((p) => p.key === original.template_pool_ref) && (
                      <NativeSelectOption value={original.template_pool_ref}>
                        {original.template_pool_ref} · retained
                      </NativeSelectOption>
                    )}
                  {(pools.data?.data.pools || []).map((p) => (
                    <NativeSelectOption key={p.key} value={p.key}>
                      {p.key} · revision {p.revision}
                    </NativeSelectOption>
                  ))}
                </NativeSelect>
                {pools.error && <ErrorNotice error={pools.error} />}
                <p className="text-sm text-muted-foreground">
                  {editor.canRead
                    ? "Pool members are checked by the server. Switching pools or following an updated revision waits for this fleet to drain."
                    : "Template read permission is required to choose a different pool. The existing reference can be retained."}
                </p>
              </Field>
            )}
          </section>
          <section className="form-stack border-t pt-6">
            <h2>Capacity</h2>
            <div className="form-grid">
              <Field label="Minimum runners">
                <Input
                  required
                  type="number"
                  min={0}
                  step={1}
                  value={capacity.min_runners}
                  onChange={(e) =>
                    setCapacity({ ...capacity, min_runners: Number(e.target.value) })
                  }
                />
              </Field>
              <Field label="Maximum runners">
                <Input
                  required
                  type="number"
                  min={0}
                  step={1}
                  value={capacity.max_runners}
                  onChange={(e) =>
                    setCapacity({ ...capacity, max_runners: Number(e.target.value) })
                  }
                />
              </Field>
            </div>
            <p className="text-sm text-muted-foreground">
              Minimum runners stay waiting and consume infrastructure. Each ephemeral runner
              executes at most one job. The server's maximum lifetime can interrupt running jobs;
              token rotation may require runners to drain and be recreated.
            </p>
          </section>
        </fieldset>
        {error !== null && <ErrorNotice error={error} />}
        <div className="dialog-actions">
          <Button type="button" variant="outline" disabled={busy} onClick={onClose}>
            Cancel
          </Button>
          <Button type="submit" disabled={busy || !authAllowed || !placementAllowed}>
            {busy ? <LoaderCircle className="animate-spin" /> : <Save />}
            {resource ? "Save changes" : "Create fleet"}
          </Button>
        </div>
      </form>
    </Modal>
  );
}
