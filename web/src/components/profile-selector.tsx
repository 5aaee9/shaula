import { RefreshCw } from "lucide-react";
import { profileUnavailableReason, type ProfileChoices } from "@/lib/profile-choice";
import { ErrorNotice, Field } from "./status";
import { Button } from "./ui/button";
import { NativeSelect, NativeSelectOption } from "./ui/native-select";

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
  const profiles = [...(query.data?.data.profiles ?? [])].sort((a, b) =>
    a.key < b.key ? -1 : a.key > b.key ? 1 : 0,
  );
  const selected = profiles.find((profile) => profile.key === value);
  const unavailable = selected && profileUnavailableReason(selected);
  return (
    <div className="form-stack gap-2 min-w-0">
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
          {profiles.map((profile) => {
            const reason = profileUnavailableReason(profile);
            return (
              <NativeSelectOption key={profile.key} value={profile.key} disabled={!!reason}>
                {profile.key} ·{" "}
                {profile.activeRevision ? `r${profile.activeRevision}` : "No Active"}
                {` · ${profile.status}${reason ? ` · ${reason}` : ""}`}
              </NativeSelectOption>
            );
          })}
        </NativeSelect>
      </Field>
      {!canRead ? (
        <p role="status" className="text-sm text-muted-foreground">
          {permission} permission is required to choose a {label.toLowerCase()}.
        </p>
      ) : (
        <>
          <Button
            type="button"
            variant="outline"
            aria-label={`Refresh ${label}s`}
            disabled={query.isFetching}
            onClick={() => void query.refetch()}
          >
            <RefreshCw className={query.isFetching ? "animate-spin" : undefined} />
            Refresh profiles
          </Button>
          {query.error !== null && query.error !== undefined ? (
            <ErrorNotice error={query.error} />
          ) : query.isPending ? (
            <p role="status" className="text-sm">
              Loading {label.toLowerCase()}s…
            </p>
          ) : query.isSuccess && !profiles.length ? (
            <p role="status" className="text-sm">
              No {label.toLowerCase()}s yet.
            </p>
          ) : query.isSuccess && profiles.every((profile) => profileUnavailableReason(profile)) ? (
            <p role="status" className="text-sm">
              No profiles are available for new references.
            </p>
          ) : null}
          {query.isSuccess && value && (!selected || unavailable) && (
            <p role="status" className="text-sm">
              {value}: {unavailable ?? "This profile is no longer in the list"}. Choose an available
              profile for a new reference. An unchanged original reference is preserved.
            </p>
          )}
        </>
      )}
    </div>
  );
}
