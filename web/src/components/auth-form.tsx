import { NativeSelect, NativeSelectOption } from "@/components/ui/native-select";
import { useRef, useState, type FormEvent } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { KeyRound, LoaderCircle } from "lucide-react";
import { api, MutationAttempt, resourcePath, type Resource } from "@/lib/api";
import type { Accepted, AuthResource, ChangeRef, GitHubTarget } from "@/lib/types";
import { Modal } from "./modal";
import { Button } from "./ui/button";
import { Input } from "./ui/input";
import { Textarea } from "./ui/textarea";
import { ErrorNotice, Field } from "./status";
export function AuthForm({
  resource,
  onClose,
  onAccepted,
}: {
  resource?: Resource<AuthResource>;
  onClose: () => void;
  onAccepted: (change: ChangeRef) => void;
}) {
  const [snapshot] = useState(resource);
  const client = useQueryClient();
  const [key, setKey] = useState(snapshot?.data.key || "");
  const [kind, setKind] = useState(snapshot?.data.kind || "github_app");
  const [appId, setAppId] = useState("");
  const [installation, setInstallation] = useState("");
  const [principal, setPrincipal] = useState("");
  const [secret, setSecret] = useState("");
  const [targets, setTargets] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const attempt = useRef(new MutationAttempt());
  async function submit(event: FormEvent) {
    event.preventDefault();
    setBusy(true);
    setError(null);
    try {
      const allowlist: GitHubTarget[] = targets
        .split(/[\n,]+/)
        .map((value) => value.trim())
        .filter(Boolean)
        .map((value) => {
          const parts = value.split("/");
          if (parts.length > 2 || parts.some((part) => !part))
            throw new Error("Targets must be an organization or owner/repository.");
          return parts.length === 2
            ? { kind: "repository", owner: parts[0], repository: parts[1] }
            : { kind: "organization", owner: parts[0] };
        });
      if (!allowlist.length) throw new Error("At least one allowed target is required.");
      const body = JSON.stringify({
        kind,
        target_allowlist: allowlist,
        ...(kind === "github_app"
          ? { app_id: appId, installation_id: Number(installation), private_key: secret }
          : { pat_principal: principal, token: secret }),
      });
      const path = resourcePath("github-auth-profiles", key);
      const { data } = await api<Accepted>(path, {
        method: "PUT",
        body,
        headers: attempt.current.headers("PUT", path, body, snapshot?.etag || null, !snapshot),
      });
      setSecret("");
      onAccepted({ id: data.changeId, resource: key, type: "profile" });
      void client.invalidateQueries();
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
      title={snapshot ? `Rotate ${key}` : "Create authentication profile"}
      onOpenChange={(open) => {
        if (!open && !busy) onClose();
      }}
    >
      <form className="form-stack" onSubmit={submit}>
        <fieldset disabled={busy} className="form-stack">
          <Field label="Profile key">
            <Input
              required
              pattern="[a-z0-9][a-z0-9-]*"
              maxLength={63}
              disabled={!!snapshot}
              value={key}
              onChange={(event) => setKey(event.target.value)}
            />
          </Field>
          <Field label="Credential type">
            <NativeSelect
              value={kind}
              onChange={(event) => {
                setKind(event.target.value);
                setSecret("");
              }}
            >
              <NativeSelectOption value="github_app">GitHub App</NativeSelectOption>
              <NativeSelectOption value="pat">Personal access token</NativeSelectOption>
            </NativeSelect>
          </Field>
          {kind === "github_app" ? (
            <>
              <div className="form-grid">
                <Field label="App ID">
                  <Input
                    required
                    value={appId}
                    onChange={(event) => setAppId(event.target.value)}
                  />
                </Field>
                <Field label="Installation ID">
                  <Input
                    required
                    type="number"
                    min={1}
                    step={1}
                    value={installation}
                    onChange={(event) => setInstallation(event.target.value)}
                  />
                </Field>
              </div>
              <Field label="Private key (PEM)">
                <Textarea
                  required
                  autoComplete="off"
                  spellCheck={false}
                  value={secret}
                  onChange={(event) => setSecret(event.target.value)}
                />
              </Field>
            </>
          ) : (
            <>
              <Field label="PAT principal">
                <Input
                  required
                  value={principal}
                  onChange={(event) => setPrincipal(event.target.value)}
                />
              </Field>
              <Field label="Personal access token">
                <Input
                  type="password"
                  autoComplete="new-password"
                  required
                  value={secret}
                  onChange={(event) => setSecret(event.target.value)}
                />
              </Field>
            </>
          )}
          <Field label="Allowed targets">
            <Textarea
              required
              placeholder={"acme\nacme/build-tools"}
              value={targets}
              onChange={(event) => setTargets(event.target.value)}
            />
          </Field>
        </fieldset>
        {error !== null && <ErrorNotice error={error} />}
        <div className="dialog-actions">
          <Button variant="outline" type="button" disabled={busy} onClick={onClose}>
            Cancel
          </Button>
          <Button type="submit" disabled={busy}>
            {busy ? <LoaderCircle className="animate-spin" /> : <KeyRound />}
            {snapshot ? "Rotate credential" : "Create profile"}
          </Button>
        </div>
      </form>
    </Modal>
  );
}
