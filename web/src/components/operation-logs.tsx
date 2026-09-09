import { useState } from "react";
import { Button } from "@/components/ui/button";
import { NativeSelect, NativeSelectOption } from "@/components/ui/native-select";
import { Empty, ErrorNotice, Loading, StatusBadge } from "@/components/status";
import {
  historyTime,
  operationName,
  useHistory,
  type Invocation,
  type LogPage,
  type InvocationsPage,
} from "@/lib/jobs";

export function OperationLogs({
  generationId,
  scopes,
}: {
  generationId: string;
  scopes: string[];
}) {
  const [attemptCursor, setAttemptCursor] = useState("");
  const invocations = useHistory<InvocationsPage>(
    `/generations/${encodeURIComponent(generationId)}/invocations?${new URLSearchParams({ limit: "50", ...(attemptCursor ? { cursor: attemptCursor } : {}) })}`,
    scopes.includes("fleet.read"),
  );
  const [selectedId, select] = useState("");
  const items = invocations.data?.items || [];
  const selected = items.find((item) => item.id === selectedId) || items[0];
  return (
    <section className="details-section">
      <h2>Apply and destroy logs</h2>
      {invocations.error && (
        <ErrorNotice error={invocations.error} retry={() => void invocations.refetch()} />
      )}
      {invocations.isPending ? (
        <Loading />
      ) : invocations.error && !invocations.data ? null : items.length === 0 ? (
        <Empty title="No recorded invocations">
          This generation has no retained execution history.
        </Empty>
      ) : (
        <>
          <div className="mb-4 flex flex-wrap gap-6">
            {["Create", "Destroy"].map((operation) => {
              const latest =
                operation === "Create"
                  ? invocations.data?.latest_create
                  : invocations.data?.latest_destroy;
              return (
                <div key={operation}>
                  <span className="mr-2 text-sm text-muted-foreground">
                    {operationName(operation)}
                  </span>
                  <StatusBadge value={latest?.execution_outcome || "not_recorded"} />
                </div>
              );
            })}
          </div>
          <NativeSelect
            aria-label="Execution attempt"
            value={selected?.id || ""}
            onChange={(e) => select(e.target.value)}
          >
            {items.map((item) => (
              <NativeSelectOption key={item.id} value={item.id}>
                {operationName(item.operation)} #{item.ordinal} ·{" "}
                {item.commands.map((command) => command.phase).join(" / ") || "Preparing"} ·{" "}
                {historyTime(item.started_at)}
              </NativeSelectOption>
            ))}
          </NativeSelect>
          <div className="mt-3 flex gap-2">
            <Button
              variant="outline"
              disabled={!attemptCursor}
              onClick={() => setAttemptCursor("")}
            >
              Latest attempts
            </Button>
            <Button
              variant="outline"
              disabled={!invocations.data?.next_cursor}
              onClick={() => setAttemptCursor(invocations.data?.next_cursor || "")}
            >
              Older attempts
            </Button>
          </div>
          {selected && (
            <div className="mt-4 space-y-4">
              <div className="flex flex-wrap items-center gap-3 text-sm">
                <span>Execution</span>
                <StatusBadge value={selected.execution_outcome} />
                <span>Log capture</span>
                <StatusBadge value={selected.capture_status} />
                <span className="text-muted-foreground">
                  {selected.retained_bytes.toLocaleString()} bytes retained ·{" "}
                  {selected.lost_bytes.toLocaleString()} bytes omitted
                </span>
              </div>
              {!scopes.includes("logs.read") ? (
                <p className="text-sm text-muted-foreground">
                  logs.read permission is required to view log text.
                </p>
              ) : (
                <LogViewer key={selected.id} invocation={selected} />
              )}
            </div>
          )}
        </>
      )}
    </section>
  );
}

function LogViewer({ invocation }: { invocation: Invocation }) {
  const [cursor, setCursor] = useState("");
  const [phase, setPhase] = useState("");
  const [stream, setStream] = useState("");
  const [copyState, setCopyState] = useState("");
  const params = new URLSearchParams({ limit_bytes: "262144" });
  if (cursor) params.set("cursor", cursor);
  if (phase) params.set("phase", phase);
  if (stream) params.set("stream", stream);
  const log = useHistory<LogPage>(
    `/invocations/${encodeURIComponent(invocation.id)}/logs?${params}`,
    true,
  );
  const text = log.data?.entries.map((entry) => entry.text).join("") || "";
  async function copy() {
    try {
      await navigator.clipboard.writeText(text);
      setCopyState("Page copied");
    } catch {
      setCopyState("Copy failed");
    }
  }
  return (
    <div>
      <div className="mb-3 flex flex-wrap gap-2">
        <NativeSelect
          aria-label="Log phase"
          value={phase}
          onChange={(e) => {
            setPhase(e.target.value);
            setCursor("");
          }}
        >
          <NativeSelectOption value="">All phases</NativeSelectOption>
          {["init", "plan", "apply"].map((value) => (
            <NativeSelectOption key={value} value={value}>
              {value}
            </NativeSelectOption>
          ))}
        </NativeSelect>
        <NativeSelect
          aria-label="Log stream"
          value={stream}
          onChange={(e) => {
            setStream(e.target.value);
            setCursor("");
          }}
        >
          <NativeSelectOption value="">Both streams</NativeSelectOption>
          <NativeSelectOption value="stdout">stdout</NativeSelectOption>
          <NativeSelectOption value="stderr">stderr</NativeSelectOption>
        </NativeSelect>
        <Button variant="outline" disabled={!text} onClick={() => void copy()}>
          Copy page
        </Button>
        <span className="self-center text-sm" role="status">
          {copyState}
        </span>
      </div>
      {log.error && <ErrorNotice error={log.error} retry={() => void log.refetch()} />}
      {log.isPending ? (
        <Loading />
      ) : (
        log.data && (
          <>
            {(log.data.has_gap || log.data.capture_status !== "complete") && (
              <p className="mb-2 text-sm text-muted-foreground">
                Log availability: {log.data.capture_status}.{" "}
                {log.data.has_gap ? "Some output was omitted." : ""}
              </p>
            )}
            <pre
              aria-label="Operation log"
              tabIndex={0}
              className="max-h-[32rem] min-h-40 overflow-auto rounded-md border bg-muted/40 p-4 font-mono text-xs leading-5 whitespace-pre-wrap break-all"
            >
              {text || "No retained text for this page."}
            </pre>
            <div className="mt-3 flex gap-2">
              <Button variant="outline" disabled={!cursor} onClick={() => setCursor("")}>
                First log page
              </Button>
              <Button
                variant="outline"
                disabled={!log.data.next_cursor}
                onClick={() => setCursor(log.data?.next_cursor || "")}
              >
                Next log page
              </Button>
            </div>
          </>
        )
      )}
    </div>
  );
}
