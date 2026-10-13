import { useState } from "react";
import type { Resource } from "@/lib/api";
import type { AuthResource, ChangeRef } from "@/lib/types";
import type { RunnerBackend } from "@/lib/runner-backend";
import { BackendSelector } from "./runner-backend-fields";
import { GitHubAuthForm } from "./github-auth-form";
import { ForgejoAuthForm } from "./forgejo-auth-form";
import { GitlabAuthForm } from "./gitlab-auth-form";

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
  if (backend === "forgejo") return <ForgejoAuthForm {...props} backendSelector={selector} />;
  if (backend === "gitlab") return <GitlabAuthForm {...props} backendSelector={selector} />;
  return <GitHubAuthForm {...props} backendSelector={selector} />;
}
