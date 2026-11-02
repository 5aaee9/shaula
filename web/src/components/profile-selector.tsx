import { RefreshCw } from "lucide-react";
import { profileUnavailableReason, type ProfileChoices } from "@/lib/profile-choice";
import { profileOptionLabel, sortedProfiles } from "@/lib/profile-selector-options";
import type { TemplateSummary } from "@/lib/types";
import { Tip } from "./icon-tooltip";
import { ErrorNotice, Field } from "./status";
import { Button } from "./ui/button";
import { NativeSelect, NativeSelectOption } from "./ui/native-select";

function ProfileListNotice({
  noun,
  query,
  profiles,
}: {
  noun: string;
  query: ProfileChoices;
  profiles: TemplateSummary[];
}) {
  if (query.error !== null && query.error !== undefined) return <ErrorNotice error={query.error} />;
  if (query.isPending)
    return (
      <p role="status" className="text-sm">
        Loading {noun}s…
      </p>
    );
  if (!query.isSuccess) return null;
  if (!profiles.length)
    return (
      <p role="status" className="text-sm">
        No {noun}s yet.
      </p>
    );
  if (profiles.every((profile) => profileUnavailableReason(profile)))
    return (
      <p role="status" className="text-sm">
        No profiles are available for new references.
      </p>
    );
  return null;
}

function ProfileRefresh({ label, query }: { label: string; query: ProfileChoices }) {
  const name = `Refresh ${label}s`;
  return (
    <Tip label={name}>
      <Button
        type="button"
        size="icon"
        variant="outline"
        aria-label={name}
        disabled={query.isFetching}
        onClick={() => void query.refetch()}
      >
        <RefreshCw className={query.isFetching ? "animate-spin" : undefined} />
      </Button>
    </Tip>
  );
}

function ProfileChoiceStatus({
  label,
  value,
  query,
  profiles,
  selected,
}: {
  label: string;
  value: string;
  query: ProfileChoices;
  profiles: TemplateSummary[];
  selected?: TemplateSummary;
}) {
  const unavailable = selected && profileUnavailableReason(selected);
  return (
    <>
      <ProfileListNotice noun={label.toLowerCase()} query={query} profiles={profiles} />
      {query.isSuccess && value && (!selected || unavailable) && (
        <p role="status" className="text-sm">
          {value}: {unavailable ?? "This profile is no longer in the list"}. Choose an available
          profile; an unchanged original reference is preserved.
        </p>
      )}
    </>
  );
}

export function ProfileSelector({
  label,
  value,
  query,
  canRead,
  permission,
  disabled = false,
  onChange,
}: {
  label: string;
  value: string;
  query: ProfileChoices;
  canRead: boolean;
  permission: string;
  disabled?: boolean;
  onChange: (key: string) => void;
}) {
  const profiles = sortedProfiles(query.data?.data.profiles);
  const selected = profiles.find((profile) => profile.key === value);
  return (
    <div className="form-stack gap-2 min-w-0">
      <div className="flex min-w-0 items-end gap-2">
        <div className="min-w-0 flex-1">
          <Field label={label}>
            <NativeSelect
              required
              value={value}
              disabled={disabled || !canRead || !query.isSuccess}
              onChange={(event) => onChange(event.target.value)}
            >
              <NativeSelectOption value="">Choose a profile</NativeSelectOption>
              {value && !selected && (
                <NativeSelectOption value={value} disabled>
                  {value} · Current selection (not in the available list)
                </NativeSelectOption>
              )}
              {profiles.map((profile) => (
                <NativeSelectOption
                  key={profile.key}
                  value={profile.key}
                  disabled={!!profileUnavailableReason(profile)}
                >
                  {profileOptionLabel(profile)}
                </NativeSelectOption>
              ))}
            </NativeSelect>
          </Field>
        </div>
        {canRead && <ProfileRefresh label={label} query={query} />}
      </div>
      {canRead ? (
        <ProfileChoiceStatus
          label={label}
          value={value}
          query={query}
          profiles={profiles}
          selected={selected}
        />
      ) : (
        <p role="status" className="text-sm text-muted-foreground">
          {permission} permission is required to choose a {label.toLowerCase()}.
        </p>
      )}
    </div>
  );
}
