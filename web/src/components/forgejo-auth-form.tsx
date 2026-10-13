import type { ReactNode } from "react";
import type { ForgejoTarget } from "@/lib/runner-backend";
import type { AuthFormProps } from "./auth-form";
import { ForgejoTargetFields } from "./runner-backend-fields";
import { TokenAuthForm } from "./token-auth-form";

export function ForgejoAuthForm(props: AuthFormProps & { backendSelector?: ReactNode }) {
  return (
    <TokenAuthForm<ForgejoTarget>
      {...props}
      provider="Forgejo"
      kind="forgejo_token"
      initialTarget={{ instance_url: "", scope: { kind: "instance" } }}
      activeTarget={props.resource?.data.active?.forgejo?.target}
      targetFields={(target, setTarget, disabled) => (
        <ForgejoTargetFields value={target} onChange={setTarget} disabled={disabled} />
      )}
      guidance="The token is write-only and is never sent to workflow runners. Instance scope requires an administrator; other scopes require runner-management access to that exact target. Shaula validates access before activation."
    />
  );
}
