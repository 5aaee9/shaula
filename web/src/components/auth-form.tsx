import { NativeSelect, NativeSelectOption } from "@/components/ui/native-select";
import { useRef, useState, type FormEvent } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { KeyRound, LoaderCircle } from "lucide-react";
import { api, MutationAttempt, resourcePath, type Resource } from "@/lib/api";
import type { Accepted, AuthResource, ChangeRef } from "@/lib/types";
import { targetName } from "@/lib/types";
import {
  parseLegacyTargets,
  parseLegacyIdentity,
  rowsFromSelectors,
  selectorFromRow,
  policyDifference,
  type SelectorRow,
} from "@/lib/auth-policy";
import { AuthPolicyPreview } from "./auth-policy-preview";
import { AuthTargetPolicy } from "./auth-target-policy";
import { Modal } from "./modal";
import { Button } from "./ui/button";
import { Input } from "./ui/input";
import { Textarea } from "./ui/textarea";
import { ErrorNotice, Field } from "./status";

/**
 * The form mode has ONE source of truth (R9): a NEW GitHub App profile is
 * always v2 (the App discovers installations — no installation input); an
 * existing profile keeps ITS OWN format unless the operator explicitly
 * chooses the upgrade path, so opening the form never silently expands
 * authorization.
 */
