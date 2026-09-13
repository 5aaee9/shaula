import { Plus, Trash2, CircleAlert } from "lucide-react";
import { useTemplates } from "@/lib/queries";
import { useMemberContract } from "@/lib/use-member-inputs";
import { inputEntries, inputsJson } from "@/lib/input-values";
import type { TemplatePoolMemberSpec } from "@/lib/types";
import { Button } from "./ui/button";
import { Input } from "./ui/input";
import { NativeSelect, NativeSelectOption } from "./ui/native-select";
import { ApprovedValueSelect } from "./template-input-values";
import { ErrorNotice, Field } from "./status";
import { cn } from "cn";

export const initialPoolMember = (index: number): TemplatePoolMemberSpec => ({
  key: `member-${index + 1}`,
  template_profile_ref: "",
  weight: 1,
  template_inputs: {},
});

/** A categorical ramp: readable on white, distinct enough to tell members apart. */
export const POOL_TONES = [
  "bg-slate-500",
  "bg-sky-600",
  "bg-amber-500",
  "bg-emerald-600",
  "bg-violet-600",
  "bg-rose-500",
  "bg-cyan-600",
  "bg-lime-600",
];

/**
 * The weighted member editor shared by the inline-pool fleet form and the
 * standalone pool resource form (spec 0037): a proportional share bar —
 * weight[i]/Σweight, the draw semantics — plus one structured member row
 * per template with its computed share.
 */
export function TemplatePoolEditor({
  members,
  setMembers,
  failurePolicy,
  setFailurePolicy,
  canRead,
}: {
  members: TemplatePoolMemberSpec[];
  setMembers: (members: TemplatePoolMemberSpec[]) => void;
  failurePolicy: "backpressure" | "redistribute";
  setFailurePolicy: (value: "backpressure" | "redistribute") => void;
  canRead: boolean;
}) {
  const templates = useTemplates(canRead);
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
            templates={templates}
            canRead={canRead}
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
          onClick={() => setMembers([...members, initialPoolMember(members.length)])}
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
  templates,
  canRead,
  canRemove,
  onChange,
  onRemove,
}: {
  index: number;
  member: TemplatePoolMemberSpec;
  tone: string;
  share: number;
  templates: ReturnType<typeof useTemplates>;
  canRead: boolean;
  canRemove: boolean;
  onChange: (member: TemplatePoolMemberSpec) => void;
  onRemove: () => void;
}) {
  const available = templates.data?.data.profiles || [];
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
        <MemberTemplateInputs
          templateKey={member.template_profile_ref}
          inputs={member.template_inputs}
          canRead={canRead}
          onChange={(inputs) => onChange({ ...member, template_inputs: inputs })}
        />
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

/**
 * A pool member's template inputs, rendered with the same approved-value
 * contract controls as the single-template path. The member's
 * `template_inputs` Record stays owned by the parent form; this component
 * resolves the Active revision's contract for the chosen template and edits
 * values within it.
 */
export function MemberTemplateInputs({
  templateKey,
  inputs,
  canRead,
  onChange,
}: {
  templateKey: string;
  inputs: Record<string, unknown>;
  canRead: boolean;
  onChange: (inputs: Record<string, unknown>) => void;
}) {
  const { contract, loading, error } = useMemberContract(templateKey, canRead);
  const values = inputEntries(JSON.stringify(inputs ?? {}));
  const write = (entries: Map<string, string>) =>
    onChange(JSON.parse(inputsJson(entries)) as Record<string, unknown>);
  const update = (field: string, value?: string) => {
    const next = new Map(values);
    if (value === undefined) next.delete(field);
    else next.set(field, value);
    write(next);
  };

  if (!templateKey.trim()) {
    return (
      <p className="text-sm text-muted-foreground">Choose a template to configure its inputs.</p>
    );
  }
  if (loading) {
    return (
      <p role="status" className="text-sm text-muted-foreground">
        Loading template inputs…
      </p>
    );
  }
  if (error) {
    return <ErrorNotice error={error} />;
  }
  if (!contract) return null;

  if (contract.mode === "presets") {
    return (
      <ApprovedValueSelect
        label="Input configuration"
        description="Choose one complete approved configuration for this member."
        required
        options={contract.presets}
        value={values.size ? inputsJson(values) : undefined}
        onChange={(value) => write(inputEntries(value ?? "{}"))}
      />
    );
  }
  if (!contract.fields.length) {
    return (
      <p className="text-sm text-muted-foreground">This template needs no input configuration.</p>
    );
  }
  return (
    <div className="form-stack">
      {contract.fields.map((field) => (
        <ApprovedValueSelect
          key={field.key}
          label={field.label}
          inputKey={field.key}
          description={field.description}
          required={field.required}
          options={field.options}
          value={values.get(field.key)}
          onChange={(value) => update(field.key, value)}
        />
      ))}
    </div>
  );
}
