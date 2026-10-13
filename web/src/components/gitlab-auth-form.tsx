import type { ReactNode } from "react";
import type { GitlabTarget } from "@/lib/runner-backend";
import type { AuthFormProps } from "./auth-form";
import { GitlabTargetFields } from "./runner-backend-fields";
import { TokenAuthForm } from "./token-auth-form";

export function GitlabAuthForm(props: AuthFormProps & { backendSelector?: ReactNode }) {
  return (
    <TokenAuthForm<GitlabTarget>
      {...props}
      provider="GitLab"
      kind="gitlab_token"
      initialTarget={{ instance_url: "", scope: { kind: "group", path: "" } }}
      activeTarget={props.resource?.data.active?.gitlab?.target}
      targetFields={(target, setTarget, disabled) => (
        <GitlabTargetFields value={target} onChange={setTarget} disabled={disabled} />
      )}
      guidance="Use a group or project access token with the Owner or Maintainer role and the create_runner, manage_runner and read_api scopes. The token is write-only and is never sent to job runners. Shaula validates access before activation."
    />
  );
}
