import { RotateCw, SlidersHorizontal, Trash2 } from "lucide-react";
import { useNavigate } from "react-router-dom";
import type { AuthResource, TargetSelector } from "@/lib/types";
import { selectorLabel } from "@/lib/types";
import { authKindLabel, forgejoTargetName, gitlabTargetName } from "@/lib/runner-backend";
import { selectorKey } from "@/lib/auth-policy";
import { Button } from "@/components/ui/button";
import { KeyValue, StatusBadge } from "@/components/status";
import { AuthReauth } from "@/components/auth-reauth";

export function AuthDetailsActions({
  profileKey,
  profile,
  canManage,
  canRetire,
  onRetire,
}: {
  profileKey: string;
  profile: AuthResource;
  canManage: boolean;
  canRetire: boolean;
  onRetire: () => void;
}) {
  const navigate = useNavigate();
  return (
    <div className="flex min-w-0 flex-wrap gap-2">
      {canManage && (
        <>
          {profile.kind === "github_app" && (
            <>
              <AuthReauth profile={profile} />
              <Button
                size="sm"
                variant="outline"
                onClick={() => navigate(`/auth/${encodeURIComponent(profileKey)}/targets/edit`)}
              >
                <SlidersHorizontal />
                Edit target policy
              </Button>
            </>
          )}
          <Button
            size="sm"
            variant="outline"
            onClick={() => navigate(`/auth/${encodeURIComponent(profileKey)}/rotate`)}
          >
            <RotateCw />
            Rotate credential
          </Button>
        </>
      )}
      <Button
        variant="outline"
        size="icon"
        aria-label="Retire authentication profile"
        disabled={!canRetire}
        onClick={onRetire}
      >
        <Trash2 />
      </Button>
    </div>
  );
}

export function AuthProfileFacts({ profile }: { profile: AuthResource }) {
  return (
    <dl className="details-grid">
      <KeyValue label="Status">
        <StatusBadge value={profile.status} />
      </KeyValue>
      <KeyValue label="Credential type">{authKindLabel(profile.kind)}</KeyValue>
      {profile.schema_version === 2 && (
        <KeyValue label="App ID">{profile.active?.app_id || profile.app_id || "--"}</KeyValue>
      )}
      <KeyValue label="Credential">{profile.credential_present ? "Configured" : "Absent"}</KeyValue>
      <KeyValue label="Active revision">
        {profile.activeRevision ? `r${profile.activeRevision}` : "--"}
      </KeyValue>
      <KeyValue label="Desired revision">r{profile.desiredRevision}</KeyValue>
      {profile.active?.schema_version === 2 && (
        <KeyValue label="Active target policy">
          <TargetPolicyChips policy={profile.active.target_policy} />
        </KeyValue>
      )}
      {profile.desired && (
        <KeyValue label={`Candidate r${profile.desired.revision}: ${profile.desired.state}`}>
          {profile.desired.reason && <p className="text-red-600">{profile.desired.reason}</p>}
          {profile.desired.forgejo ? (
            forgejoTargetName(profile.desired.forgejo.target)
          ) : profile.desired.gitlab ? (
            gitlabTargetName(profile.desired.gitlab.target)
          ) : (
            <TargetPolicyChips policy={profile.desired.target_policy} />
          )}
        </KeyValue>
      )}
    </dl>
  );
}

function TargetPolicyChips({ policy }: { policy?: TargetSelector[] }) {
  if (!policy?.length) return "--";
  return (
    <div className="flex flex-wrap gap-2">
      {policy.map((selector) => (
        <span className="label-chip" key={selectorKey(selector)}>
          {selectorLabel(selector)}
        </span>
      ))}
    </div>
  );
}
