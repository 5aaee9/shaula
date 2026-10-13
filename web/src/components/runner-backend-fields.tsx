import type {
  ForgejoScope,
  ForgejoTarget,
  GitlabTarget,
  RunnerBackend,
} from "@/lib/runner-backend";
import { Field } from "./status";
import { Input } from "./ui/input";
import { NativeSelect, NativeSelectOption } from "./ui/native-select";

export function BackendSelector({
  value,
  onChange,
}: {
  value: RunnerBackend;
  onChange: (value: RunnerBackend) => void;
}) {
  return (
    <Field label="Runner backend">
      <NativeSelect value={value} onChange={(e) => onChange(e.target.value as RunnerBackend)}>
        <NativeSelectOption value="github">GitHub Actions</NativeSelectOption>
        <NativeSelectOption value="forgejo">Forgejo</NativeSelectOption>
        <NativeSelectOption value="gitlab">GitLab</NativeSelectOption>
      </NativeSelect>
      <p className="text-sm text-muted-foreground">Changing backend starts a new configuration.</p>
    </Field>
  );
}

export function ForgejoTargetFields({
  value,
  onChange,
  disabled = false,
}: {
  value: ForgejoTarget;
  onChange: (target: ForgejoTarget) => void;
  disabled?: boolean;
}) {
  const scope = value.scope;
  return (
    <div className="form-stack min-w-0">
      <Field label="Forgejo instance URL">
        <Input
          required
          type="url"
          disabled={disabled}
          value={value.instance_url}
          placeholder="https://forgejo.example.com"
          onChange={(e) => onChange({ ...value, instance_url: e.target.value })}
        />
      </Field>
      <Field label="Forgejo scope">
        <NativeSelect
          disabled={disabled}
          value={scope.kind}
          onChange={(e) => {
            const kind = e.target.value as ForgejoScope["kind"];
            onChange({
              ...value,
              scope:
                kind === "repository"
                  ? { kind, owner: "", name: "" }
                  : kind === "organization"
                    ? { kind, name: "" }
                    : { kind },
            });
          }}
        >
          <NativeSelectOption value="instance">Instance (administrator)</NativeSelectOption>
          <NativeSelectOption value="organization">Organization</NativeSelectOption>
          <NativeSelectOption value="repository">Repository</NativeSelectOption>
          <NativeSelectOption value="user">Token owner</NativeSelectOption>
        </NativeSelect>
      </Field>
      {scope.kind === "repository" && (
        <Field label="Repository owner">
          <Input
            required
            disabled={disabled}
            value={scope.owner}
            onChange={(e) => onChange({ ...value, scope: { ...scope, owner: e.target.value } })}
          />
        </Field>
      )}
      {(scope.kind === "repository" || scope.kind === "organization") && (
        <Field label={scope.kind === "repository" ? "Repository" : "Organization"}>
          <Input
            required
            disabled={disabled}
            value={scope.name}
            onChange={(e) => onChange({ ...value, scope: { ...scope, name: e.target.value } })}
          />
        </Field>
      )}
    </div>
  );
}

export function GitlabTargetFields({
  value,
  onChange,
  disabled = false,
}: {
  value: GitlabTarget;
  onChange: (target: GitlabTarget) => void;
  disabled?: boolean;
}) {
  const scope = value.scope;
  const project = scope.kind === "project";
  return (
    <div className="form-stack min-w-0">
      <Field label="GitLab instance URL">
        <Input
          required
          type="url"
          disabled={disabled}
          value={value.instance_url}
          placeholder="https://gitlab.example.com"
          onChange={(e) => onChange({ ...value, instance_url: e.target.value })}
        />
      </Field>
      <Field label="GitLab scope">
        <NativeSelect
          disabled={disabled}
          value={scope.kind}
          onChange={(e) =>
            onChange({
              ...value,
              scope: { kind: e.target.value as GitlabTarget["scope"]["kind"], path: scope.path },
            })
          }
        >
          <NativeSelectOption value="group">Group</NativeSelectOption>
          <NativeSelectOption value="project">Project</NativeSelectOption>
        </NativeSelect>
      </Field>
      <Field label={project ? "Project path" : "Group path"}>
        <Input
          required
          disabled={disabled}
          value={scope.path}
          // Project paths always include their namespace.
          pattern={project ? "[^/]+(/[^/]+)+" : "[^/]+(/[^/]+)*"}
          title={project ? "Full path, for example my-group/my-project" : undefined}
          placeholder={project ? "my-group/my-project" : "my-group/subgroup"}
          onChange={(e) => onChange({ ...value, scope: { ...scope, path: e.target.value } })}
        />
      </Field>
    </div>
  );
}
