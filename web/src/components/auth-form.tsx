import { useState } from "react";
import type { Resource } from "@/lib/api";
import type { AuthResource, ChangeRef } from "@/lib/types";
import type { RunnerBackend } from "@/lib/runner-backend";
import { BackendSelector } from "./runner-backend-fields";
import { GitHubAuthForm } from "./github-auth-form";
import { ForgejoAuthForm } from "./forgejo-auth-form";

export type AuthFormMode = "create" | "policy" | "rotate";
export type AuthFormProps = {
  onClose: () => void;
  onAccepted: (change: ChangeRef) => void;
} & (
  | { mode: "create"; resource?: never }
  | { mode: "policy" | "rotate"; resource: Resource<AuthResource> }
);

export function AuthForm(props: AuthFormProps) {
  const [backend, setBackend] = useState<RunnerBackend>(
    props.resource?.data.kind === "forgejo_token" ? "forgejo" : "github",
  );
  const selector = props.mode === "create" && (
    <BackendSelector value={backend} onChange={setBackend} />
  );
  return backend === "forgejo" ? (
    <ForgejoAuthForm {...props} backendSelector={selector} />
  ) : (
    <GitHubAuthForm {...props} backendSelector={selector} />
  );
}
