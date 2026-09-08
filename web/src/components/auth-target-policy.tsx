import { useState } from "react";
import { selectorFromRow, type SelectorRow } from "@/lib/auth-policy";
import { selectorLabel } from "@/lib/types";
import { Plus, X } from "lucide-react";
import { AdvancedSettings } from "./advanced-settings";
import { Field } from "./status";
import { Button } from "./ui/button";
import { Input } from "./ui/input";
import { NativeSelect, NativeSelectOption } from "./ui/native-select";

const SELECTOR_KIND_LABELS: Record<SelectorRow["kind"], string> = {
  organization: "Organization runners",
  repository: "Single repository",
  account_repositories: "Account repositories",
};

export function AuthTargetPolicy({
  rows,
  onChange,
  existing,
  enabled,
  upgrade,
}: {
  rows: SelectorRow[];
  onChange: (rows: SelectorRow[]) => void;
  existing: boolean;
  enabled: boolean;
  upgrade?: { requested: boolean; onChange: (requested: boolean) => void };
}) {
  const [advanced, setAdvanced] = useState(false);
  const additional = rows.slice(1);
  const summary =
    !existing && additional.length
      ? `${additional.length} additional target${additional.length === 1 ? "" : "s"}: ${additional.map((row) => (row.owner.trim() ? selectorLabel(selectorFromRow(row)) : "Unnamed target")).join(", ")}`
      : upgrade
        ? upgrade.requested
          ? "Multi-account upgrade selected"
          : "Multi-account upgrade"
        : existing
          ? "GitHub access details"
          : "Additional targets and GitHub access details";
  function targetFields(row: SelectorRow, index: number, removable: boolean) {
    return (
      <AuthTargetFields
        key={index}
        row={row}
        onChange={(patch) =>
          onChange(rows.map((current, i) => (i === index ? { ...current, ...patch } : current)))
        }
        onRemove={removable ? () => onChange(rows.filter((_, i) => i !== index)) : undefined}
      />
    );
  }
  const addTarget = (
    <Button
      variant="outline"
      type="button"
      onClick={() =>
        onChange([
          ...rows,
          { kind: "organization", owner: "", repository: "", account_kind: "user" },
        ])
      }
    >
      <Plus />
      Add selector
    </Button>
  );
  return (
    <>
      {enabled && (
        <Field label={existing ? "Target policy" : "Target"}>
          <div className="form-stack">
            {(existing ? rows : rows.slice(0, 1)).map((row, index) =>
              targetFields(row, index, existing && rows.length > 1),
            )}
            {existing && addTarget}
          </div>
        </Field>
      )}
      <AdvancedSettings open={advanced} onOpenChange={setAdvanced} summary={summary}>
        {upgrade && (
          <label className="flex items-center gap-2 text-sm">
            <input
              type="checkbox"
              checked={upgrade.requested}
              onChange={(event) => upgrade.onChange(event.target.checked)}
            />
            Upgrade to multi-account policy
          </label>
        )}
        {enabled && (
          <>
            {!existing && (
              <>
                {additional.length > 0 && (
                  <Field label="Additional targets">
                    <div className="form-stack">
                      {additional.map((row, index) => targetFields(row, index + 1, true))}
                    </div>
                  </Field>
                )}
                {addTarget}
              </>
            )}
            <p className="text-sm text-muted-foreground">
              Account repositories include future repositories only when the GitHub installation
              uses “All repositories”. Organization targets allow organization runners; repository
              access follows the runner group.
            </p>
          </>
        )}
      </AdvancedSettings>
    </>
  );
}

function AuthTargetFields({
  row,
  onChange,
  onRemove,
}: {
  row: SelectorRow;
  onChange: (patch: Partial<SelectorRow>) => void;
  onRemove?: () => void;
}) {
  return (
    <div className="space-y-2">
      <div className="flex items-start gap-2">
        <div className="grid min-w-0 flex-1 gap-2 sm:grid-cols-2 [&_[data-slot=native-select-wrapper]]:w-full">
          <NativeSelect
            aria-label="Selector type"
            value={row.kind}
            onChange={(event) => onChange({ kind: event.target.value as SelectorRow["kind"] })}
          >
            {(Object.entries(SELECTOR_KIND_LABELS) as [SelectorRow["kind"], string][]).map(
              ([value, label]) => (
                <NativeSelectOption key={value} value={value}>
                  {label}
                </NativeSelectOption>
              ),
            )}
          </NativeSelect>
          <Input
            required
            pattern={".*\\S.*"}
            title="Enter a GitHub owner"
            placeholder="owner"
            aria-label="Owner"
            value={row.owner}
            onChange={(event) => onChange({ owner: event.target.value })}
          />
          {row.kind === "account_repositories" && (
            <NativeSelect
              aria-label="Account type"
              value={row.account_kind}
              onChange={(event) =>
                onChange({ account_kind: event.target.value as "user" | "organization" })
              }
            >
              <NativeSelectOption value="user">User</NativeSelectOption>
              <NativeSelectOption value="organization">Organization</NativeSelectOption>
            </NativeSelect>
          )}
          {row.kind === "repository" && (
            <Input
              required
              pattern={".*\\S.*"}
              title="Enter a repository name"
              placeholder="repository"
              aria-label="Repository"
              value={row.repository}
              onChange={(event) => onChange({ repository: event.target.value })}
            />
          )}
        </div>
        {onRemove && (
          <Button
            variant="outline"
            size="icon"
            type="button"
            aria-label="Remove selector"
            onClick={onRemove}
          >
            <X />
          </Button>
        )}
      </div>
      {row.kind === "account_repositories" && (
        <p className="text-sm text-muted-foreground">
          Only owned repositories accessible to this App, including future repositories when its
          installation allows them.
        </p>
      )}
    </div>
  );
}
