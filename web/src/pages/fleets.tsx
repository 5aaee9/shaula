import { NativeSelect, NativeSelectOption } from "@/components/ui/native-select";
import { useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { ListFilter, Plus, RefreshCw, Search } from "lucide-react";
import { Link, useLocation } from "react-router-dom";
import { useFleets } from "@/lib/queries";
import type { ChangeRef } from "@/lib/types";
import { fleetRowMatches, useFleetRows, type FleetRow } from "@/lib/use-fleet-rows";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Tip } from "@/components/icon-tooltip";
import { Empty, ErrorNotice, Loading } from "@/components/status";
import { SectionCards } from "@/components/section-cards";
import { ChangeNotice } from "@/components/change-notice";
import { FleetsTable } from "@/components/fleets-table";
export function FleetsPage({ scopes }: { scopes: string[] }) {
  const location = useLocation();
  const fleets = useFleets(scopes.includes("fleet.read"));
  const [change] = useState<ChangeRef | null>(
    () => (location.state as { change?: ChangeRef } | null)?.change || null,
  );
  const { rows, total } = useFleetRows(fleets.data?.data.fleets || []);
  if (!scopes.includes("fleet.read"))
    return <ErrorNotice error={new Error("Fleet read permission is required.")} />;
  return (
    <>
      <div className="page-heading">
        <div>
          <h1>Fleets</h1>
          <p>GitHub Actions runner capacity and reconciliation.</p>
        </div>
        {scopes.includes("fleet.write") ? (
          <Button asChild>
            <Link to="/fleets/new" role="button">
              <Plus />
              Create fleet
            </Link>
          </Button>
        ) : (
          <Button disabled>
            <Plus />
            Create fleet
          </Button>
        )}
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
      <FleetInventory fleets={fleets} rows={rows} scopes={scopes} />
    </>
  );
}

function FleetInventory({
  fleets,
  rows,
  scopes,
}: {
  fleets: ReturnType<typeof useFleets>;
  rows: FleetRow[];
  scopes: string[];
}) {
  const client = useQueryClient();
  const [search, setSearch] = useState("");
  const [filter, setFilter] = useState("all");
  const filtered = rows.filter((row) => fleetRowMatches(row, search, filter));
  return (
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
            <Button asChild variant="outline">
              <Link to="/fleets/new" role="button">
                <Plus />
                Create fleet
              </Link>
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
        <FleetsTable rows={filtered} total={rows.length} />
      )}
    </section>
  );
}
