import { NativeSelect, NativeSelectOption } from "@/components/ui/native-select";
import { useRef, useState, type FormEvent } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { LoaderCircle, Save } from "lucide-react";
import { api, MutationAttempt, resourcePath, type Resource } from "@/lib/api";
import { useTemplates } from "@/lib/queries";
import type { Accepted, ChangeRef, FleetResource, FleetSpec } from "@/lib/types";
import { AdvancedSettings } from "./advanced-settings";
import { Modal } from "./modal";
import { Button } from "./ui/button";
import { Input } from "./ui/input";
import { Textarea } from "./ui/textarea";
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
  const [inputs, setInputs] = useState(JSON.stringify(spec.template_inputs, null, 2));
  const [labels, setLabels] = useState(spec.github.labels.join(", "));
  const [error, setError] = useState<unknown>(null);
  const [busy, setBusy] = useState(false);
  const [advancedOpen, setAdvancedOpen] = useState(false);
  const attempt = useRef(new MutationAttempt());
  const templates = useTemplates(scopes.includes("template.read"));
  const currentTemplate =
    typeof spec.template_profile_ref === "string"
      ? spec.template_profile_ref
      : spec.template_profile_ref.key;
  const [template, setTemplate] = useState(currentTemplate);
  const [revision, setRevision] = useState(
    typeof spec.template_profile_ref === "string" ? "" : String(spec.template_profile_ref.revision),
  );
  const customSettings = [
    !!spec.github.scale_set_name && spec.github.scale_set_name !== key,
    spec.github.runner_group !== "Default",
    spec.capacity.min_runners !== 0,
    !!labels.trim(),
    !!revision,
    !/^\{\s*\}$/.test(inputs.trim()),
  ].filter(Boolean).length;
  function github(patch: Partial<FleetSpec["github"]>) {
    setSpec({ ...spec, github: { ...spec.github, ...patch } });
  }
  async function submit(event: FormEvent) {
    event.preventDefault();
    setError(null);
    try {
      let parsed: unknown;
      try {
        parsed = JSON.parse(inputs);
        if (!parsed || typeof parsed !== "object" || Array.isArray(parsed))
          throw new Error("Template inputs must be a JSON object.");
        if (spec.capacity.min_runners > spec.capacity.max_runners)
          throw new Error("Minimum runners cannot exceed maximum runners.");
      } catch (error) {
        setAdvancedOpen(true);
        throw error;
      }
      const body = JSON.stringify({
        ...spec,
        github: {
          ...spec.github,
          scale_set_name: resource ? spec.github.scale_set_name : spec.github.scale_set_name || key,
          labels: labels
            .split(",")
            .map((value) => value.trim())
            .filter(Boolean),
        },
        template_inputs: parsed,
        template_profile_ref: revision ? { key: template, revision: Number(revision) } : template,
      });
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
          <Field label="GitHub authentication profile">
            <Input
              required
              value={spec.github.auth_profile_ref}
              onChange={(event) => github({ auth_profile_ref: event.target.value })}
              placeholder="github-build"
            />
          </Field>
          <Field label="Template profile">
            <Input
              required
              list="template-keys"
              value={template}
              onChange={(event) => setTemplate(event.target.value)}
              placeholder="kubernetes-linux"
            />
            <datalist id="template-keys">
              {templates.data?.data.profiles
                .filter((value) => value.activeRevision)
                .map((value) => (
                  <option key={value.key} value={value.key} />
                ))}
            </datalist>
          </Field>
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
                ? `${customSettings} custom ${customSettings === 1 ? "setting" : "settings"}`
                : "Default runner group, 0 minimum runners, active template revision"
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
              <Field label="Pinned revision">
                <Input
                  type="number"
                  min={1}
                  step={1}
                  value={revision}
                  placeholder="Current active revision"
                  onChange={(event) => setRevision(event.target.value)}
                />
              </Field>
            </div>
            <Field label="Labels">
              <Input
                value={labels}
                onChange={(event) => setLabels(event.target.value)}
                placeholder="linux, x64"
              />
            </Field>
            <Field label="Template inputs (JSON)">
              <Textarea
                spellCheck={false}
                className="mono"
                value={inputs}
                onChange={(event) => setInputs(event.target.value)}
              />
            </Field>
          </AdvancedSettings>
        </fieldset>
        {error !== null && <ErrorNotice error={error} />}
        <div className="dialog-actions">
          <Button type="button" variant="outline" onClick={onClose} disabled={busy}>
            Cancel
          </Button>
          <Button disabled={busy} type="submit">
            {busy ? <LoaderCircle className="animate-spin" /> : <Save />}
            {resource ? "Save changes" : "Create fleet"}
          </Button>
        </div>
      </form>
    </Modal>
  );
}