type FormMode = "v2" | "legacy";
function formMode(snapshot?: Resource<AuthResource>, upgrade = false): FormMode {
  if (!snapshot) return "v2";
  if (
    upgrade ||
    snapshot.data.active?.schema_version === 2 ||
    (snapshot.data.schema_version === 2 && !snapshot.data.identity)
  )
    return "v2";
  return "legacy";
}

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
  const isLegacySnapshot =
    !!snapshot && snapshot.data.active?.schema_version !== 2 && !!snapshot.data.identity;
  const [upgradeRequested, setUpgradeRequested] = useState(
    isLegacySnapshot && snapshot?.data.schema_version === 2,
  );
  const mode = formMode(snapshot, upgradeRequested);
  const legacyAllowlist = snapshot?.data.target_allowlist || [];
  // F1: the legacy identity string carries the immutable App id and
  // installation id — parse it so rotation prefills the real members.
  const legacyIdentity = parseLegacyIdentity(snapshot?.data.identity);
  const [key, setKey] = useState(snapshot?.data.key || "");
  const [kind, setKind] = useState(snapshot?.data.kind || "github_app");
  const [appId, setAppId] = useState(
    snapshot?.data.active?.app_id || snapshot?.data.app_id || legacyIdentity.appId,
  );
  const [installation, setInstallation] = useState(legacyIdentity.installationId);
  const [principal, setPrincipal] = useState(
    snapshot?.data.kind === "pat" ? snapshot.data.identity || "" : "",
  );
  const [secret, setSecret] = useState("");
  // Legacy exact targets, prefilled from the active revision so an
  // existing profile can always rotate without retyping its scope. The
  // config URLs are normalized into bare owner[/repository] entries.
  const [targets, setTargets] = useState(
    parseLegacyTargets(legacyAllowlist.join("\n")).map(targetName).join("\n"),
  );
  const existingSelectors =
    snapshot?.data.desired?.target_policy ||
    snapshot?.data.active?.target_policy ||
    parseLegacyTargets(legacyAllowlist.join("\n"));
  const [selectors, setSelectors] = useState<SelectorRow[]>(
    existingSelectors.length
      ? rowsFromSelectors(existingSelectors)
      : // A new policy always starts with one editable row.
        [{ kind: "organization", owner: "", repository: "", account_kind: "user" }],
  );
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const attempt = useRef(new MutationAttempt());
  function selectorError(rows: SelectorRow[]): string | null {
    if (!rows.length) return "At least one target selector is required.";
    for (const row of rows) {
      if (!row.owner.trim()) return "Every selector needs an owner.";
      if (row.kind === "repository" && !row.repository.trim())
        return "A repository selector needs a repository name.";
    }
    return null;
  }
  async function submit(event: FormEvent) {
    event.preventDefault();
    setBusy(true);
    setError(null);
    try {
      if (!impactReady) throw new Error("Load the current Fleet impact before publishing.");
      let body: string;
      if (kind === "github_app" && mode === "v2") {
        if (!/^[1-9][0-9]*$/.test(appId))
          throw new Error("Enter the numeric App ID of the same GitHub App before publishing.");
        const problem = selectorError(selectors);
        if (problem) throw new Error(problem);
        body = JSON.stringify({
          kind,
          schema_version: 2,
          app_id: appId,
          private_key: secret,
          target_policy: selectors.map(selectorFromRow),
        });
      } else if (kind === "github_app") {
        if (!installation)
          throw new Error("An existing profile rotates its exact installation ID.");
        const allowlist = parseLegacyTargets(targets);
        if (!allowlist.length) throw new Error("At least one allowed target is required.");
        body = JSON.stringify({
          kind,
          app_id: appId,
          installation_id: Number(installation),
          private_key: secret,
          target_allowlist: allowlist,
        });
      } else {
        const allowlist = parseLegacyTargets(targets);
        if (!allowlist.length) throw new Error("At least one allowed target is required.");
        body = JSON.stringify({
          kind,
          pat_principal: principal,
          token: secret,
          target_allowlist: allowlist,
        });
      }
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
    } catch (thrown) {
      setError(thrown);
    } finally {
      setBusy(false);
    }
  }
  const nextPolicy = selectors.map(selectorFromRow);
  const previousPolicy =
    snapshot?.data.active?.target_policy || parseLegacyTargets(legacyAllowlist.join("\n"));
  const difference = policyDifference(previousPolicy, nextPolicy);
  const policyChanged = !!difference.added.length || !!difference.removed.length;
  const impact = useQuery({
    queryKey: ["auth-impact", key],
    enabled: !!snapshot && kind === "github_app" && mode === "v2",
    queryFn: async ({ signal }) => {
      const response = await api<{ liveFleets: import("@/lib/types").AuthLiveFleet[] }>(
        resourcePath("github-auth-profiles", key) + "/impact",
        { signal },
      );
      if (!Array.isArray(response.data.liveFleets))
        throw new Error("Fleet impact response is unavailable");
      return response.data.liveFleets;
    },
    refetchInterval: 5_000,
  });
  const impactReady =
    !snapshot || mode !== "v2" || kind !== "github_app" || (impact.isSuccess && !impact.isFetching);
  return (
    <Modal
      open
      title={snapshot ? `Publish ${key}` : "Create authentication profile"}
      onOpenChange={(open) => {
        if (!open && !busy) onClose();
      }}
    >
      <form className="form-stack" onSubmit={submit}>
        <fieldset disabled={busy} className="form-stack">
          <div className="form-grid">
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
                disabled={!!snapshot}
                onChange={(event) => {
                  setKind(event.target.value);
                  setSecret("");
                }}
              >
                <NativeSelectOption value="github_app">GitHub App</NativeSelectOption>
                <NativeSelectOption value="pat">Personal access token</NativeSelectOption>
              </NativeSelect>
            </Field>
          </div>
          {kind === "github_app" ? (
            <>
              <div className="form-grid">
                <Field label="App ID">
                  <Input
                    required
                    value={appId}
                    disabled={snapshot?.data.active?.schema_version === 2}
                    pattern={mode === "v2" ? "[1-9][0-9]*" : undefined}
                    title={
                      mode === "v2"
                        ? "Enter the positive numeric App ID, not a client ID"
                        : undefined
                    }
                    placeholder={isLegacySnapshot ? "numeric App ID" : ""}
                    onChange={(event) => setAppId(event.target.value)}
                  />
                </Field>
                {isLegacySnapshot && mode === "v2" && (
                  <p className="text-sm text-muted-foreground">
                    Enter the same App numeric ID for the upgrade. The existing client ID remains
                    unchanged in the previous revision.
                  </p>
                )}
                {mode === "legacy" && (
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
                )}
              </div>
              <Field label="Private key (PEM)">
                <Textarea
                  required
                  rows={3}
                  className="h-24 field-sizing-fixed"
                  autoComplete="off"
                  spellCheck={false}
                  value={secret}
                  onChange={(event) => setSecret(event.target.value)}
                />
              </Field>
              {mode === "legacy" && (
                <Field label="Allowed targets (immutable exact scope)">
                  <Textarea
                    required
                    placeholder={"acme\nacme/build-tools"}
                    value={targets}
                    onChange={(event) => setTargets(event.target.value)}
                  />
                </Field>
              )}
              {(mode === "v2" || isLegacySnapshot) && (
                <AuthTargetPolicy
                  rows={selectors}
                  onChange={setSelectors}
                  existing={!!snapshot}
                  enabled={mode === "v2"}
                  upgrade={
                    isLegacySnapshot
                      ? {
                          requested: upgradeRequested,
                          onChange: (requested) => {
                            setUpgradeRequested(requested);
                            setAppId(
                              requested
                                ? snapshot?.data.app_id || legacyIdentity.appId
                                : legacyIdentity.appId,
                            );
                          },
                        }
                      : undefined
                  }
                />
              )}
              {mode === "v2" && snapshot && (
                <AuthPolicyPreview
                  previous={previousPolicy}
                  next={nextPolicy}
                  fleets={impact.data}
                  loading={impact.isPending}
                  error={impact.isError}
                />
              )}
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
              <Field label="Allowed targets">
                <Textarea
                  required
                  placeholder={"acme\nacme/build-tools"}
                  value={targets}
                  onChange={(event) => setTargets(event.target.value)}
                />
              </Field>
            </>
          )}
        </fieldset>
        {error !== null && <ErrorNotice error={error} />}
        <div className="dialog-actions">
          <Button variant="outline" type="button" disabled={busy} onClick={onClose}>
            Cancel
          </Button>
          <Button type="submit" disabled={busy || !impactReady}>
            {busy ? <LoaderCircle className="animate-spin" /> : <KeyRound />}
            {snapshot
              ? kind === "github_app" && mode === "v2" && policyChanged
                ? "Publish policy"
                : "Rotate credential"
              : "Create profile"}
          </Button>
        </div>
      </form>
    </Modal>
  );
}
