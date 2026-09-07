import { useRef, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { LoaderCircle, Trash2 } from "lucide-react";
import { api, MutationAttempt } from "@/lib/api";
import type { Accepted, ChangeRef } from "@/lib/types";
import { Button } from "./ui/button";
import { Input } from "./ui/input";
import { Modal } from "./modal";
import { ErrorNotice, Field } from "./status";
export function RetireDialog({
  name,
  path,
  etag,
  type,
  onClose,
  onAccepted,
}: {
  name: string;
  path: string;
  etag: string | null;
  type: "fleet" | "profile";
  onClose: () => void;
  onAccepted: (change: ChangeRef) => void;
}) {
  const client = useQueryClient();
  const [version] = useState(etag);
  const [confirm, setConfirm] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const attempt = useRef(new MutationAttempt());
  async function retire() {
    setBusy(true);
    setError(null);
    try {
      const { data } = await api<Accepted>(path, {
        method: "DELETE",
        headers: attempt.current.headers("DELETE", path, undefined, version, false),
      });
      onAccepted({ id: data.changeId, resource: name, type });
      void client.invalidateQueries();
      onClose();
    } catch (error) {
      setError(error);
    } finally {
      setBusy(false);
    }
  }
  return (
    <Modal
      open
      title={`Retire ${name}?`}
      description={
        type === "fleet"
          ? "The fleet will stop accepting new capacity and begin decommissioning. Existing work may delay retirement."
          : "The profile will be retired. Referencing fleets and pending work may block completion."
      }
      onOpenChange={(open) => {
        if (!open && !busy) onClose();
      }}
    >
      <div className="form-stack">
        <Field label={`Confirm resource key: ${name}`}>
          <Input
            autoComplete="off"
            value={confirm}
            onChange={(event) => setConfirm(event.target.value)}
          />
        </Field>
        {error !== null && <ErrorNotice error={error} />}
        <div className="dialog-actions">
          <Button variant="outline" disabled={busy} onClick={onClose}>
            Cancel
          </Button>
          <Button
            variant="destructive"
            disabled={confirm !== name || busy}
            onClick={() => void retire()}
          >
            {busy ? <LoaderCircle className="animate-spin" /> : <Trash2 />}Retire
          </Button>
        </div>
      </div>
    </Modal>
  );
}
