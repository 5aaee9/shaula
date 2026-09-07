import {
  Table,
  TableHeader,
  TableBody,
  TableRow,
  TableHead,
  TableCell,
} from "@/components/ui/table";
import { NativeSelect, NativeSelectOption } from "@/components/ui/native-select";
import { useState } from "react";
import { useQueries, useQueryClient } from "@tanstack/react-query";
import { ArrowRight, Boxes, CircleDot, ListFilter, Plus, RefreshCw, Search } from "lucide-react";
import { Link } from "react-router-dom";
import { api, resourcePath } from "@/lib/api";
import { useFleets } from "@/lib/queries";
import type { ChangeRef, FleetResource, FleetStatus } from "@/lib/types";
import { targetName } from "@/lib/types";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Tip } from "@/components/icon-tooltip";
import { Empty, ErrorNotice, Loading, StatusBadge } from "@/components/status";
import { ChangeNotice } from "@/components/change-notice";
import { FleetForm } from "@/components/fleet-form";
import { SectionCards } from "@/components/section-cards";
export function FleetsPage({ scopes }: { scopes: string[] }) {
  const client = useQueryClient();
  const fleets = useFleets(scopes.includes("fleet.read"));
  const [search, setSearch] = useState("");
  const [filter, setFilter] = useState("all");
  const [create, setCreate] = useState(false);
  const [change, setChange] = useState<ChangeRef | null>(null);
  const summaries = fleets.data?.data.fleets || [];
  const resources = useQueries({
    queries: summaries.map(({ key }) => ({
      queryKey: ["fleet", key],
      queryFn: ({ signal }: { signal: AbortSignal }) =>
        api<FleetResource>(resourcePath("fleets", key), { signal }),
      refetchInterval: 10000,
    })),
  });
  const statuses = useQueries({
    queries: summaries.map(({ key }) => ({
      queryKey: ["fleet-status", key],
      queryFn: ({ signal }: { signal: AbortSignal }) =>
        api<FleetStatus>(`${resourcePath("fleets", key)}/status`, { signal }),
      refetchInterval: 5000,
    })),
  });
  const rows = summaries.map((summary, i) => ({
    ...summary,
    resource: resources[i].data?.data,
    status: statuses[i].data?.data,
    error: resources[i].error || statuses[i].error,
  }));
  const filtered = rows.filter(
    (row) =>
      `${row.key} ${row.resource ? targetName(row.resource.spec.github.target) : ""}`
        .toLowerCase()
        .includes(search.toLowerCase()) &&
      (filter === "all" ||
        (filter === "attention"
          ? /block|error|fail|reject/i.test(row.status?.phase || "") || !!row.error
          : row.status?.phase === filter)),
  );
  const complete = statuses.every((query) => query.isSuccess);
  const total = (key: keyof FleetStatus["capacity"]) =>
    complete ? rows.reduce((sum, row) => sum + (row.status?.capacity[key] || 0), 0) : null;
  if (!scopes.includes("fleet.read"))
    return <ErrorNotice error={new Error("Fleet read permission is required.")} />;
  return (
    <>
      <div className="page-heading">
        <div>
          <h1>Fleets</h1>
          <p>GitHub Actions runner capacity and reconciliation.</p>
        </div>
        <Button onClick={() => setCreate(true)} disabled={!scopes.includes("fleet.write")}>
          <Plus />
          Create fleet
        </Button>
      </div>
      <ChangeNotice change={change} />
      <SectionCards
        metrics={[
          {
            label: "Total fleets",
            value: fleets.data ? rows.length : null,
            detail: "Registered fleets",
          },
          {
            label: "Effective capacity",
            value: total("effective"),
            detail: "Available runner capacity",
          },
          { label: "Assigned demand", value: total("assignedDemand"), detail: "Assigned jobs" },
          { label: "Target capacity", value: total("target"), detail: "Desired runner capacity" },
        ]}
      />
      <section className="resource-section" aria-label="Fleet inventory">
        <div className="section-heading">
          <h2>
            All fleets <span className="count">{rows.length}</span>
          </h2>
          <span className="text-xs text-muted-foreground">
            {fleets.dataUpdatedAt
              ? `Updated ${new Date(fleets.dataUpdatedAt).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}`
              : "Awaiting response"}
          </span>
        </div>
        <div className="toolbar">
          <div className="search-input">
            <Search />
            <Input
              className="pl-9"
              aria-label="Search fleets"
              placeholder="Search fleets or GitHub targets..."
              value={search}
              onChange={(event) => setSearch(event.target.value)}
            />
          </div>
          <div className="filter-select">
            <ListFilter className="size-4" />
            <NativeSelect
              aria-label="Filter fleet status"
              value={filter}
              onChange={(event) => setFilter(event.target.value)}
            >
              <NativeSelectOption value="all">All statuses</NativeSelectOption>
              <NativeSelectOption value="Ready">Ready</NativeSelectOption>
              <NativeSelectOption value="attention">Needs attention</NativeSelectOption>
              <NativeSelectOption value="Pending">Pending</NativeSelectOption>
              <NativeSelectOption value="Retiring">Retiring</NativeSelectOption>
            </NativeSelect>
          </div>
          <Tip label="Refresh fleets">
            <Button
              variant="outline"
              size="icon"
              aria-label="Refresh fleets"
              onClick={() => {
                void client.invalidateQueries({ queryKey: ["fleets"] });
                void client.invalidateQueries({ queryKey: ["fleet"] });
                void client.invalidateQueries({ queryKey: ["fleet-status"] });
              }}
            >
              <RefreshCw className={fleets.isFetching ? "animate-spin" : ""} />
            </Button>
          </Tip>
        </div>
        {fleets.error ? (
          <ErrorNotice error={fleets.error} retry={() => void fleets.refetch()} />
        ) : fleets.isPending ? (
          <Loading />
        ) : !rows.length ? (
          <Empty title="No fleets yet">
            {scopes.includes("fleet.write") && (
              <Button variant="outline" onClick={() => setCreate(true)}>
                <Plus />
                Create fleet
              </Button>
            )}
          </Empty>
        ) : !filtered.length ? (
          <Empty title="No matching fleets">
            <Button
              variant="ghost"
              onClick={() => {
                setSearch("");
                setFilter("all");
              }}
            >
              Clear filters
            </Button>
          </Empty>
        ) : (
          <div className="table-scroll">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Fleet / GitHub target</TableHead>
                  <TableHead>Status</TableHead>
                  <TableHead>Capacity</TableHead>
                  <TableHead>Template</TableHead>
                  <TableHead>Revision</TableHead>
                  <TableHead>
                    <span className="sr-only">Open</span>
                  </TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {filtered.map((row) => (
                  <TableRow key={row.key}>
                    <TableCell>
                      <Link to={`/fleets/${encodeURIComponent(row.key)}`} className="resource-name">
                        <span className="resource-icon">
                          <Boxes />
                        </span>
                        <span>
                          <strong>{row.key}</strong>
                          <small>
                            {row.resource
                              ? targetName(row.resource.spec.github.target)
                              : "Loading target..."}
                          </small>
                        </span>
                      </Link>
                    </TableCell>
                    <TableCell>
                      <StatusBadge
                        value={row.error ? "Unavailable" : row.status?.phase || "Loading"}
                      />
                      {row.status?.lastError && (
                        <p className="table-detail text-red-700">{row.status.lastError}</p>
                      )}
                    </TableCell>
                    <TableCell>
                      {row.status ? (
                        <div className="capacity-cell">
                          <span>
                            <strong>{row.status.capacity.effective}</strong>
                            <span className="text-muted-foreground">
                              {" "}
                              / {row.status.capacity.target}
                            </span>
                          </span>
                          <div className="capacity-track">
                            <span
                              style={{
                                width: `${Math.min(100, (row.status.capacity.effective / Math.max(1, row.status.capacity.target)) * 100)}%`,
                              }}
                            />
                          </div>
                        </div>
                      ) : (
                        "--"
                      )}
                    </TableCell>
                    <TableCell>
                      <div className="table-detail">
                        {row.resource?.resolved.template?.key || "--"}
                      </div>
                      {row.resource?.resolved.template && (
                        <small className="text-muted-foreground">
                          Revision {row.resource.resolved.template.revision}
                        </small>
                      )}
                    </TableCell>
                    <TableCell>
                      <span className="mono text-xs">r{row.revision}</span>
                    </TableCell>
                    <TableCell>
                      <Tip label={`Open ${row.key}`}>
                        <Button asChild size="icon" variant="ghost">
                          <Link
                            aria-label={`Open ${row.key}`}
                            to={`/fleets/${encodeURIComponent(row.key)}`}
                          >
                            <ArrowRight />
                          </Link>
                        </Button>
                      </Tip>
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
            <div className="table-footer">
              <span>
                {filtered.length} of {rows.length} fleets
              </span>
              <span className="flex items-center gap-1.5">
                <CircleDot className="size-3 text-emerald-600" />
                Live status
              </span>
            </div>
          </div>
        )}
      </section>
      {create && (
        <FleetForm scopes={scopes} onClose={() => setCreate(false)} onAccepted={setChange} />
      )}
    </>
  );
}
