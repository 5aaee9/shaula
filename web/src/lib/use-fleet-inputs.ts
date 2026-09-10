import { useEffect, useRef, useState } from "react";
import { api, resourcePath, type Resource } from "./api";
import { checkInputContract, type InputContract } from "./input-contract";
import { fleetInputsJson, inputEntries, inputsJson } from "./input-values";
import { profileUnavailableReason } from "./profile-choice";
import type { FleetResource, TemplateResource } from "./types";

// Follow-only model (spec 0023): the submitted reference is always the
// bare profile key, so a selection is fully described by its contract.
type Selection = { contract: InputContract };
type ContractRead = {
  key: string;
  revision: number;
  digest?: string;
  incarnation?: string;
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
  const originalKey = resource?.data.spec.template_profile_ref ?? "";
  const [template, setTemplate] = useState(originalKey);
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
    retry = false,
    key = template.trim(),
  }: { original?: boolean; retry?: boolean; key?: string } = {}) {
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
      let incarnation: string | undefined;
      if (previousRead) {
        selectedKey = previousRead.key;
        selectedRevision = previousRead.revision;
        digest = previousRead.digest;
        incarnation = previousRead.incarnation;
        useOriginal = previousRead.original;
      } else if (useOriginal && resource) {
        const pin = resource.data.resolved.template;
        if (!pin) throw new Error("This fleet has no resolved template revision to read.");
        selectedKey = pin.key;
        selectedRevision = pin.revision;
        digest = pin.artifactDigest;
      } else {
        if (!selectedKey) throw new Error("Choose a template profile first.");
        const profile = await api<TemplateResource>(
          resourcePath("template-profiles", selectedKey),
          { signal: controller.signal },
        );
        const unavailable = profileUnavailableReason(profile.data);
        if (unavailable) throw new Error(`This template cannot be selected: ${unavailable}.`);
        selectedRevision = profile.data.activeRevision!;
        incarnation = profile.data.incarnation;
      }
      if (generation !== sequence.current) return;
      lastRead.current = {
        key: selectedKey,
        revision: selectedRevision,
        digest,
        incarnation,
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
      const next = { contract };
      setFailedSwitch(false);
      if (useOriginal) {
        originalSelection.current = next;
        setSelection(next);
        setTemplate(selectedKey);
      } else if (selection && identity(selection.contract) === identity(contract)) {
        // Re-reading identical material cannot reset drafts.
        setTemplate(selectedKey);
      } else if (values.size) {
        setPending(next);
      } else {
        setSelection(next);
        setTemplate(selectedKey);
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
    lastRead.current = undefined;
  }
  function confirmSwitch() {
    if (!pending) return;
    setSelection(pending);
    setTemplate(pending.contract.profileKey);
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
    setError(null);
    setFailedSwitch(false);
    lastRead.current = undefined;
    if (key) void load({ key });
  }
  function updateValue(key: string, value?: string) {
    setValues((current) => {
      const next = new Map(current);
      if (value === undefined) next.delete(key);
      else next.set(key, value);
      return next;
    });
  }
  const dirtyReference = template.trim() !== currentKey;
  return {
    template,
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
      !!resource && (selection?.contract.profileKey ?? originalKey) !== originalKey,
    restoreOriginal,
    reference: currentKey,
    json: inputsJson(values),
  };
}

export type FleetInputs = ReturnType<typeof useFleetInputs>;
