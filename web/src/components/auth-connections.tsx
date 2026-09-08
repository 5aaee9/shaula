import { useState } from "react";
import { RefreshCw, Search, ShieldCheck } from "lucide-react";
import { useAuthProfiles } from "@/lib/queries";
import { selectorKey } from "@/lib/auth-policy";
import { selectorLabel, type AuthResource } from "@/lib/types";
import { Empty, ErrorNotice, Loading, StatusBadge } from "./status";
import { Button } from "./ui/button";
import { Input } from "./ui/input";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "./ui/table";

export function AuthConnections({
  selectedKey,
  onSelect,
}: {
  selectedKey: string;
  onSelect: (key: string) => void;
}) {
  const [search, setSearch] = useState("");
  const query = useAuthProfiles();
  const profiles = query.data?.data.profiles || [];
  const items = profiles.filter((profile) =>
    profile.key.toLowerCase().includes(search.trim().toLowerCase()),
  );
  return (
    <section aria-label="Authentication connections">
      <div className="toolbar">
        <div className="search-input">
          <Search />
          <Input
            className="pl-9"
            aria-label="Search authentication connections"
            placeholder="Search connections..."
            value={search}
            onChange={(event) => setSearch(event.target.value)}
          />
        </div>
        <Button
          variant="outline"
          size="icon"
          aria-label="Refresh authentication connections"
          disabled={query.isFetching}
          onClick={() => void query.refetch()}
        >
          <RefreshCw />
        </Button>
      </div>
      {query.error ? (
        <ErrorNotice error={query.error} retry={() => void query.refetch()} />
      ) : query.isPending ? (
        <Loading />
      ) : !items.length ? (
        <Empty
          title={search.trim() ? "No matching connections" : "No authentication connections yet"}
        />
      ) : (
        <div className="table-scroll">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Connection</TableHead>
                <TableHead>Credential type</TableHead>
                <TableHead>Status</TableHead>
                <TableHead>Active targets</TableHead>
                <TableHead>Active revision</TableHead>
                <TableHead>Desired revision</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {items.map((profile) => (
                <TableRow
                  key={profile.key}
                  data-state={selectedKey === profile.key ? "selected" : undefined}
                >
                  <TableCell>
                    <button
                      className="resource-name text-left"
                      onClick={() => onSelect(profile.key)}
                    >
                      <span className="resource-icon">
                        <ShieldCheck />
                      </span>
                      <strong>{profile.key}</strong>
                    </button>
                  </TableCell>
                  <TableCell>
                    {profile.kind === "github_app"
                      ? "GitHub App"
                      : profile.kind === "pat"
                        ? "Personal access token"
                        : "--"}
                  </TableCell>
                  <TableCell>
                    <StatusBadge value={profile.status} />
                  </TableCell>
                  <TableCell>
                    <ActiveTargets profile={profile} />
                  </TableCell>
                  <TableCell>
                    {profile.activeRevision ? `r${profile.activeRevision}` : "--"}
                  </TableCell>
                  <TableCell>r{profile.desiredRevision}</TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </div>
      )}
    </section>
  );
}

function ActiveTargets({ profile }: { profile: AuthResource }) {
  if (!profile.activeRevision) return "--";
  const active = profile.active;
  const labels =
    active?.schema_version === 2
      ? (active.target_policy || []).map((selector) => ({
          key: selectorKey(selector),
          label: selectorLabel(selector),
        }))
      : (active?.target_allowlist || profile.target_allowlist || []).map((target) => ({
          key: target,
          label: target,
        }));
  if (!labels.length) return "--";
  return (
    <div className="flex flex-wrap gap-2">
      {labels.map(({ key, label }) => (
        <span className="label-chip" key={key}>
          {label}
        </span>
      ))}
    </div>
  );
}
