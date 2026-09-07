{ lib }:
lib.cleanSourceWith {
  src = ../..;
  filter =
    path: type:
    let
      relative = lib.removePrefix (toString ../.. + "/") (toString path);
      root = builtins.head (lib.splitString "/" relative);
    in
    lib.cleanSourceFilter path type
    && builtins.elem root [
      "Cargo.toml"
      "Cargo.lock"
      "crates"
      "templates"
      "web"
    ]
    && !(builtins.elem (baseNameOf path) [
      "target"
      "node_modules"
      "dist"
      "test-results"
      "playwright-report"
    ])
    && !(lib.hasPrefix ".env" (baseNameOf path));
}
