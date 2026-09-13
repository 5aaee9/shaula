import { useRef, useState, type FormEvent } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { CheckCheck, LoaderCircle } from "lucide-react";
import { api } from "@/lib/api";
import { Modal } from "./modal";
import { Button } from "./ui/button";
import { Input } from "./ui/input";
import { ErrorNotice, Field } from "./status";

/**
 * Operator disposition for a Quarantined generation (spec 0028): a
 * ledger-only transition to Destroyed once the operator has verified the
 * external runner resources are gone. No remote effect runs; the reason
 * is the attestation. Requires the fleet.retire scope — the same
 * privilege as decommission.
 */
export function FinalizeRunner({
  generationId,
  runnerName,
  onAccepted,
}: {
  generationId: string;
  runnerName: string;
  onAccepted: () => void;
}) {
  const client = useQueryClient();
  const [open, setOpen] = useState(false);
  const [reason, setReason] = useState("");
  const [error, setError] = useState<unknown>(null);
  const [busy, setBusy] = useState(false);
  const attempt = useRef(crypto.randomUUID());

  async function submit(event: FormEvent) {
    event.preventDefault();
    setError(null);
    const trimmed = reason.trim();
    if (!trimmed) {
      setError(new Error("Describe how you verified the external resources are gone."));
      return;
    }
    try {
      setBusy(true);
      await api(`/generations/${encodeURIComponent(generationId)}/finalize`, {
        method: "POST",
        headers: {
          "content-type": "application/json",
          "idempotency-key": attempt.current,
        },
        body: JSON.stringify({ reason: trimmed }),
      });
      void client.invalidateQueries({ queryKey: ["history"] });
      setOpen(false);
      onAccepted();
    } catch (error) {
      setError(error);
      // A rejected attempt must not replay under the same idempotency key.
      attempt.current = crypto.randomUUID();
    } finally {
      setBusy(false);
    }
  }

  return (
    <>
      <Button onClick={() => setOpen(true)}>
        <CheckCheck />
        Finalize runner
      </Button>
      <Modal
        open={open}
        title={`Finalize ${runnerName}`}
        description="Record that this runner's external resources are gone and close the generation."
        onOpenChange={(next) => {
          if (!next && !busy) setOpen(false);
        }}
      >
        <form onSubmit={submit} className="form-stack">
          <fieldset disabled={busy} className="form-stack">
            <Field label="Verification">
              <Input
                required
                value={reason}
                onChange={(event) => setReason(event.target.value)}
                placeholder="How you confirmed the VM / runner is gone"
              />
              <p className="text-sm text-muted-foreground">
                Finalizing marks the generation Destroyed without contacting the provider. Use it
                only after you have verified the external resources no longer exist.
              </p>
            </Field>
          </fieldset>
          {error !== null && <ErrorNotice error={error} />}
          <div className="dialog-actions">
            <Button type="button" variant="outline" disabled={busy} onClick={() => setOpen(false)}>
              Cancel
            </Button>
            <Button type="submit" disabled={busy || !reason.trim()}>
              {busy ? <LoaderCircle className="animate-spin" /> : <CheckCheck />}
              Finalize
            </Button>
          </div>
        </form>
      </Modal>
    </>
  );
}
