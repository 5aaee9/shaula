import { parseInputValue } from "./input-values";

export interface ApprovedInputOption {
  valueJson: string;
}
export interface InputContractField {
  key: string;
  label: string;
  description: string;
  required: boolean;
  options: ApprovedInputOption[];
}
interface ContractIdentity {
  version: 1;
  profileKey: string;
  incarnation: string;
  revision: number;
  artifactDigest: string;
}
export type InputContract = ContractIdentity &
  (
    | { mode: "fields"; fields: InputContractField[] }
    | { mode: "presets"; presets: ApprovedInputOption[] }
  );

/** Check the response envelope and tokens; admission remains server-owned. */
export function checkInputContract(
  value: InputContract,
  key: string,
  revision: number,
  digest?: string,
): InputContract {
  if (
    value.version !== 1 ||
    value.profileKey !== key ||
    value.revision !== revision ||
    !value.incarnation ||
    !value.artifactDigest ||
    (digest && value.artifactDigest !== digest)
  )
    throw new Error(
      "The input contract does not match the selected template revision. Reload it explicitly.",
    );
  const options =
    value.mode === "fields"
      ? value.fields?.flatMap((field) => field.options)
      : value.mode === "presets"
        ? value.presets
        : undefined;
  if (!Array.isArray(options)) throw new Error("This input contract version is not supported.");
  for (const option of options) parseInputValue(option.valueJson);
  return value;
}
