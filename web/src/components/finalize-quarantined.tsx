import { useRef, useState, type FormEvent } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { CheckCheck, LoaderCircle } from "lucide-react";
import { api } from "@/lib/api";
import type { Generation, Page } from "@/lib/jobs";
import { Modal } from "./modal";
import { Button } from "./ui/button";
import { Input } from "./ui/input";
import { ErrorNotice, Field } from "./status";

/**
 * Bulk operator disposition for a fleet's Quarantined generations
 * (spec 0028). Each generation still receives its own attested finalize
 * request — this only collapses the verification statement and the
 * click-through into one confirmation. Requires fleet.retire.
 */
export function FinalizeQuarantined({
  fleetKey,
  onAccepted,
}: {
  fleetKey: string;
  onAccepted: () => void;
}) {
  const client = useQueryClient();
  const [open, setOpen] = useState(false);
  const [reason, setReason] = useState("");
  const [error, setError] = useState<unknown>(null);
  const [busy, setBusy] = useState(false);
  const [progress, setProgress] = useState<{ done: number; total: number } | null>(null);
  const attempt = useRef(crypto.randomUUID());

  const quarantined = useQuery({
    queryKey: [
      "history",
      `/generations?fleet_key=${encodeURIComponent(fleetKey)}&status=Quarantined`,
    ],
    queryFn: async ({ signal }) =>
      (
        await api<Page<Generation>>(
          `/generations?fleet_key=${encodeURIComponent(fleetKey)}&status=Quarantined&limit=200`,
          { signal },
        )
      ).data,
    enabled: open,
    staleTime: 0,
    retry: false,
  });
  const items = quarantined.data?.items || [];

  async function submit(event: FormEvent) {
    event.preventDefault();
    setError(null);
    const trimmed = reason.trim();
    if (!trimmed) {
      setError(new Error("Describe how you verified the external resources are gone."));
      return;
    }
    if (items.length === 0) {
      setError(new Error("No quarantined runners to finalize."));
      return;
    }
    setBusy(true);
    setProgress({ done: 0, total: items.length });
    let failed = 0;
    try {
      for (const [index, generation] of items.entries()) {
        try {
          await api(`/generations/${encodeURIComponent(generation.id)}/finalize`, {
            method: "POST",
            headers: {
              "content-type": "application/json",
              "idempotency-key": `${attempt.current}-${generation.id}`,
            },
            body: JSON.stringify({ reason: trimmed }),
          });
        } catch {
          failed += 1;
        }
        setProgress({ done: index + 1, total: items.length });
      }
      void client.invalidateQueries({ queryKey: ["history"] });
      void client.invalidateQueries({ queryKey: ["fleet", fleetKey] });
      if (failed === 0) {
        setOpen(false);
        setReason("");
        onAccepted();
      } else {
        setError(
          new Error(
            `${failed} of ${items.length} runners could not be finalized. Retry to finish.`,
          ),
        );
        attempt.current = crypto.randomUUID();
      }
    } finally {
      setBusy(false);
      setProgress(null);
    }
  }

  return (
    <>
      <Button variant="outline" onClick={() => setOpen(true)}>
        <CheckCheck />
        Finalize quarantined
      </Button>
      <Modal
        open={open}
        title={`Finalize quarantined runners — ${fleetKey}`}
        description="Mark every quarantined runner generation Destroyed after you verify the external resources are gone. This unblocks the fleet's decommission."
        onOpenChange={(next) => {
          if (!next && !busy) setOpen(false);
        }}
      >
        <form onSubmit={submit} className="form-stack">
          <fieldset disabled={busy} className="form-stack">
            {open && quarantined.isPending ? (
              <p className="text-sm text-muted-foreground">Loading quarantined runners…</p>
            ) : (
              <p className="text-sm">
                {items.length} quarantined runner{items.length === 1 ? "" : "s"} will be marked
                Destroyed.
              </p>
            )}
            <Field label="Verification">
              <Input
                required
                value={reason}
                onChange={(event) => setReason(event.target.value)}
                placeholder="How you confirmed the VMs / runners are gone"
              />
              <p className="text-sm text-muted-foreground">
                This attests every listed runner's external resources no longer exist. Finalizing
                never contacts the provider — it is a ledger-only close.
              </p>
            </Field>
          </fieldset>
          {progress && (
            <p className="text-sm text-muted-foreground">
              Finalizing {progress.done} / {progress.total}…
            </p>
          )}
          {error !== null && <ErrorNotice error={error} />}
          <div className="dialog-actions">
            <Button type="button" variant="outline" disabled={busy} onClick={() => setOpen(false)}>
              Cancel
            </Button>
            <Button
              type="submit"
              disabled={busy || !reason.trim() || items.length === 0 || quarantined.isPending}
            >
              {busy ? <LoaderCircle className="animate-spin" /> : <CheckCheck />}
              Finalize all
            </Button>
          </div>
        </form>
      </Modal>
    </>
  );
}
