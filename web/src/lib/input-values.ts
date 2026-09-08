/** Read JSON tokens without converting numbers through JavaScript Number. */
export type InputValue = {
  raw: string;
  kind: "string" | "number" | "boolean" | "null" | "object" | "array";
  text: string;
  children?: [string, InputValue][];
};

export function parseInputValue(source: string): InputValue {
  let offset = 0;
  function space() {
    while (/[ \t\r\n]/.test(source[offset] ?? "") && offset < source.length) offset++;
  }
  function quoted(): string {
    if (source[offset] !== '"') throw new Error("Invalid input value.");
    const start = offset++;
    while (offset < source.length) {
      if (source[offset++] === '"') return JSON.parse(source.slice(start, offset));
      if (source[offset - 1] === "\\") offset++;
    }
    throw new Error("Invalid input value.");
  }
  function read(depth: number): InputValue {
    if (depth > 64) throw new Error("Input value is too deeply nested to display.");
    space();
    const start = offset;
    const first = source[offset];
    let kind: InputValue["kind"];
    let text: string;
    let children: InputValue["children"];
    if (first === '"') {
      kind = "string";
      text = quoted();
    } else if (first === "{" || first === "[") {
      kind = first === "{" ? "object" : "array";
      const end = first === "{" ? "}" : "]";
      children = [];
      offset++;
      space();
      while (source[offset] !== end) {
        const key = kind === "object" ? quoted() : String(children.length);
        if (kind === "object") {
          space();
          if (source[offset++] !== ":") throw new Error("Invalid input value.");
        }
        children.push([key, read(depth + 1)]);
        space();
        if (source[offset] === end) break;
        if (source[offset++] !== ",") throw new Error("Invalid input value.");
        space();
        if (source[offset] === end) throw new Error("Invalid input value.");
      }
      offset++;
      text = `${children.length} ${kind === "object" ? "properties" : "items"}`;
    } else {
      const token = source
        .slice(offset)
        .match(/^(?:true|false|null|-?(?:0|[1-9]\d*)(?:\.\d+)?(?:[eE][+-]?\d+)?)/)?.[0];
      if (!token) throw new Error("Invalid input value.");
      offset += token.length;
      kind = token === "null" ? "null" : /^(true|false)$/.test(token) ? "boolean" : "number";
      text = token;
    }
    return { raw: source.slice(start, offset), kind, text, children };
  }
  const result = read(0);
  space();
  if (offset !== source.length) throw new Error("Invalid input value.");
  return result;
}

function numberKey(raw: string): string {
  const [mantissa, exponent = "0"] = raw.toLowerCase().split("e");
  const negative = mantissa.startsWith("-");
  const unsigned = negative ? mantissa.slice(1) : mantissa;
  const [whole, fraction = ""] = unsigned.split(".");
  const digits = (whole + fraction).replace(/^0+/, "");
  if (!digits) return "0";
  const significant = digits.replace(/0+$/, "");
  const power =
    BigInt(exponent) - BigInt(fraction.length) + BigInt(digits.length - significant.length);
  return `${negative ? "-" : ""}${significant}e${power}`;
}

export function inputValueKey(value: InputValue): string {
  if (value.kind === "number")
    return `${/[.eE]/.test(value.raw) ? "float" : "integer"}:${numberKey(value.raw)}`;
  if (value.kind === "object")
    return JSON.stringify([
      "object",
      [...(value.children ?? [])]
        .sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))
        .map(([key, child]) => [key, inputValueKey(child)]),
    ]);
  if (value.kind === "array")
    return JSON.stringify(["array", value.children?.map(([, child]) => inputValueKey(child))]);
  return JSON.stringify([value.kind, value.text]);
}

export function sameInputValue(left: string, right: string): boolean {
  if (left === right) return true;
  try {
    return inputValueKey(parseInputValue(left)) === inputValueKey(parseInputValue(right));
  } catch {
    // Old values can exceed the projection's display depth. Keep them as opaque data.
    return false;
  }
}

export function inputEntries(raw: string): Map<string, string> {
  // Validate syntax only; never use the resulting numbers to render or write inputs.
  const parsed: unknown = JSON.parse(raw);
  if (!parsed || typeof parsed !== "object" || Array.isArray(parsed))
    throw new Error("Template inputs must be an object.");
  let offset = raw.indexOf("{") + 1;
  const values = new Map<string, string>();
  function space() {
    while (/[ \t\r\n]/.test(raw[offset] ?? "")) offset++;
  }
  function tokenEnd() {
    const start = offset;
    let depth = 0;
    let quoted = false;
    for (; offset < raw.length; offset++) {
      const char = raw[offset];
      if (quoted) {
        if (char === "\\") offset++;
        else if (char === '"') {
          quoted = false;
          if (!depth) return ++offset;
        }
      } else if (char === '"') quoted = true;
      else if (char === "[" || char === "{") depth++;
      else if (char === "]" || char === "}") {
        if (!depth) return offset;
        if (--depth === 0) return ++offset;
      } else if (!depth && (char === "," || /[ \t\r\n]/.test(char))) return offset;
    }
    if (offset === start) throw new Error("Invalid input value.");
    return offset;
  }
  space();
  while (raw[offset] !== "}") {
    const keyStart = offset;
    const key: string = JSON.parse(raw.slice(keyStart, tokenEnd()));
    space();
    offset++;
    space();
    const valueStart = offset;
    const value = raw.slice(valueStart, tokenEnd());
    values.set(key, value);
    space();
    if (raw[offset] === "}") break;
    offset++;
    space();
  }
  return values;
}

export function inputsJson(values: ReadonlyMap<string, string>): string {
  return `{${[...values].map(([key, value]) => `${JSON.stringify(key)}:${value}`).join(",")}}`;
}

export function fleetInputsJson(rawFleet: string): string {
  const spec = inputEntries(rawFleet).get("spec");
  return spec ? (inputEntries(spec).get("template_inputs") ?? "{}") : "{}";
}

export function optionLabel(value: InputValue): string {
  if (value.kind === "boolean") return value.text === "true" ? "Yes (boolean)" : "No (boolean)";
  if (value.kind === "string") return `${value.text === "" ? "Empty string" : value.text} (string)`;
  if (value.kind === "null") return "Null (null)";
  return `${value.text} (${value.kind})`;
}
