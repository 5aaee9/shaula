import { useState } from "react";
import type { Resource } from "@/lib/api";
import type { AuthResource, ChangeRef } from "@/lib/types";
import type { ForgejoTarget, GitlabTarget, RunnerBackend } from "@/lib/runner-backend";
import { BackendSelector, ForgejoTargetFields, GitlabTargetFields } from "./runner-backend-fields";
import { GitHubAuthForm } from "./github-auth-form";
import { TokenAuthForm } from "./token-auth-form";

export type AuthFormMode = "create" | "policy" | "rotate";
export type AuthFormProps = {
  onClose: () => void;
  onAccepted: (change: ChangeRef) => void;
} & (
  | { mode: "create"; resource?: never }
  | { mode: "policy" | "rotate"; resource: Resource<AuthResource> }
);

function profileBackend(kind: string | null | undefined): RunnerBackend {
  return kind === "forgejo_token" ? "forgejo" : kind === "gitlab_token" ? "gitlab" : "github";
}

export function AuthForm(props: AuthFormProps) {
  const [backend, setBackend] = useState(profileBackend(props.resource?.data.kind));
  const selector = props.mode === "create" && (
    <BackendSelector value={backend} onChange={setBackend} />
  );
  if (backend === "forgejo")
    return (
      <TokenAuthForm<ForgejoTarget>
        {...props}
        backendSelector={selector}
        provider="Forgejo"
        kind="forgejo_token"
        initialTarget={{ instance_url: "", scope: { kind: "instance" } }}
        activeTarget={props.resource?.data.active?.forgejo?.target}
        targetFields={(target, setTarget, disabled) => (
          <ForgejoTargetFields value={target} onChange={setTarget} disabled={disabled} />
        )}
        guidance="Instance scope needs an administrator token; other scopes need runner management on that target."
      />
    );
  if (backend === "gitlab")
    return (
      <TokenAuthForm<GitlabTarget>
        {...props}
        backendSelector={selector}
        provider="GitLab"
        kind="gitlab_token"
        initialTarget={{ instance_url: "", scope: { kind: "group", path: "" } }}
        activeTarget={props.resource?.data.active?.gitlab?.target}
        targetFields={(target, setTarget, disabled) => (
          <GitlabTargetFields value={target} onChange={setTarget} disabled={disabled} />
        )}
        guidance="Group or project token · Owner or Maintainer · create_runner, manage_runner, read_api"
      />
    );
  return <GitHubAuthForm {...props} backendSelector={selector} />;
}
