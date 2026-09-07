import { NativeSelect, NativeSelectOption } from "@/components/ui/native-select";
import { useState, type FormEvent } from "react";
import { Search } from "lucide-react";
import { useSearchParams } from "react-router-dom";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Empty, ErrorNotice, KeyValue, Loading, StatusBadge } from "@/components/status";
import { useChange } from "@/components/change-notice";
export function ChangesPage({ scopes }: { scopes: string[] }) {
  const [params, setParams] = useSearchParams();
  const [id, setId] = useState(params.get("id") || "");
  const [kind, setKind] = useState(params.get("type") === "profile" ? "profile" : "fleet");
  const currentId = params.get("id");
  const type = params.get("type") === "profile" ? "profile" : "fleet";
  const permitted =
    type === "fleet"
      ? scopes.includes("fleet.read")
      : scopes.includes("template.read") || scopes.includes("auth.read");
  const change = useChange(currentId && permitted ? { id: currentId, type, resource: "" } : null);
  function search(event: FormEvent) {
    event.preventDefault();
    setParams({ id: id.trim(), type: kind });
  }
  return (
    <>
      <div className="page-heading">
        <div>
          <h1>Changes</h1>
          <p>Accepted mutations and their convergence state.</p>
        </div>
      </div>
      <form className="toolbar" onSubmit={search}>
        <NativeSelect
          className="w-36"
          aria-label="Change type"
          value={kind}
          onChange={(event) => setKind(event.target.value)}
        >
          <NativeSelectOption value="fleet">Fleet</NativeSelectOption>
          <NativeSelectOption value="profile">Profile</NativeSelectOption>
        </NativeSelect>
        <div className="search-input">
          <Search />
          <Input
            className="pl-9"
            required
            aria-label="Change ID"
            placeholder="Change ID"
            value={id}
            onChange={(event) => setId(event.target.value)}
          />
        </div>
        <Button variant="outline" type="submit">
          Open change
        </Button>
      </form>
      {!currentId ? (
        <Empty title="No change selected" />
      ) : !permitted ? (
        <ErrorNotice error={new Error("Read permission is required for this change.")} />
      ) : change.isPending ? (
        <Loading />
      ) : change.error ? (
        <ErrorNotice error={change.error} retry={() => void change.refetch()} />
      ) : (
        <section className="details-section">
          <h2>Change details</h2>
          <dl className="details-grid">
            <KeyValue label="Change ID">
              <span className="mono">{currentId}</span>
            </KeyValue>
            <KeyValue label="State">
              <StatusBadge value={change.data.state} />
            </KeyValue>
            <KeyValue label="Resource">{change.data.resourceKey}</KeyValue>
            <KeyValue label="Operation">{change.data.kind}</KeyValue>
            <KeyValue label="Revision">r{change.data.revision}</KeyValue>
            <KeyValue label="Reason">{change.data.reason || "--"}</KeyValue>
          </dl>
        </section>
      )}
    </>
  );
}
