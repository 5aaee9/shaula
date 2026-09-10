import { useEffect, useRef, useState } from "react";
import { api, resourcePath, type Resource } from "./api";
import { checkInputContract, type InputContract } from "./input-contract";
import { fleetInputsJson, inputEntries, inputsJson } from "./input-values";
import { profileUnavailableReason, sameTemplateReference } from "./profile-choice";
import type { FleetResource, FleetSpec, TemplateResource } from "./types";

type Selection = { contract: InputContract; reference: FleetSpec["template_profile_ref"] };
type ContractRead = {
  key: string;
  revision: number;
  digest?: string;
  incarnation?: string;
  reference: FleetSpec["template_profile_ref"];
  original: boolean;
};
const identity = (contract: InputContract) =>
  JSON.stringify([
    contract.profileKey,
    contract.incarnation,
    contract.revision,
    contract.artifactDigest,
  ]);

export function useFleetInputs(resource: Resource<FleetResource> | undefined, canRead: boolean) {
  const originalRef = resource?.data.spec.template_profile_ref;
  const originalKey = typeof originalRef === "string" ? originalRef : (originalRef?.key ?? "");
  const [template, setTemplate] = useState(originalKey);
  const [revision, setRevision] = useState("");
  const [values, setValues] = useState(() =>
    inputEntries(
      resource?.rawJson
        ? fleetInputsJson(resource.rawJson)
        : JSON.stringify(resource?.data.spec.template_inputs ?? {}),
    ),
  );
  const [selection, setSelection] = useState<Selection>();
  const originalSelection = useRef<Selection | undefined>(undefined);
  const [pending, setPending] = useState<Selection>();
  const [error, setError] = useState<unknown>(
    !canRead ? new Error("Template read permission is required to edit template inputs.") : null,
  );
  const [loading, setLoading] = useState(false);
  const [failedSwitch, setFailedSwitch] = useState(false);
  const [presetChosen, setPresetChosen] = useState(!!resource);
  const request = useRef<AbortController | null>(null);
  const sequence = useRef(0);
  const lastRead = useRef<ContractRead | undefined>(undefined);
  const currentKey = selection?.contract.profileKey ?? originalKey;
  const locked = !!resource && !selection;

  async function load({
    original: useOriginal = false,
    latest: useLatest = false,
    retry = false,
    key = template.trim(),
  }: { original?: boolean; latest?: boolean; retry?: boolean; key?: string } = {}) {
    request.current?.abort();
    const controller = new AbortController();
    request.current = controller;
    const generation = ++sequence.current;
    setLoading(true);
    setError(null);
    setPending(undefined);
    const previousRead = retry ? lastRead.current : undefined;
    if (!retry) lastRead.current = undefined;
    try {
      if (!canRead)
        throw new Error("Template read permission is required to edit template inputs.");
      let selectedKey = key;
      let selectedRevision: number;
      let digest: string | undefined;
      let reference: FleetSpec["template_profile_ref"];
      let incarnation: string | undefined;
      if (previousRead) {
        selectedKey = previousRead.key;
        selectedRevision = previousRead.revision;
        digest = previousRead.digest;
        incarnation = previousRead.incarnation;
        reference = previousRead.reference;
        useOriginal = previousRead.original;
      } else if (useOriginal && resource) {
        const pin = resource.data.resolved.template;
        if (!pin) throw new Error("This fleet has no resolved template revision to read.");
        selectedKey = pin.key;
        selectedRevision = pin.revision;
        digest = pin.artifactDigest;
        reference = resource.data.spec.template_profile_ref;
      } else {
        if (!selectedKey) throw new Error("Choose a template profile first.");
        const profile = await api<TemplateResource>(
          resourcePath("template-profiles", selectedKey),
          { signal: controller.signal },
        );
        const unavailable = profileUnavailableReason(profile.data);
        if (unavailable) throw new Error(`This template cannot be selected: ${unavailable}.`);
        const active = profile.data.activeRevision!;
        if (!useLatest && revision && Number(revision) !== active)
          throw new Error(
            `Revision ${revision} is not current Active. The current Active revision is ${active}.`,
          );
        selectedRevision = active;
        incarnation = profile.data.incarnation;
        // Follow-latest is the default (spec 0023): an explicit revision
        // input is the pin opt-in; an empty field submits the bare key
        // reference and the daemon cascades future Active revisions.
        reference = revision ? { key: selectedKey, revision: active } : selectedKey;
      }
      if (generation !== sequence.current) return;
      lastRead.current = {
        key: selectedKey,
        revision: selectedRevision,
        digest,
        incarnation,
        reference,
        original: useOriginal,
      };
      const response = await api<InputContract>(
        `${resourcePath("template-profiles", selectedKey)}/revisions/${selectedRevision}/input-contract`,
        { signal: controller.signal },
      );
      const contract = checkInputContract(response.data, selectedKey, selectedRevision, digest);
      if (incarnation && contract.incarnation !== incarnation)
        throw new Error(
          "The template was replaced while loading its inputs. Load it again explicitly.",
        );
      if (generation !== sequence.current) return;
      const next = { contract, reference };
      setFailedSwitch(false);
      if (useOriginal) {
        originalSelection.current = next;
        setSelection(next);
        setTemplate(selectedKey);
      } else if (selection && identity(selection.contract) === identity(contract)) {
        // Re-reading identical material cannot reset drafts or rewrite a legacy bare reference.
        setTemplate(selectedKey);
        setRevision("");
      } else if (values.size) {
        setPending(next);
      } else {
        setSelection(next);
        setTemplate(selectedKey);
        setRevision("");
        setValues(new Map());
        setPresetChosen(false);
      }
    } catch (failure) {
      if (generation !== sequence.current || controller.signal.aborted) return;
      setError(failure);
      setFailedSwitch(!useOriginal);
    } finally {
      if (generation === sequence.current) setLoading(false);
    }
  }

  useEffect(() => {
    if (resource) void load({ original: true });
    return () => {
      sequence.current++;
      request.current?.abort();
    };
    // The captured Fleet snapshot is the owner of this dialog's initial read.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  function cancelSwitch() {
    sequence.current++;
    request.current?.abort();
    setLoading(false);
    setPending(undefined);
    setFailedSwitch(false);
    setError(null);
    setTemplate(currentKey);
    setRevision("");
    lastRead.current = undefined;
  }
  function confirmSwitch() {
    if (!pending) return;
    setSelection(pending);
    setTemplate(pending.contract.profileKey);
    setRevision("");
    setValues(new Map());
    setPresetChosen(false);
    setPending(undefined);
    setError(null);
  }
  function restoreOriginal() {
    if (!resource) return;
    cancelSwitch();
    setSelection(originalSelection.current);
    setTemplate(originalKey);
    setValues(
      inputEntries(
        resource.rawJson
          ? fleetInputsJson(resource.rawJson)
          : JSON.stringify(resource.data.spec.template_inputs),
      ),
    );
    setPresetChosen(true);
  }
  function changeTemplate(key: string) {
    sequence.current++;
    request.current?.abort();
    setLoading(false);
    setPending(undefined);
    setTemplate(key);
    setRevision("");
    setError(null);
    setFailedSwitch(false);
    lastRead.current = undefined;
    if (key) void load({ key, latest: true });
  }
  function changeRevision(value: string) {
    sequence.current++;
    request.current?.abort();
    setLoading(false);
    setPending(undefined);
    setRevision(value);
    lastRead.current = undefined;
  }
  function updateValue(key: string, value?: string) {
    setValues((current) => {
      const next = new Map(current);
      if (value === undefined) next.delete(key);
      else next.set(key, value);
      return next;
    });
  }
  const dirtyReference = template.trim() !== currentKey || !!revision;
  return {
    template,
    revision,
    setRevision: changeRevision,
    changeTemplate,
    values,
    updateValue,
    replaceValues: (value?: string) => {
      setValues(inputEntries(value ?? "{}"));
      setPresetChosen(value !== undefined);
    },
    presetChosen,
    selection,
    pending,
    error,
    loading,
    locked,
    canRead,
    canSubmit:
      !loading && !pending && !dirtyReference && !failedSwitch && (!!selection || !!resource),
    needsCancel: !!pending || failedSwitch || dirtyReference,
    load,
    retry: () => load({ original: locked, retry: true }),
    cancelSwitch,
    confirmSwitch,
    canRestoreOriginal:
      !!resource && !sameTemplateReference(selection?.reference ?? originalRef ?? "", originalRef),
    restoreOriginal,
    reference: selection?.reference ?? originalRef ?? "",
    json: inputsJson(values),
  };
}

export type FleetInputs = ReturnType<typeof useFleetInputs>;
